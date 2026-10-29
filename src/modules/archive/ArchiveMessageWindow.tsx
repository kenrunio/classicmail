// An archived message in its own window (the mail message window's Ribbon, read-only).
// Opened as `message` window with `?archive=<id>`.
import { useEffect, useState } from "react";
import { closeCurrentWindow, setWindowTitle, windowParams } from "../../api/windows";
import { archiveApi } from "../../api/archive";
import { Ribbon, RibbonBig, RibbonSmall, RibbonStack, alertBox, useHotkeys, type RibbonTab } from "../../ui";
import { MailDialogHost } from "../mail/dialogs";
import { ArchiveReader, useArchiveMessage } from "./ArchiveReader";
import { archiveCompose, printArchive } from "./actions";
import "../mail/mail.css";
import "./archive.css";

export default function ArchiveMessageWindow() {
  const [id, setId] = useState(Number(windowParams().get("archive")));
  const q = useArchiveMessage(id, false);
  const m = q.data;

  useEffect(() => {
    if (m) setWindowTitle(`${m.subject || "(no subject)"} - Message (${m.isPlain ? "Plain Text" : "HTML"})`);
  }, [m]);

  const go = async (direction: "next" | "previous") => {
    const n = await archiveApi.adjacent(id, direction).catch(() => null);
    if (n) {
      setId(n);
      history.replaceState(null, "", `?window=message&archive=${n}`);
    }
  };
  const headers = async () => {
    const h = await archiveApi.headers(id).catch(() => "");
    await alertBox(<pre className="cm-archive-headers">{h || "(no Internet headers)"}</pre>, "Message Options");
  };
  useHotkeys({
    "ctrl+r": () => archiveCompose("reply", id),
    "ctrl+shift+r": () => archiveCompose("replyAll", id),
    "ctrl+f": () => archiveCompose("forward", id),
    "ctrl+p": () => m && printArchive(m),
    "ctrl+.": () => go("next"),
    "ctrl+,": () => go("previous"),
    escape: () => closeCurrentWindow(),
  });

  const tabs: RibbonTab[] = [
    {
      id: "message",
      label: "Message",
      groups: [
        {
          label: "Respond",
          content: (
            <>
              <RibbonBig icon="reply" label="Reply" onClick={() => archiveCompose("reply", id)} />
              <RibbonBig icon="reply-all" label="Reply to All" onClick={() => archiveCompose("replyAll", id)} />
              <RibbonBig icon="forward" label="Forward" onClick={() => archiveCompose("forward", id)} />
            </>
          ),
        },
        {
          label: "Actions",
          content: (
            <RibbonStack>
              <RibbonSmall icon="print" label="Print" onClick={() => m && printArchive(m)} />
              <RibbonSmall icon="options" label="Message Options…" onClick={headers} />
            </RibbonStack>
          ),
        },
        {
          label: "Find",
          content: (
            <RibbonStack>
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
          { label: "Reply", icon: "reply", onClick: () => archiveCompose("reply", id) },
          { label: "Forward", icon: "forward", onClick: () => archiveCompose("forward", id) },
          { type: "separator" },
          { label: "Print", icon: "print", onClick: () => m && printArchive(m) },
          { type: "separator" },
          { label: "Close", icon: "close", onClick: () => closeCurrentWindow() },
        ]}
      />
      <div className="cm-itemwin-body">
        <ArchiveReader id={id} />
      </div>
      <MailDialogHost />
    </div>
  );
}
