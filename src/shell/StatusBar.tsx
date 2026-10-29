import { useStatus } from "./store";
import { Icon } from "../ui";
import { rpc } from "../api/rpc";
import { useEffect, useState } from "react";
import { useCmEvent } from "../api/events";
import { onUndoMessage } from "../api/undo";

export function StatusBar() {
  const { left, sync, message } = useStatus();
  useEffect(() => onUndoMessage((m) => useStatus.getState().flash(m, m.startsWith("Couldn't") ? 8000 : 4000)), []);
  const states = Object.values(sync);
  const err = states.find((s) => s.phase === "error");
  const busy = states.find((s) => s.phase === "sync");
  // background download of older mail (Options › Mail), per account: what is left
  const [download, setDownload] = useState<Record<number, { bodies: number; attachments: number }>>({});
  useCmEvent("mail.download", (e) => setDownload((d) => ({ ...d, [e.accountId as number]: { bodies: Number(e.bodies) || 0, attachments: Number(e.attachments) || 0 } })));
  const todo = Object.values(download).reduce((a, d) => ({ bodies: a.bodies + d.bodies, attachments: a.attachments + d.attachments }), { bodies: 0, attachments: 0 });
  const downloading =
    todo.bodies > 0
      ? ` · Downloading older mail: ${todo.bodies.toLocaleString()} left`
      : todo.attachments > 0
        ? ` · Downloading attachments: ${todo.attachments.toLocaleString()} left`
        : "";
  const right = err?.text ?? busy?.text ?? (states.length ? `All folders are up to date.${downloading}` : "Connecting…");
  const progress = busy?.progress;
  return (
    <div className="cm-statusbar">
      <span className={`cm-statusbar-left cm-ellipsis${message ? " cm-statusbar-flash" : ""}`} title={message || undefined}>
        {message || left}
      </span>
      <div className="cm-fill" />
      {progress != null && (
        <div className="cm-progress" title={`${Math.round(progress * 100)}%`}>
          <div style={{ width: `${Math.round(progress * 100)}%` }} />
        </div>
      )}
      <span className={`cm-statusbar-right cm-ellipsis${err ? " error" : ""}`} title={err ? "Click to retry Send/Receive" : undefined} onClick={() => err && rpc("sync.now", {})}>
        {err && <Icon name="warning" size={14} />}
        {right}
      </span>
    </div>
  );
}
