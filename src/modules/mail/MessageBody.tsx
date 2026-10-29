// Message body in a sandboxed iframe (no scripts; strict CSP). Links open in the browser through
// the back end; keys pressed inside the body are forwarded to the window (Delete, Ctrl+R, ...).
import { useEffect, useMemo, useRef } from "react";
import { rpc } from "../../api/rpc";
import { showMenu } from "../../ui";
import { keyOf } from "../../ui/keys";
import { compose } from "./actions";
import { useBodyZoom, wireFrame } from "../../ui/zoom";
import { markPainted } from "../../api/windows";

const ATTACH = "http://cmattach.localhost cmattach:";

function srcdoc(html: string, plain: boolean, allowRemote: boolean) {
  const img = allowRemote ? `${ATTACH} data: https: http:` : `${ATTACH} data:`;
  const csp = `default-src 'none'; img-src ${img}; style-src 'unsafe-inline'${allowRemote ? " https: http:" : ""}; font-src data:${allowRemote ? " https: http:" : ""}; media-src 'none'; frame-src 'none'; form-action 'none'`;
  return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${csp}">
<style>
html{background:#fff}
body{margin:0;padding:10px 12px;font-family:Calibri,'Segoe UI',Tahoma,Arial,sans-serif;font-size:13.5px;color:#000;word-wrap:break-word;overflow-wrap:anywhere}
.cm-plain p{margin:0 0 1em} .cm-plain{font-family:Calibri,'Segoe UI',Arial,sans-serif;font-size:13.5px}
a{color:#0645ad} blockquote{margin:0 0 0 .8ex;border-left:2px solid #b5c4df;padding-left:1ex;color:#333}
pre{white-space:pre-wrap}
/* "anywhere" lets a table shrink every column to one letter (min-content): in cells a long word
   still breaks, but a table too wide for the pane scrolls instead (a spreadsheet, a financial table) */
td,th{overflow-wrap:break-word}
/* Outlook's Hebrew lists are an LTR <ol> of RTL items: the numbers hang off the right edge */
ol:has(> li[dir="rtl" i]),ul:has(> li[dir="rtl" i]){padding-right:40px}
</style></head><body><div dir="auto" class="${plain ? "cm-plain-wrap" : "cm-html-wrap"}">${html}</div></body></html>`;
}

export function MessageBody({
  html,
  plain,
  allowRemote,
  accountId,
  onDocument,
  onViewSource,
}: {
  html: string;
  plain: boolean;
  allowRemote: boolean;
  accountId?: number;
  /** right-click › View Source (the reader shows the message's source in tabs) */
  onViewSource?: () => void;
  /** called with the body document after every (re)load (Translate bar) */
  onDocument?: (d: Document) => void;
}) {
  const ref = useRef<HTMLIFrameElement>(null);
  const onDocRef = useRef(onDocument);
  onDocRef.current = onDocument;
  const onSourceRef = useRef(onViewSource);
  onSourceRef.current = onViewSource;
  const doc = useMemo(() => srcdoc(html, plain, allowRemote), [html, plain, allowRemote]);
  const zoom = useBodyZoom();
  const zoomRef = useRef(zoom);
  zoomRef.current = zoom;
  const fitRef = useRef<(() => void) | null>(null);
  // message zoom: the whole body scales (text, pictures, tables)
  useEffect(() => {
    const d = ref.current?.contentDocument;
    if (!d?.documentElement) return;
    d.documentElement.style.zoom = String(zoom / 100);
    fitRef.current?.();
  }, [zoom]);

  useEffect(() => {
    const f = ref.current;
    if (!f) return;
    const wire = () => {
      const d = f.contentDocument;
      if (!d) return;
      // a message window that was waiting hidden (spare) shows itself once the body is in
      markPainted();
      d.addEventListener("click", (e) => {
        const a = (e.target as Element)?.closest?.("a");
        if (!a) return;
        e.preventDefault();
        const href = a.getAttribute("href") ?? "";
        if (/^mailto:/i.test(href)) {
          const to = decodeURIComponent(href.replace(/^mailto:/i, "").split("?")[0]);
          compose("new", undefined, { to, ...(accountId ? { accountId: String(accountId) } : {}) });
        } else if (/^https?:/i.test(href)) {
          rpc("app.openUrl", { url: href }).catch(() => {});
        }
      });
      d.addEventListener("contextmenu", (e) => {
        e.preventDefault();
        const r = f.getBoundingClientRect();
        const sel = d.getSelection()?.toString() ?? "";
        const a = (e.target as Element)?.closest?.("a");
        const href = a?.getAttribute("href") ?? "";
        showMenu(r.left + e.clientX, r.top + e.clientY, [
          { label: "Copy", icon: "copy", disabled: !sel, onClick: () => navigator.clipboard.writeText(sel).catch(() => {}) },
          { label: "Select All", onClick: () => d.execCommand("selectAll") },
          ...(href
            ? [
                { type: "separator" as const },
                { label: "Open Hyperlink", onClick: () => /^https?:/i.test(href) && rpc("app.openUrl", { url: href }) },
                { label: "Copy Hyperlink", icon: "link", onClick: () => navigator.clipboard.writeText(href.replace(/^mailto:/i, "")).catch(() => {}) },
              ]
            : []),
          ...(onSourceRef.current ? [{ type: "separator" as const }, { label: "View Source", onClick: () => onSourceRef.current?.() }] : []),
        ]);
      });
      d.addEventListener("keydown", (e) => {
        // let the window's shortcuts work while the body has focus (but keep Ctrl+C / Ctrl+A)
        if (e.ctrlKey && (keyOf(e) === "c" || keyOf(e) === "a")) return;
        if (e.ctrlKey || e.altKey || e.key === "Delete" || e.key === "Insert" || e.key === "Backspace" || e.key === "Escape" || (e.key.startsWith("F") && e.key.length > 1)) {
          window.dispatchEvent(new KeyboardEvent("keydown", { key: e.key, code: e.code, ctrlKey: e.ctrlKey, shiftKey: e.shiftKey, altKey: e.altKey, bubbles: true }));
          if (e.ctrlKey) e.preventDefault();
        }
      });
      d.addEventListener("mousedown", () => window.dispatchEvent(new CustomEvent("cm-mail-body-click")));
      // no horizontal scroll bar for a few pixels of overhang (RTL rounding, hanging list markers);
      // genuinely wide mail (tables, newsletters) still scrolls
      d.documentElement.style.zoom = String(zoomRef.current / 100);
      wireFrame(d);
      const fitWidth = () => {
        const e = d.documentElement;
        e.style.overflowX = "";
        const over = e.scrollWidth - e.clientWidth;
        if (over > 0 && over <= 4) e.style.overflowX = "hidden";
      };
      fitWidth();
      fitRef.current = fitWidth;
      d.defaultView?.addEventListener("resize", fitWidth);
      onDocRef.current?.(d);
    };
    f.addEventListener("load", wire);
    // already loaded before this effect ran: at least hand the document over
    const ready = f.contentDocument;
    if (ready?.querySelector(".cm-html-wrap, .cm-plain-wrap")) onDocRef.current?.(ready);
    return () => f.removeEventListener("load", wire);
  }, [accountId]);

  return <iframe ref={ref} className="cm-mail-body" title="Message body" sandbox="allow-same-origin" srcDoc={doc} data-zoom />;
}
