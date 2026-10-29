// A received message's source (right-click › View Source in its text): its HTML as it arrived,
// and the whole original message (headers, every part) as fetched from Gmail. Read-only, with
// Copy and Save As.
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useRpc } from "../../api/query";
import { rpc } from "../../api/rpc";
import { composeApi } from "../../api/mailCompose";
import { errorBox } from "../../ui";

export type SourceTab = "html" | "raw";

/** Long HTML on one line made readable: a line break after each block and line break tag. */
export function prettyHtml(html: string): string {
  return html.replace(/(<\/(p|div|h[1-6]|li|ul|ol|table|tbody|thead|tr|blockquote|pre|section|header|footer)>|<br\s*\/?>)(?!\n)/gi, "$1\n");
}

export function MessageSourceView({ id, tab }: { id: number; tab: SourceTab }) {
  const q = useRpc<{ html: string | null; raw: string }>(["mailSource", id], "mail.getSource", { id }, { staleTime: 10 * 60_000, retry: false });
  if (q.isError) return <div className="cm-mail-rp-empty">{(q.error as Error).message}</div>;
  if (!q.data) return <div className="cm-mail-rp-empty">Loading the source…</div>;
  const text = tab === "html" ? (q.data.html ? prettyHtml(q.data.html) : "This message has no HTML: it is plain text (see Original Message).") : q.data.raw;
  return <textarea className="cm-source-view" readOnly spellCheck={false} value={text} />;
}

/** Copy / Save As for the source shown. */
export function sourceActions(id: number, tab: SourceTab, subject: string) {
  const get = async () => {
    const s = await rpc<{ html: string | null; raw: string }>("mail.getSource", { id });
    return tab === "html" ? (s.html ?? "") : s.raw;
  };
  return {
    copy: () =>
      get()
        .then((t) => navigator.clipboard.writeText(t))
        .catch(errorBox),
    save: async () => {
      const name = `${(subject.trim() || "message").replace(/[\\/:*?"<>|]/g, "_")}.${tab === "html" ? "html" : "eml"}`;
      const path = await saveDialog({
        title: "Save Source",
        defaultPath: name,
        filters: tab === "html" ? [{ name: "Web Page", extensions: ["html"] }] : [{ name: "E-mail Message", extensions: ["eml"] }],
      }).catch(() => null);
      if (!path) return;
      try {
        await composeApi.saveAs(path, await get());
      } catch (e) {
        errorBox(e);
      }
    },
  };
}
