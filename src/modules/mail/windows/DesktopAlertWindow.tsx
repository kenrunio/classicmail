// Outlook 2007 New Mail Desktop Alert: translucent box at the bottom right; fades in, stays ~7 s,
// fades out; hovering keeps it; click opens the message. Several messages cycle (◂ 1 of 3 ▸).
import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { emitTo, listen } from "@tauri-apps/api/event";
import { undoable } from "../../../api/undo";
import { openWindow, windowParams } from "../../../api/windows";
import type { NewMailEvent } from "../../../api/mailCompose";
import { ALERT_CLOSING, ALERT_EVENT, ALERT_READY, trimAlert } from "../compose/alerts";
import { CIcon } from "../compose/icons";
import "../compose/compose.css";

const STAY_MS = 7000;
const CYCLE_MS = 3500;
const FADE_MS = 1200;

type Msg = NewMailEvent["messages"][number] & { accountId: number };

function initial(): Msg[] {
  try {
    const d = trimAlert(JSON.parse(windowParams().get("data") ?? "{}") as NewMailEvent);
    return d.messages.map((m) => ({ ...m, accountId: d.accountId }));
  } catch {
    return [];
  }
}

export default function DesktopAlertWindow() {
  const [msgs, setMsgs] = useState<Msg[]>(initial);
  const [idx, setIdx] = useState(0);
  const [phase, setPhase] = useState<"in" | "shown" | "fading">("in");
  const [hover, setHover] = useState(false);
  const deadline = useRef(Date.now() + STAY_MS);
  const closed = useRef(false);

  const close = useCallback(() => {
    if (closed.current) return;
    closed.current = true;
    // the main window holds new mail back from now on (it opens a new alert for it)
    void emitTo("main", ALERT_CLOSING)
      .catch(() => {})
      .then(() => getCurrentWindow().destroy())
      .catch(() => getCurrentWindow().close());
  }, []);

  useEffect(() => {
    document.title = "New Mail Desktop Alert";
    const t = setTimeout(() => setPhase("shown"), 30);
    const un = listen<NewMailEvent>(ALERT_EVENT, ({ payload }) => {
      const d = trimAlert(payload);
      setMsgs((m) => [...m, ...d.messages.map((x) => ({ ...x, accountId: d.accountId }))]);
      setPhase("shown");
      deadline.current = Date.now() + STAY_MS;
    });
    // listening now: the main window sends what came while this window opened
    un.then(() => emitTo("main", ALERT_READY)).catch(() => {});
    return () => {
      clearTimeout(t);
      un.then((f) => f());
    };
  }, []);

  // stay / fade / close; hover pauses
  useEffect(() => {
    const t = setInterval(() => {
      if (hover) {
        deadline.current = Date.now() + STAY_MS;
        if (phase === "fading") setPhase("shown");
        return;
      }
      const left = deadline.current - Date.now();
      if (left <= 0 && phase !== "fading") setPhase("fading");
      if (left <= -FADE_MS) close();
    }, 200);
    return () => clearInterval(t);
  }, [hover, phase, close]);

  // cycle through several new items
  useEffect(() => {
    if (msgs.length < 2 || hover) return;
    const t = setInterval(() => setIdx((i) => (i + 1) % msgs.length), CYCLE_MS);
    return () => clearInterval(t);
  }, [msgs.length, hover]);

  useEffect(() => {
    if (msgs.length === 0) close();
  }, [msgs.length, close]);

  const m = msgs[Math.min(idx, msgs.length - 1)];
  if (!m) return null;

  const act = (fn: () => Promise<unknown>) => (e: React.MouseEvent) => {
    e.stopPropagation();
    fn().catch(() => {});
    // the item is handled: drop it from the alert
    setMsgs((all) => all.filter((x) => x !== m));
    setIdx(0);
  };
  const ids = m.id ? [m.id] : [];

  return (
    <div
      className={`cm-alert ${phase === "in" ? "" : phase}${hover ? " hover" : ""}`}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onClick={async () => {
        if (m.id) {
          // wait until the message window exists: destroying this window first can cancel the request
          const w = await openWindow("message", { id: m.id }, { title: m.subject || "Message" }).catch(() => null);
          if (w)
            await new Promise<void>((res) => {
              const t = setTimeout(res, 2000);
              w.once("tauri://created", () => (clearTimeout(t), res())).catch(() => res());
              w.once("tauri://error", () => (clearTimeout(t), res())).catch(() => res());
            });
        }
        close();
      }}
      title="Click to open this item"
    >
      <div className="cm-alert-icon">
        <CIcon name="mail-unread" size={32} />
      </div>
      <div className="cm-alert-text" dir="auto">
        <div className="cm-alert-from">{m.from}</div>
        <div className="cm-alert-subject">{m.subject || "(no subject)"}</div>
        <div className="cm-alert-preview">{m.preview}</div>
      </div>
      <div className="cm-alert-side">
        <button
          className="cm-alert-btn"
          title="Close"
          onClick={(e) => {
            e.stopPropagation();
            close();
          }}
        >
          <CIcon name="close" size={10} />
        </button>
        {msgs.length > 1 && (
          <div className="cm-alert-count" onClick={(e) => e.stopPropagation()}>
            <span style={{ cursor: "pointer" }} onClick={() => setIdx((i) => (i - 1 + msgs.length) % msgs.length)}>
              ◂
            </span>{" "}
            {Math.min(idx, msgs.length - 1) + 1} of {msgs.length}{" "}
            <span style={{ cursor: "pointer" }} onClick={() => setIdx((i) => (i + 1) % msgs.length)}>
              ▸
            </span>
          </div>
        )}
        <div className="cm-alert-btns">
          <button className="cm-alert-btn" title="Flag Item" disabled={!m.id} onClick={act(() => undoable("Flag (1 item)", "mail.setFlag", { ids, flag: "today" }))}>
            <CIcon name="flag" />
          </button>
          <button className="cm-alert-btn" title="Delete Item" disabled={!m.id} onClick={act(() => undoable("Delete 1 item", "mail.delete", { ids }))}>
            <CIcon name="delete" />
          </button>
          <button className="cm-alert-btn" title="Mark as Read" disabled={!m.id} onClick={act(() => undoable("Mark 1 item as Read", "mail.markRead", { ids, read: true }))}>
            <CIcon name="mark-read" />
          </button>
        </div>
      </div>
    </div>
  );
}
