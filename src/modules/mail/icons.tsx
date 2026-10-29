// Mail-only icons (file types for attachments, list glyphs). Our own drawings.
import { Icon } from "../../ui";

function doc(color: string, label: string) {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden>
      <path d="M3.5 1.5h6l3 3v10h-9z" fill="#fff" stroke="#8a8a8a" />
      <path d="M9.5 1.5v3h3" fill="#eee" stroke="#8a8a8a" />
      <rect x="1.5" y="7" width="9" height="6" rx="1" fill={color} />
      <text x="6" y="11.6" fontSize="4.6" fontFamily="Arial" fontWeight="bold" fill="#fff" textAnchor="middle">
        {label}
      </text>
    </svg>
  );
}

export function FileIcon({ name, mime }: { name: string; mime: string }) {
  const ext = (name.split(".").pop() ?? "").toLowerCase();
  if (ext === "pdf" || mime === "application/pdf") return doc("#c4302b", "PDF");
  if (["xls", "xlsx", "csv"].includes(ext)) return doc("#1d7044", "XLS");
  if (["doc", "docx", "rtf"].includes(ext)) return doc("#2b579a", "DOC");
  if (["ppt", "pptx"].includes(ext)) return doc("#c55a11", "PPT");
  if (["zip", "rar", "7z"].includes(ext)) return doc("#7a6a2a", "ZIP");
  if (ext === "ics" || mime === "text/calendar") return <Icon name="calendar" />;
  if (ext === "eml" || mime === "message/rfc822") return <Icon name="mail" />;
  if (mime.startsWith("image/")) return <Icon name="picture" />;
  return doc("#6b7b8f", ext.slice(0, 3).toUpperCase() || "?");
}

/** Envelope state icon for a list row. */
export function rowIcon(r: { isRead: boolean; lastVerb: string | null; isInvite: boolean; isDraft: boolean; outboxId?: number | null }): string {
  if (r.outboxId) return "outbox";
  if (r.isDraft) return "drafts";
  if (r.isInvite) return "meeting-request";
  if (r.lastVerb === "forwarded") return "mail-forwarded";
  if (r.lastVerb) return "mail-replied";
  return r.isRead ? "mail-read" : "mail-unread";
}
