// An archived message in the Reading Pane / message window: the mail reader's look, read-only.
import { useEffect, useState } from "react";
import { useRpc } from "../../api/query";
import type { ArchiveAttachment, ArchiveMessageView } from "../../api/archive";
import { Icon, contextMenu, showMenu, type MenuItem } from "../../ui";
import { MessageBody } from "../mail/MessageBody";
import { markPainted } from "../../api/windows";
import { fullDate, sizeText } from "../mail/format";
import { AddressTokens } from "../mail/AddressTokens";
import { FileIcon } from "../mail/icons";
import { dragOut, prepareDragOut } from "../mail/actions";
import { attachmentTitle, openArchiveAttachment, saveAllArchiveAttachments, saveArchiveAttachment } from "./actions";

export function useArchiveMessage(id: number | null, allowRemote: boolean) {
  return useRpc<ArchiveMessageView>(
    ["archive", "message", id, allowRemote],
    "archive.getMessage",
    { id, allowRemote },
    { enabled: id !== null && id > 0, staleTime: 60_000, retry: false },
  );
}

export function ArchiveReader({ id, compact }: { id: number; compact?: boolean }) {
  // "download pictures" for this message only (a state reset in an effect would first fetch the
  // next message with pictures allowed)
  const [allowedId, setAllowedId] = useState<number | null>(null);
  const allow = allowedId === id;
  const q = useArchiveMessage(id, allow);
  const m = q.data;
  useEffect(() => {
    if (q.isError) markPainted();
  }, [q.isError]);
  if (q.isError) return <div className="cm-mail-rp-empty">{(q.error as Error).message}</div>;
  if (!m) return <div className="cm-mail-rp-empty">{q.isFetching ? "Loading…" : ""}</div>;
  return (
    <div className={`cm-mail-reader${compact ? " compact" : ""}`} data-zoom>
      <ArchiveHeader m={m} onDownload={() => setAllowedId(id)} />
      <MessageBody html={m.html} plain={m.isPlain} allowRemote={allow || (m.hasRemote && !m.remoteBlocked)} />
    </div>
  );
}

function ArchiveHeader({ m, onDownload }: { m: ArchiveMessageView; onDownload: () => void }) {
  const bars: { icon: string; text: React.ReactNode; onClick?: () => void; key: string }[] = [];
  if (m.importance === 2) bars.push({ key: "imp", icon: "importance-high", text: "This message was sent with High importance." });
  if (m.importance === 0) bars.push({ key: "low", icon: "importance-low", text: "This message was sent with Low importance." });
  if (m.lastVerb === "forwarded") bars.push({ key: "verb", icon: "mail-forwarded", text: "You forwarded this message." });
  else if (m.lastVerb) bars.push({ key: "verb", icon: "mail-replied", text: "You replied to this message." });
  if (m.flagStatus === "flagged") bars.push({ key: "flag", icon: "flag", text: "Follow up." });
  if (m.flagStatus === "complete") bars.push({ key: "done", icon: "flag-complete", text: "Completed." });
  if (m.remoteBlocked)
    bars.push({
      key: "pics",
      icon: "info",
      text: "Click here to download pictures. To help protect your privacy, ClassicMail prevented automatic download of some pictures in this message.",
      onClick: onDownload,
    });
  if (m.bodySource === "unreadable") bars.push({ key: "err", icon: "warning", text: "The contents of this item could not be read from the archive." });
  const atts = m.attachments.filter((a) => !a.isInline);
  return (
    <div className="cm-mail-rp-header">
      <div className="cm-mail-rp-subject" dir="auto">
        {m.subject || "(no subject)"}
      </div>
      <div className="cm-mail-rp-from">
        <AddressTokens list={[{ name: m.fromName, email: m.fromEmail }]} showEmail />
      </div>
      {bars.map((b) => (
        <div key={b.key} className={`cm-mail-infobar${b.onClick ? " clickable" : ""}`} onClick={b.onClick}>
          <Icon name={b.icon} size={14} />
          <span>{b.text}</span>
        </div>
      ))}
      <table className="cm-mail-rp-fields">
        <tbody>
          <tr>
            <td>Sent:</td>
            <td>{fullDate(m.sentAt ?? m.receivedAt)}</td>
          </tr>
          <tr>
            <td>To:</td>
            <td className="cm-ellipsis">
              <AddressTokens list={m.to} />
            </td>
          </tr>
          {m.cc.length > 0 && (
            <tr>
              <td>Cc:</td>
              <td className="cm-ellipsis">
                <AddressTokens list={m.cc} />
              </td>
            </tr>
          )}
          {m.bcc.length > 0 && (
            <tr>
              <td>Bcc:</td>
              <td className="cm-ellipsis">
                <AddressTokens list={m.bcc} />
              </td>
            </tr>
          )}
        </tbody>
      </table>
      {atts.length > 0 && <AttachmentStrip messageId={m.id} atts={atts} />}
    </div>
  );
}

function AttachmentStrip({ messageId, atts }: { messageId: number; atts: ArchiveAttachment[] }) {
  const [sel, setSel] = useState<number | null>(null);
  const menu = (a: ArchiveAttachment): MenuItem[] => [
    { label: "Open", bold: true, onClick: () => openArchiveAttachment(messageId, a) },
    { label: "Open With…", onClick: () => openArchiveAttachment(messageId, a, true) },
    { label: "Save As…", icon: "save", onClick: () => saveArchiveAttachment(messageId, a) },
    { label: "Save All Attachments…", onClick: () => saveAllArchiveAttachments(messageId) },
  ];
  return (
    <div className="cm-mail-atts">
      <span className="cm-mail-att message">
        <Icon name="mail" size={14} />
        Message
      </span>
      {atts.map((a) => (
        <span
          key={a.index}
          className={`cm-mail-att${sel === a.index ? " selected" : ""}`}
          title={attachmentTitle(a)}
          onMouseDown={() => {
            setSel(a.index);
            prepareDragOut({ archiveId: messageId, index: a.index });
          }}
          onDoubleClick={() => openArchiveAttachment(messageId, a)}
          draggable
          onDragStart={(e) => dragOut(e, { archiveId: messageId, index: a.index })}
          onContextMenu={(e) => {
            setSel(a.index);
            contextMenu(menu(a))(e);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") openArchiveAttachment(messageId, a);
            if (e.key === "ContextMenu") {
              const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
              showMenu(r.left, r.bottom, menu(a));
            }
          }}
        >
          <FileIcon name={a.filename} mime={a.mimeType} />
          <span dir="auto">{a.filename}</span>
          <span className="cm-muted">({sizeText(a.size)})</span>
        </span>
      ))}
    </div>
  );
}
