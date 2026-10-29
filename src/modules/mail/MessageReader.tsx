// A message as shown in the Reading Pane and the message window: header, InfoBars, meeting
// request bar, attachments strip and the sandboxed body — or, after a click on an attachment, its
// preview (Outlook's attachment previewers; "Message" goes back).
import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { rpc, RpcError } from "../../api/rpc";
import { useRpc } from "../../api/query";
import { mailApi, type Attachment, type AttachmentPreview, type Invite, type MessageView } from "../../api/mail";
import { Icon, alertBox, contextMenu, showMenu, type MenuItem } from "../../ui";
import { useTranslation } from "../ai/useTranslation";
import { MessageBody } from "./MessageBody";
import { BodyTabs } from "./BodyTabs";
import { MessageSourceView, sourceActions, type SourceTab } from "./MessageSource";
import { fullDate, longDay, sizeText } from "./format";
import { dragOut, openAttachment, prepareDragOut, saveAllAttachments, saveAttachment } from "./actions";
import { AddressTokens } from "./AddressTokens";
import { FileIcon } from "./icons";
import { promptDialog } from "./dialogs";
import { closeCurrentWindow, markPainted, windowParams } from "../../api/windows";

export function useMessage(id: number | null, allowRemote: boolean) {
  return useRpc<MessageView>(["mail", "message", id, allowRemote], "mail.getMessage", { id, allowRemote }, { enabled: id !== null && id > 0, staleTime: 60_000, retry: false });
}

export function MessageReader({ id, compact }: { id: number; compact?: boolean }) {
  // both belong to one message: on the next one they're off from the first render (asking for
  // its pictures even once would remember them for that message and its sender)
  const [allowFor, setAllowFor] = useState<number | null>(null);
  const allow = allowFor === id;
  // the attachment shown instead of the body (null = the message)
  const [previewOf, setPreviewOf] = useState<{ id: number; a: Attachment } | null>(null);
  const preview = previewOf?.id === id ? previewOf.a : null;
  const setPreview = (a: Attachment | null) => setPreviewOf(a ? { id, a } : null);
  // View Source: the tabs above the text, for this message only
  type SourceView = "message" | SourceTab;
  const [sourceFor, setSourceFor] = useState<{ id: number; tab: SourceView } | null>(null);
  const source = sourceFor?.id === id ? sourceFor.tab : null;
  const q = useMessage(id, allow);
  const m = q.data;
  const tr = useTranslation(m);
  useEffect(() => {
    if (q.isError) markPainted();
  }, [q.isError]);
  if (q.isError) return <div className="cm-mail-rp-empty">{(q.error as Error).message}</div>;
  if (!m) return <div className="cm-mail-rp-empty">{q.isFetching ? "Loading…" : ""}</div>;
  return (
    <div className={`cm-mail-reader${compact ? " compact" : ""}`} data-zoom>
      <Header m={m} onDownload={() => setAllowFor(id)} translateBar={tr.bar} subject={tr.subject} preview={preview} onPreview={setPreview} />
      {source && !preview && (
        <BodyTabs
          tabs={[
            { key: "message", label: "Message" },
            { key: "html", label: "HTML Source" },
            { key: "raw", label: "Original Message" },
          ]}
          active={source}
          onPick={(k) => setSourceFor({ id, tab: k as SourceView })}
          onClose={() => setSourceFor(null)}
        >
          {source !== "message" && (
            <>
              <button className="cm-button" onClick={() => void sourceActions(id, source, m.subject).copy()}>
                Copy
              </button>
              <button className="cm-button" onClick={() => void sourceActions(id, source, m.subject).save()}>
                Save As…
              </button>
            </>
          )}
        </BodyTabs>
      )}
      {preview ? (
        <AttachmentPreviewPane key={preview.id} a={preview} />
      ) : source && source !== "message" ? (
        <MessageSourceView id={id} tab={source} />
      ) : (
        <MessageBody
          html={m.html}
          plain={m.isPlain}
          allowRemote={allow || m.safeSender || (m.hasRemote && !m.remoteBlocked)}
          accountId={m.accountId}
          onDocument={tr.onBodyLoad}
          onViewSource={() => setSourceFor({ id, tab: "html" })}
        />
      )}
    </div>
  );
}

/** An attachment in place of the body: as HTML in the sandboxed body frame, a PDF in the WebView's
 *  own viewer, or why there's no preview (with Open / Save As). */
function AttachmentPreviewPane({ a }: { a: Attachment }) {
  // not under "mail": an attachment doesn't change, so mail.changed needn't rebuild its preview
  const q = useRpc<AttachmentPreview>(["attachmentPreview", a.id], "mail.previewAttachment", { id: a.id }, { staleTime: 5 * 60_000, retry: false });
  const p = q.data;
  if (q.isError || p?.kind === "none")
    return (
      <div className="cm-mail-preview-none">
        <FileIcon name={a.filename} mime={a.mimeType} />
        <div dir="auto">{a.filename}</div>
        <div className="cm-muted">{p?.kind === "none" ? p.note : `This file couldn't be previewed: ${(q.error as Error)?.message ?? ""}`}</div>
        <div className="cm-mail-preview-buttons">
          <button className="cm-button" onClick={() => openAttachment(a)}>
            Open
          </button>
          <button className="cm-button" onClick={() => saveAttachment(a)}>
            Save As…
          </button>
        </div>
      </div>
    );
  if (!p) return <div className="cm-mail-rp-empty">Preparing the preview…</div>;
  if (p.kind === "pdf") return <iframe className="cm-mail-preview-pdf" src={p.url} title={a.filename} />;
  return <MessageBody html={p.html} plain={false} allowRemote={false} />;
}

function Header({
  m,
  onDownload,
  translateBar,
  subject,
  preview,
  onPreview,
}: {
  m: MessageView;
  onDownload: () => void;
  translateBar?: React.ReactNode;
  subject?: string | null;
  preview: Attachment | null;
  onPreview: (a: Attachment | null) => void;
}) {
  const bars: { icon: string; text: React.ReactNode; menu?: () => MenuItem[]; onClick?: () => void; key: string }[] = [];
  if (m.folders.some((f) => f.role === "spam"))
    bars.push({ key: "junk", icon: "junk", text: "This message is in the Junk E-mail folder. Pictures are not downloaded automatically." });
  if (m.importance === 2) bars.push({ key: "imp", icon: "importance-high", text: "This message was sent with High importance." });
  if (m.importance === 0) bars.push({ key: "low", icon: "importance-low", text: "This message was sent with Low importance." });
  if (m.lastVerb && m.lastVerbAt) {
    const verb = m.lastVerb === "forwarded" ? "forwarded" : "replied to";
    bars.push({ key: "verb", icon: m.lastVerb === "forwarded" ? "mail-forwarded" : "mail-replied", text: `You ${verb} this message on ${fullDate(m.lastVerbAt)}.` });
  }
  if (m.flagStatus === "flagged" || m.isFlagged) bars.push({ key: "flag", icon: "flag", text: m.flagDue ? `Follow up. Due by ${longDay(m.flagDue)}.` : "Follow up." });
  if (m.flagStatus === "complete" && !m.isFlagged) bars.push({ key: "done", icon: "flag-complete", text: "Completed." });
  if (m.remoteBlocked)
    bars.push({
      key: "pics",
      icon: "info",
      text: "Click here to download pictures. To help protect your privacy, ClassicMail prevented automatic download of some pictures in this message.",
      onClick: onDownload,
      menu: () => [
        { label: "Download Pictures", icon: "download-pictures", onClick: onDownload },
        { type: "separator" },
        { label: "Add Sender to Safe Senders List", onClick: () => mailApi.addSafeSender(m.fromEmail).catch(() => {}) },
        { label: `Add the Domain @${m.fromEmail.split("@")[1] ?? ""} to Safe Senders List`, onClick: () => mailApi.addSafeSender(`@${m.fromEmail.split("@")[1]}`).catch(() => {}) },
      ],
    });
  if (m.bodyError) bars.push({ key: "err", icon: "warning", text: `The full message could not be downloaded: ${m.bodyError}` });
  const atts = m.attachments.filter((a) => !a.isInline);
  return (
    <div className="cm-mail-rp-header">
      {m.invite && (m.invite.method === "REPLY" || m.invite.method === "CANCEL" ? <InviteNoticeBar invite={m.invite} /> : <InviteBar m={m} invite={m.invite} />)}
      <div className="cm-mail-rp-subject" dir="auto" title={subject ? m.subject : undefined}>
        {subject || m.subject || "(no subject)"}
      </div>
      <div className="cm-mail-rp-from">
        <AddressTokens list={[{ name: m.fromName, email: m.fromEmail }]} showEmail />
      </div>
      {bars.map((b) => (
        <div
          key={b.key}
          className={`cm-mail-infobar${b.onClick || b.menu ? " clickable" : ""}`}
          onClick={(e) => {
            if (b.menu) {
              const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
              showMenu(e.clientX, r.bottom, b.menu());
            } else b.onClick?.();
          }}
          onContextMenu={b.menu ? contextMenu(b.menu) : undefined}
        >
          <Icon name={b.icon} size={14} />
          <span>{b.text}</span>
        </div>
      ))}
      {translateBar}
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
      {atts.length > 0 && <AttachmentStrip messageId={m.id} atts={atts} preview={preview} onPreview={onPreview} />}
    </div>
  );
}

/** "Message" + the attachments: a click previews one in place of the body, "Message" goes back,
 *  a double-click opens it in its program. */
function AttachmentStrip({
  messageId,
  atts,
  preview,
  onPreview,
}: {
  messageId: number;
  atts: Attachment[];
  preview: Attachment | null;
  onPreview: (a: Attachment | null) => void;
}) {
  const menu = (a: Attachment): MenuItem[] => [
    { label: "Preview", onClick: () => onPreview(a) },
    { label: "Open", bold: true, onClick: () => openAttachment(a) },
    { label: "Open With…", onClick: () => openAttachment(a, true) },
    { label: "Save As…", icon: "save", onClick: () => saveAttachment(a) },
    { label: "Save All Attachments…", onClick: () => saveAllAttachments(messageId) },
  ];
  return (
    <div className="cm-mail-atts">
      <span className={`cm-mail-att message${preview ? "" : " selected"}`} title="Show the message" onMouseDown={() => onPreview(null)}>
        <Icon name="mail" size={14} />
        Message
      </span>
      {atts.map((a) => (
        <span
          key={a.id}
          className={`cm-mail-att${preview?.id === a.id ? " selected" : ""}`}
          title={`${a.filename} (${sizeText(a.size)})\nClick to preview, double-click to open, drag to a folder to save`}
          onMouseDown={(e) => {
            if (e.button !== 0) return;
            onPreview(a);
            prepareDragOut({ attachmentId: a.id });
          }}
          onDoubleClick={() => openAttachment(a)}
          onContextMenu={contextMenu(menu(a))}
          // drag it onto a folder, the desktop or another program
          draggable
          onDragStart={(e) => dragOut(e, { attachmentId: a.id })}
        >
          <FileIcon name={a.filename} mime={a.mimeType} />
          <span dir="auto">{a.filename}</span>
          <span className="cm-muted">({sizeText(a.size)})</span>
        </span>
      ))}
    </div>
  );
}

interface CalEvent {
  id: number;
  myResponse?: string;
  start: number;
  end: number;
}

function InviteBar({ m, invite }: { m: MessageView; invite: Invite }) {
  const found = useQuery({
    queryKey: ["calendar", "byIcalUid", m.accountId, invite.uid],
    queryFn: () => rpc<CalEvent | null>("calendar.findByIcalUid", { accountId: m.accountId, icalUid: invite.uid }).catch(() => null),
    staleTime: 30_000,
  });
  const [busy, setBusy] = useState(false);
  const [answered, setAnswered] = useState<string | null>(null);
  const respond = async (response: "accepted" | "tentative" | "declined", sendResponse = true, comment?: string) => {
    setBusy(true);
    try {
      await rpc("calendar.respondToInvite", { accountId: m.accountId, icalUid: invite.uid, response, sendResponse, ...(comment ? { comment } : {}) });
      setAnswered(response);
      found.refetch();
      // Outlook: "Delete meeting request from Inbox when responding" (on by default)
      const del = await rpc<boolean | null>("settings.get", { key: "mail.deleteInviteOnResponse" }).catch(() => null);
      if (del !== false && !m.folders.some((f) => f.role === "trash")) {
        await mailApi.delete([m.id]).catch(() => {});
        if (windowParams().get("window") === "message") closeCurrentWindow();
      }
    } catch (e) {
      if (e instanceof RpcError && e.code === "no_method") alertBox("The calendar isn't available yet, so this meeting can't be answered from here.", "ClassicMail", "info");
      else alertBox(e instanceof Error ? e.message : String(e), "ClassicMail", "error");
    } finally {
      setBusy(false);
    }
  };
  const respMenu = (r: "accepted" | "tentative" | "declined"): MenuItem[] => [
    {
      label: "Edit the Response before Sending",
      onClick: async () => {
        const verb = { accepted: "Accepted", tentative: "Tentative", declined: "Declined" }[r];
        const c = await promptDialog(`${verb}: ${invite.summary || m.subject}`, "Add a note to your response:", "", true);
        if (c !== null) respond(r, true, c.trim() || undefined);
      },
    },
    { label: "Send the Response Now", bold: true, onClick: () => respond(r, true) },
    { label: "Don't Send a Response", onClick: () => respond(r, false) },
  ];
  const my = answered ?? found.data?.myResponse;
  const btn = (r: "accepted" | "tentative" | "declined", icon: string, label: string) => (
    <button
      className={`cm-mail-invite-btn${my === r ? " pressed" : ""}`}
      disabled={busy}
      onClick={(e) => {
        const rr = e.currentTarget.getBoundingClientRect();
        showMenu(rr.left, rr.bottom, respMenu(r));
      }}
    >
      <Icon name={icon} size={16} />
      {label}
      <Icon name="chevron-down" size={8} />
    </button>
  );
  return (
    <div className="cm-mail-invite">
      <div className="cm-mail-invite-buttons">
        {btn("accepted", "accept", "Accept")}
        {btn("tentative", "tentative", "Tentative")}
        {btn("declined", "decline", "Decline")}
      </div>
      <div className="cm-mail-infobar">
        <Icon name="meeting-request" size={14} />
        <span>
          {my === "accepted"
            ? "You have accepted this meeting."
            : my === "tentative"
              ? "You have tentatively accepted this meeting."
              : my === "declined"
                ? "You have declined this meeting."
                : found.data
                  ? "Please respond. This meeting is on your calendar."
                  : "Please respond."}
        </span>
      </div>
      <InviteFields invite={invite} />
    </div>
  );
}

function inviteWhen(invite: Invite): string {
  if (!invite.start) return "";
  if (invite.allDay) return new Date(invite.start).toLocaleDateString(undefined, { weekday: "long", year: "numeric", month: "long", day: "numeric", timeZone: "UTC" });
  return `${fullDate(invite.start)}${invite.end ? ` - ${new Date(invite.end).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })}` : ""}`;
}

function InviteFields({ invite }: { invite: Invite }) {
  const when = inviteWhen(invite);
  if (!when && !invite.location) return null;
  return (
    <table className="cm-mail-rp-fields">
      <tbody>
        {when && (
          <tr>
            <td>When:</td>
            <td>{when}</td>
          </tr>
        )}
        {invite.location && (
          <tr>
            <td>Where:</td>
            <td dir="auto">{invite.location}</td>
          </tr>
        )}
      </tbody>
    </table>
  );
}

/** An attendee's answer to your meeting ("Accepted: …"), or a cancellation: nothing to respond to, as in Outlook. */
function InviteNoticeBar({ invite }: { invite: Invite }) {
  const who = invite.replyFrom || "The attendee";
  const status = (invite.replyStatus ?? "").toUpperCase();
  const [icon, text] =
    invite.method === "CANCEL"
      ? ["decline", "This meeting has been cancelled."]
      : status === "ACCEPTED"
        ? ["accept", `${who} has accepted this meeting.`]
        : status === "TENTATIVE"
          ? ["tentative", `${who} has tentatively accepted this meeting.`]
          : status === "DECLINED"
            ? ["decline", `${who} has declined this meeting.`]
            : ["meeting-request", `${who} has replied to this meeting.`];
  return (
    <div className="cm-mail-invite">
      <div className="cm-mail-infobar">
        <Icon name={icon} size={14} />
        <span>{text}</span>
      </div>
      <InviteFields invite={invite} />
    </div>
  );
}
