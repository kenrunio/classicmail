// Outlook 2007 Reminders window: details of the selected reminder, Subject / Due in list,
// Dismiss All / Open Item / Dismiss, and "Click Snooze to be reminded again in:".
import { useEffect, useMemo, useRef, useState } from "react";
import { useRpc } from "../../../api/query";
import { queryClient } from "../../../api/query";
import { useCmEvent } from "../../../api/events";
import { cal, type ReminderItem } from "../../../api/calendar";
import { closeCurrentWindow, openWindow, setWindowTitle } from "../../../api/windows";
import { errorBox, Icon, useHotkeys } from "../../../ui";
import { fmtDuration, fmtLongDate, fmtTime } from "../dates";
import "../calendar.css";

const SNOOZE = [
  { m: 5, label: "5 minutes" },
  { m: 10, label: "10 minutes" },
  { m: 15, label: "15 minutes" },
  { m: 30, label: "30 minutes" },
  { m: 60, label: "1 hour" },
  { m: 120, label: "2 hours" },
  { m: 240, label: "4 hours" },
  { m: 480, label: "8 hours" },
  { m: 720, label: "0.5 days" },
  { m: 1440, label: "1 day" },
  { m: 2880, label: "2 days" },
  { m: 4320, label: "3 days" },
  { m: 5760, label: "4 days" },
  { m: 10080, label: "1 week" },
  { m: 20160, label: "2 weeks" },
];

export function dueIn(it: ReminderItem, now: number): string {
  const diff = Math.round((it.start - now) / 60_000);
  if (Math.abs(diff) < 1) return "Now";
  if (diff > 0) return fmtDuration(diff);
  return `${fmtDuration(-diff)} overdue`;
}

function chime() {
  try {
    const ctx = new AudioContext();
    [880, 660].forEach((f, i) => {
      const o = ctx.createOscillator();
      const g = ctx.createGain();
      o.frequency.value = f;
      g.gain.setValueAtTime(0.0001, ctx.currentTime + i * 0.18);
      g.gain.exponentialRampToValueAtTime(0.15, ctx.currentTime + i * 0.18 + 0.02);
      g.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + i * 0.18 + 0.3);
      o.connect(g).connect(ctx.destination);
      o.start(ctx.currentTime + i * 0.18);
      o.stop(ctx.currentTime + i * 0.18 + 0.32);
    });
  } catch {
    /* no audio */
  }
}

export default function RemindersWindow() {
  const q = useRpc<ReminderItem[]>(["calendar", "reminders"], "calendar.dueReminders", {}, { refetchInterval: 30_000 });
  const items = useMemo(() => q.data ?? [], [q.data]);
  const [sel, setSel] = useState<string | null>(null);
  const [snooze, setSnooze] = useState(5);
  const [now, setNow] = useState(Date.now());
  const seen = useRef(new Set<string>());
  useCmEvent("reminder.due", () => queryClient.invalidateQueries({ queryKey: ["calendar", "reminders"] }));

  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 15_000);
    return () => window.clearInterval(t);
  }, []);
  useEffect(() => {
    if (!q.isSuccess) return;
    if (items.length === 0) {
      closeCurrentWindow();
      return;
    }
    setWindowTitle(`${items.length} Reminder${items.length === 1 ? "" : "s"}`);
    if (!sel || !items.some((i) => i.key === sel)) setSel(items[0].key);
    if (items.some((i) => !seen.current.has(i.key))) chime();
    items.forEach((i) => seen.current.add(i.key));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [items, q.isSuccess]);

  const cur = items.find((i) => i.key === sel) ?? items[0];
  const after = async (p: Promise<unknown>) => {
    try {
      await p;
    } catch (e) {
      errorBox(e);
    }
    queryClient.invalidateQueries({ queryKey: ["calendar", "reminders"] });
  };
  const open = (it: ReminderItem | undefined) => {
    if (!it) return;
    if (it.kind === "task") openWindow("task", { id: it.eventId }, { title: `${it.summary} - Task`, label: `task-${it.eventId}` });
    else openWindow("appointment", { id: it.eventId, scope: "occurrence" }, { title: `${it.summary || "Untitled"} - Appointment`, label: `appointment-${it.eventId}-occurrence` });
  };
  // snoozing "before start" makes sense only for items that haven't started yet
  const options = cur && cur.start > now && cur.kind !== "task" ? [{ m: 0, label: "0 minutes before start" }, ...SNOOZE] : SNOOZE;
  // a choice that is no longer offered (0 minutes, once the item has started) falls back to 5
  const snoozeFor = options.some((o) => o.m === snooze) ? snooze : 5;

  useHotkeys({
    enter: () => open(cur),
    delete: () => cur && after(cal.dismiss([cur.key])),
    arrowdown: () => {
      const i = items.findIndex((x) => x.key === cur?.key);
      if (items[i + 1]) setSel(items[i + 1].key);
    },
    arrowup: () => {
      const i = items.findIndex((x) => x.key === cur?.key);
      if (i > 0) setSel(items[i - 1].key);
    },
  });

  if (!cur)
    return (
      <div className="cm-cal-remwin">
        <div className="cm-empty">{q.isLoading ? "Loading…" : "No reminders."}</div>
      </div>
    );
  return (
    <div className="cm-cal-remwin">
      <div className="cm-cal-rem-head">
        <Icon name={cur.kind === "task" ? "task" : "reminder"} size={32} />
        <div className="cm-fill">
          <div className="cm-cal-rem-subj cm-ellipsis">{cur.summary || "(No subject)"}</div>
          <div className="cm-ellipsis">
            {cur.kind === "task" ? "Reminder time" : "Start time"}: {cur.allDay ? fmtLongDate(cur.start) : `${fmtLongDate(cur.start)} ${fmtTime(cur.start)}`}
          </div>
          {cur.location && <div className="cm-ellipsis">Location: {cur.location}</div>}
        </div>
      </div>
      <div className="cm-cal-rem-list">
        <div className="cm-cal-rem-row head">
          <span className="icon" />
          <span className="subj">Subject</span>
          <span className="due">Due in</span>
        </div>
        <div className="cm-cal-rem-body">
          {items.map((it) => (
            <div
              key={it.key}
              className={`cm-cal-rem-row${it.key === cur.key ? " selected" : ""}${it.start < now ? " overdue" : ""}`}
              onMouseDown={() => setSel(it.key)}
              onDoubleClick={() => open(it)}
            >
              <span className="icon">
                <Icon name={it.kind === "task" ? "task" : it.isMeeting ? "meeting" : "appointment"} size={14} />
              </span>
              <span className="subj cm-ellipsis">{it.summary || "(No subject)"}</span>
              <span className="due cm-ellipsis">{dueIn(it, now)}</span>
            </div>
          ))}
        </div>
      </div>
      <div className="cm-cal-rem-buttons">
        <button className="cm-button" onClick={() => after(cal.dismiss(items.map((i) => i.key)))}>
          Dismiss All
        </button>
        <div className="cm-fill" />
        <button className="cm-button" onClick={() => open(cur)}>
          Open Item
        </button>
        <button className="cm-button default" onClick={() => after(cal.dismiss([cur.key]))}>
          Dismiss
        </button>
      </div>
      <div className="cm-cal-rem-snooze-label">Click Snooze to be reminded again in:</div>
      <div className="cm-cal-rem-buttons">
        <select className="cm-input cm-fill" value={snoozeFor} onChange={(e) => setSnooze(Number(e.target.value))}>
          {options.map((o) => (
            <option key={o.m} value={o.m}>
              {o.label}
            </option>
          ))}
        </select>
        <button className="cm-button" onClick={() => after(cal.snooze([cur.key], snoozeFor))}>
          Snooze
        </button>
      </div>
    </div>
  );
}
