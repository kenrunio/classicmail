// Outlook 2007 read-message window: Ribbon (Message tab), header, InfoBars, attachments, body.
// Params: id (local message id), folderId? (for Previous / Next item).
import { useEffect, useState } from "react";
import { closeCurrentWindow, setWindowTitle, windowParams } from "../../../api/windows";
import { mailApi } from "../../../api/mail";
import { Ribbon, RibbonBig, RibbonSmall, RibbonStack, useHotkeys, type RibbonTab } from "../../../ui";
import { MessageReader, useMessage } from "../MessageReader";
import { MailDialogHost, messageOptionsDialog } from "../dialogs";
import { archiveItems, compose, replyWithAi, createRule, deleteItems, findFromSender, followUpMenu, junkMenu, markJunk, markRead, moveToFolder, printView } from "../actions";
import { useMailSel } from "../store";
import ArchiveMessageWindow from "../../archive/ArchiveMessageWindow";
import "../mail.css";

/** `?archive=<id>`: an item of an Outlook archive (read-only window); else a mailbox message. */
export default function MessageWindow() {
  return windowParams().get("archive") ? <ArchiveMessageWindow /> : <MailMessageWindow />;
}

function MailMessageWindow() {
  const p = windowParams();
  const [id, setId] = useState(Number(p.get("id")));
  const folderId = p.get("folderId") ? Number(p.get("folderId")) : undefined;
  const q = useMessage(id, false);
  const m = q.data;
  const [gone, setGone] = useState(false);

  useEffect(() => {
    if (m) setWindowTitle(`${m.subject || "(no subject)"} - Message (${m.isPlain ? "Plain Text" : "HTML"})`);
  }, [m?.subject, m?.isPlain, m]);
  // opening an item marks it as read
  useEffect(() => {
    if (m && !m.isRead) markRead([m.id], true, false);
  }, [m?.id]); // eslint-disable-line react-hooks/exhaustive-deps
  // mail.changed refetches the message (api/events.ts): failing after it was shown means it's gone
  const failedAfterShown = q.isError && q.data !== undefined;
  useEffect(() => {
    if (failedAfterShown) setGone(true);
  }, [failedAfterShown]);
  // actions in this window work on this message
  useEffect(() => useMailSel.getState().setSelection([id], id), [id]);

  const go = async (direction: "next" | "previous") => {
    const n = await mailApi.adjacent(id, direction, folderId).catch(() => null);
    if (n) {
      setGone(false);
      setId(n);
      history.replaceState(null, "", `?window=message&id=${n}${folderId ? `&folderId=${folderId}` : ""}`);
    }
  };
  const del = async () => {
    const next = await mailApi.adjacent(id, "next", folderId).catch(() => null);
    // "No" to the warning (or a failed delete): stay on this message
    if (!(await deleteItems([id]))) return;
    if (next) {
      setId(next);
    } else closeCurrentWindow();
  };
  const archive = async () => {
    const next = await mailApi.adjacent(id, "next", folderId).catch(() => null);
    if (!(await archiveItems([id]))) return;
    if (next) setId(next);
    else closeCurrentWindow();
  };
  useHotkeys({
    "ctrl+r": () => compose("replyAll", id),
    "ctrl+shift+r": () => compose("replyAll", id),
    "ctrl+f": () => compose("forward", id),
    "ctrl+d": del,
    delete: del,
    backspace: archive,
    "ctrl+p": () => m && printView(m),
    "ctrl+u": () => markRead([id], false),
    "ctrl+q": () => markRead([id], true),
    "ctrl+shift+v": () => moveToFolder([id], false, m?.accountId),
    "ctrl+.": () => go("next"),
    "ctrl+,": () => go("previous"),
    "ctrl+shift+>": () => go("next"),
    "ctrl+shift+<": () => go("previous"),
    escape: () => closeCurrentWindow(),
  });

  const flagged = !!m && (m.isFlagged || m.flagStatus === "flagged");
  const tabs: RibbonTab[] = [
    {
      id: "message",
      label: "Message",
      groups: [
        {
          label: "Respond",
          content: (
            <>
              <RibbonBig icon="reply" label="Reply" onClick={() => compose("replyAll", id)} />
              <RibbonBig icon="reply-ai" label="Reply with AI" onClick={() => replyWithAi(id)} />
              <RibbonBig icon="forward" label="Forward" onClick={() => compose("forward", id)} />
            </>
          ),
        },
        {
          label: "Actions",
          content: (
            <>
              <RibbonBig icon="delete" label="Delete" onClick={del} />
              <RibbonBig icon="archive" label="Archive" onClick={archive} />
              <RibbonStack>
                <RibbonSmall icon="move" label="Move to Folder" onClick={() => moveToFolder([id], false, m?.accountId)} />
                <RibbonSmall icon="rules" label="Create Rule…" onClick={() => createRule(id)} />
                <RibbonSmall
                  icon="options"
                  label="Other Actions"
                  menu={() => [
                    { label: "Print", icon: "print", onClick: () => m && printView(m) },
                    { label: "Message Options…", icon: "options", onClick: () => m && messageOptionsDialog(id, m.importance) },
                    { type: "separator" },
                    { label: "Copy to Folder…", icon: "copy", onClick: () => moveToFolder([id], true, m?.accountId) },
                  ]}
                />
              </RibbonStack>
            </>
          ),
        },
        {
          label: "Junk E-mail",
          content: (
            <RibbonStack>
              <RibbonSmall icon="junk" label="Block Sender" onClick={() => markJunk([id], true)} />
              <RibbonSmall icon="check" label="Not Junk" onClick={() => markJunk([id], false)} />
              <RibbonSmall
                icon="junk"
                label="Junk E-mail"
                menu={() =>
                  junkMenu(
                    () => [id],
                    () => m?.fromEmail,
                  )
                }
              />
            </RibbonStack>
          ),
        },
        {
          label: "Options",
          content: (
            <RibbonStack>
              <RibbonSmall icon="categorize" label="Categorize" disabled />
              <RibbonSmall icon={flagged ? "flag" : "flag-outline"} label="Follow Up" menu={() => followUpMenu(() => [id])} />
              <RibbonSmall icon="mark-unread" label="Mark as Unread" onClick={() => markRead([id], false)} />
            </RibbonStack>
          ),
        },
        {
          label: "Find",
          content: (
            <RibbonStack>
              <RibbonSmall icon="find" label="Related" menu={() => [{ label: "Messages from Sender", onClick: () => m && findFromSender(m.fromEmail) }]} />
              <RibbonSmall icon="arrow-up" label="Previous Item" onClick={() => go("previous")} />
              <RibbonSmall icon="arrow-down" label="Next Item" onClick={() => go("next")} />
            </RibbonStack>
          ),
        },
      ],
    },
  ];
  return (
    <div className="cm-itemwin cm-mail-msgwin">
      <Ribbon
        tabs={tabs}
        quickAccess={
          <>
            <button className="cm-rb-qat" title="Previous Item (Ctrl+,)" onClick={() => go("previous")}>
              ▲
            </button>
            <button className="cm-rb-qat" title="Next Item (Ctrl+.)" onClick={() => go("next")}>
              ▼
            </button>
          </>
        }
        orbMenu={[
          { label: "Reply", icon: "reply", onClick: () => compose("replyAll", id) },
          { label: "Forward", icon: "forward", onClick: () => compose("forward", id) },
          { type: "separator" },
          { label: "Print", icon: "print", onClick: () => m && printView(m) },
          { label: "Delete", icon: "delete", onClick: del },
          { type: "separator" },
          { label: "Close", icon: "close", onClick: () => closeCurrentWindow() },
        ]}
      />
      <div className="cm-itemwin-body">{gone ? <div className="cm-mail-rp-empty">This item has been moved or deleted.</div> : <MessageReader id={id} />}</div>
      <MailDialogHost />
    </div>
  );
}
