// The From / To / Cc / Bcc names of a message you're reading, as in Outlook: click a name to select
// it (Ctrl+click / Shift+click / arrows for more, Ctrl+A for all of them), Ctrl+C or Edit › Copy
// puts "Name <email>; …" on the clipboard. Right-click for Copy / Copy E-mail Address / Send.
import { useEffect, useRef, useState } from "react";
import { contextMenu, type MenuItem } from "../../ui";
import { openWindow } from "../../api/windows";
import { NAMES_MIME, addrClipText, addrsClipText, clickSelect, copyText, span } from "./addrClip";

interface A {
  name: string;
  email: string;
}

export function AddressTokens({ list, className, showEmail }: { list: A[]; className?: string; showEmail?: boolean }) {
  const [sel, setSel] = useState<number[]>([]);
  const anchor = useRef<number | null>(null);
  const boxRef = useRef<HTMLSpanElement>(null);
  const selRef = useRef(sel);
  selRef.current = sel;
  const listRef = useRef(list);
  listRef.current = list;
  const key = list.map((a) => a.email || a.name).join(";");
  useEffect(() => {
    setSel([]);
    anchor.current = null;
  }, [key]);

  const chosen = () => selRef.current.map((i) => listRef.current[i]).filter(Boolean);
  const copy = (items: A[]) => {
    if (items.length) copyText(addrsClipText(items));
  };

  // Edit › Copy and the ribbon's Copy run document.execCommand("copy"): answer it while we have focus
  useEffect(() => {
    const onCopy = (e: ClipboardEvent) => {
      if (!boxRef.current?.contains(document.activeElement) || !selRef.current.length) return;
      e.clipboardData?.setData("text/plain", addrsClipText(chosen()));
      e.preventDefault();
    };
    document.addEventListener("copy", onCopy);
    return () => document.removeEventListener("copy", onCopy);
  }, []);

  if (!list.length) return null;
  const selectAll = () => {
    setSel(span(0, list.length - 1));
    anchor.current = 0;
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    const k = e.key.toLowerCase();
    if (e.ctrlKey && !e.altKey && (k === "a" || k === "c" || k === "insert")) {
      e.preventDefault();
      e.stopPropagation();
      if (k === "a") selectAll();
      else copy(chosen());
      return;
    }
    if (e.key === "ArrowLeft" || e.key === "ArrowRight" || e.key === "Home" || e.key === "End") {
      e.preventDefault();
      e.stopPropagation();
      const cur = sel.length ? (e.key === "ArrowLeft" ? sel[0] : sel[sel.length - 1]) : -1;
      const last = list.length - 1;
      const to = e.key === "Home" ? 0 : e.key === "End" ? last : e.key === "ArrowLeft" ? Math.max(0, cur - 1) : Math.min(last, cur + 1);
      if (e.shiftKey && anchor.current !== null) setSel(span(anchor.current, to));
      else {
        setSel([to]);
        anchor.current = to;
      }
      return;
    }
    if (e.key === "Escape" && sel.length) {
      e.stopPropagation();
      setSel([]);
    }
  };

  const menu = (i: number): MenuItem[] => {
    const items = sel.includes(i) ? chosen() : [list[i]];
    const a = list[i];
    const emails = items.map((x) => x.email).filter(Boolean);
    return [
      { label: a.email || a.name, type: "header" as const },
      { label: "Copy", icon: "copy", shortcut: "Ctrl+C", onClick: () => copy(items) },
      { label: "Copy E-mail Address", disabled: !emails.length, onClick: () => copyText(emails.join("; ")) },
      { label: "Select All", shortcut: "Ctrl+A", onClick: selectAll },
      { type: "separator" },
      {
        label: "Send E-mail Message",
        icon: "new-mail",
        disabled: !emails.length,
        onClick: () =>
          void openWindow(
            "compose",
            {
              mode: "new",
              to: items
                .filter((x) => x.email)
                .map(addrClipText)
                .join("; "),
            },
            { title: "Untitled - Message" },
          ),
      },
    ];
  };

  return (
    <span
      ref={boxRef}
      className={`cm-addr-tokens${className ? ` ${className}` : ""}`}
      tabIndex={-1}
      onKeyDown={onKeyDown}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setSel([]);
      }}
    >
      {list.map((a, i) => (
        <span key={i}>
          <span
            className={`cm-addr-token${sel.includes(i) ? " selected" : ""}`}
            title={a.email || undefined}
            dir="auto"
            onMouseDown={(e) => {
              if (e.button === 2 && sel.includes(i)) return; // right-click keeps a multi-selection
              window.getSelection()?.removeAllRanges(); // a name, not the header's text
              const r = clickSelect(sel, anchor.current, i, { ctrl: e.ctrlKey, shift: e.shiftKey });
              setSel(r.sel);
              anchor.current = r.anchor;
            }}
            onContextMenu={contextMenu(() => menu(i))}
            // drag a name (or the selected ones) into To / Cc / Bcc of a message being written
            draggable
            onDragStart={(e) => {
              const items = sel.includes(i) ? chosen() : [list[i]];
              e.dataTransfer.setData(NAMES_MIME, JSON.stringify(items.map(({ name, email }) => ({ name, email }))));
              e.dataTransfer.setData("text/plain", addrsClipText(items));
              e.dataTransfer.effectAllowed = "copy";
            }}
          >
            {a.name ? (
              <>
                <bdi>{a.name}</bdi>
                {showEmail && a.email && <span className="cm-mail-rp-addr"> [{a.email}]</span>}
              </>
            ) : (
              a.email
            )}
          </span>
          {i < list.length - 1 && "; "}
        </span>
      ))}
    </span>
  );
}
