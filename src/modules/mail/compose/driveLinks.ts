// Attachments too large for Gmail, sent as Google Drive links (src-tauri/src/mail/compose/drive.rs):
// which files go as links, the question asked first, and the uploads' progress.
import { useEffect, useRef, useState } from "react";
import { messageBox } from "../../../ui";
import { useCmEvent } from "../../../api/events";
import { driveApi, formatSize, type DriveUpload } from "../../../api/mailCompose";

/** Gmail's limit per message (attachments together). */
export const LINK_ABOVE = 25 * 1024 * 1024;

/**
 * Which of the files being attached (indexes into `sizes`) go as Drive links: every file over the
 * limit, then — when the rest still doesn't fit beside what is attached already (`attached`
 * bytes) — the largest of them, one by one, until it does.
 */
export function planLinks(attached: number, sizes: number[]): Set<number> {
  const links = new Set<number>();
  sizes.forEach((s, i) => s > LINK_ABOVE && links.add(i));
  const rest = sizes.map((s, i) => ({ s, i })).filter((x) => !links.has(x.i));
  let total = attached + rest.reduce((t, x) => t + x.s, 0);
  for (const x of [...rest].sort((a, b) => b.s - a.s)) {
    if (total <= LINK_ABOVE) break;
    links.add(x.i);
    total -= x.s;
  }
  return links;
}

/** "Send as Link" or not (Cancel leaves those files out). Skipped once "Don't ask me again" is ticked. */
export async function askSendAsLink(files: { name: string; size: number }[], folder: string): Promise<boolean> {
  const one = files.length === 1;
  const list = files.map((f) => `   ${f.name} (${formatSize(f.size)})`).join("\n");
  const r = await messageBox({
    title: "Send as Google Drive Link",
    text:
      `${one ? "This file is" : "These files are"} too large to attach (Gmail takes up to 25 MB per message):\n\n${list}\n\n` +
      `ClassicMail can put ${one ? "it" : "them"} in your Google Drive (the "${folder}" folder) and send a link instead. ` +
      `When you press Send, the people you send to are given access.`,
    icon: "info",
    buttons: ["Send as Link", "Cancel"],
    defaultButton: "Send as Link",
    dontAsk: { key: "compose.driveLink", answer: "Send as Link" },
  });
  return r === "Send as Link";
}

/** The uploads behind a compose's Drive attachments, kept current from `drive.upload` events. */
export function useDriveUploads(ids: number[]): Record<number, DriveUpload> {
  const [byId, setById] = useState<Record<number, DriveUpload>>({});
  const known = useRef(new Set<number>());
  const key = ids.join(",");
  useEffect(() => {
    const missing = ids.filter((id) => !known.current.has(id));
    if (!missing.length) return;
    missing.forEach((id) => known.current.add(id));
    driveApi
      .uploads(missing)
      .then((list) => setById((m) => ({ ...m, ...Object.fromEntries(list.map((u) => [u.id, u])) })))
      .catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  useCmEvent("drive.upload", (e) => {
    const u = e as unknown as DriveUpload;
    if (known.current.has(u.id)) setById((m) => ({ ...m, [u.id]: u }));
  });
  return byId;
}

/** What the attachment line shows after the size: "45%", "link", "upload failed". */
export function uploadNote(u: DriveUpload | undefined, uploadId: number | null | undefined): string {
  if (!uploadId) return "link";
  if (!u) return "uploading…";
  if (u.status === "failed") return "upload failed";
  if (u.status === "done") return "link";
  return `${u.size ? Math.floor((u.sent / u.size) * 100) : 0}%`;
}
