// Calendar's own modal dialogs (Outlook "Open Recurring Item", "Confirm Delete", "Go To Date"...):
// promise-based, rendered by <CalDialogHost/> (mounted by the calendar view and item windows).
import { useState } from "react";
import { create } from "zustand";
import { Dialog, Icon } from "../../ui";
import { atMinutes, fmtLongDate, fmtTime, minutesOf, parseDayKey, dayKey } from "./dates";
import { DateField, TimeField } from "./windows/fields";
import type { ViewKind } from "./dates";

type Opt = { value: string; label: string };
type Req =
  | { kind: "radio"; title: string; text: string; options: Opt[]; value: string; icon?: string; resolve: (v: string | null) => void }
  | { kind: "prompt"; title: string; text: string; value: string; multiline?: boolean; resolve: (v: string | null) => void }
  | { kind: "goto"; date: number; view: ViewKind; resolve: (v: { date: number; view: ViewKind } | null) => void }
  | { kind: "propose"; summary: string; start: number; end: number; resolve: (v: Proposal | null) => void };

export type Proposal = { start: number; end: number; note: string };

const useDlg = create<{ stack: Req[]; push: (r: Req) => void; pop: (r: Req) => void }>((set) => ({
  stack: [],
  push: (r) => set((s) => ({ stack: [...s.stack, r] })),
  pop: (r) => set((s) => ({ stack: s.stack.filter((x) => x !== r) })),
}));

/** Radio-button question (Outlook's "Open this occurrence / Open the series"). */
export function radioBox(title: string, text: string, options: Opt[], value = options[0]?.value, icon = "help"): Promise<string | null> {
  return new Promise((resolve) => useDlg.getState().push({ kind: "radio", title, text, options, value, icon, resolve }));
}

export function promptBox(title: string, text: string, value = "", multiline = false): Promise<string | null> {
  return new Promise((resolve) => useDlg.getState().push({ kind: "prompt", title, text, value, multiline, resolve }));
}

export function goToDateBox(date: number, view: ViewKind): Promise<{ date: number; view: ViewKind } | null> {
  return new Promise((resolve) => useDlg.getState().push({ kind: "goto", date, view, resolve }));
}

/** Outlook's "Propose New Time": a new start / end for a meeting, and a note for the organizer. */
export function proposeTimeBox(summary: string, start: number, end: number): Promise<Proposal | null> {
  return new Promise((resolve) => useDlg.getState().push({ kind: "propose", summary, start, end, resolve }));
}

export function CalDialogHost() {
  const stack = useDlg((s) => s.stack);
  const top = stack[stack.length - 1];
  if (!top) return null;
  const done = (v: unknown) => {
    useDlg.getState().pop(top);
    (top.resolve as (x: unknown) => void)(v);
  };
  if (top.kind === "radio") return <RadioDlg key={stack.length} r={top} done={done} />;
  if (top.kind === "prompt") return <PromptDlg key={stack.length} r={top} done={done} />;
  if (top.kind === "propose") return <ProposeDlg key={stack.length} r={top} done={done} />;
  return <GotoDlg key={stack.length} r={top} done={done} />;
}

function Buttons({ ok, cancel }: { ok: () => void; cancel: () => void }) {
  return (
    <>
      <button className="cm-button default" onClick={ok}>
        OK
      </button>
      <button className="cm-button" onClick={cancel}>
        Cancel
      </button>
    </>
  );
}

function RadioDlg({ r, done }: { r: Extract<Req, { kind: "radio" }>; done: (v: unknown) => void }) {
  const [v, setV] = useState(r.value);
  return (
    <Dialog title={r.title} onClose={() => done(null)} width={430} footer={<Buttons ok={() => done(v)} cancel={() => done(null)} />}>
      <div
        className="cm-cal-dlg"
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            done(v);
          }
        }}
      >
        {r.icon && <Icon name={r.icon} size={32} />}
        <div className="cm-fill">
          <div className="cm-cal-dlg-text">{r.text}</div>
          {r.options.map((o) => (
            <label key={o.value} className="cm-cal-radio">
              <input type="radio" checked={v === o.value} onChange={() => setV(o.value)} autoFocus={v === o.value} />
              {o.label}
            </label>
          ))}
        </div>
      </div>
    </Dialog>
  );
}

function PromptDlg({ r, done }: { r: Extract<Req, { kind: "prompt" }>; done: (v: unknown) => void }) {
  const [v, setV] = useState(r.value);
  return (
    <Dialog title={r.title} onClose={() => done(null)} width={420} footer={<Buttons ok={() => done(v)} cancel={() => done(null)} />}>
      <div className="cm-cal-dlg-text">{r.text}</div>
      {r.multiline ? (
        <textarea className="cm-input" rows={5} style={{ width: "100%" }} value={v} onChange={(e) => setV(e.target.value)} autoFocus />
      ) : (
        <input
          className="cm-input"
          style={{ width: "100%" }}
          value={v}
          onChange={(e) => setV(e.target.value)}
          autoFocus
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              done(v);
            }
          }}
        />
      )}
    </Dialog>
  );
}

function GotoDlg({ r, done }: { r: Extract<Req, { kind: "goto" }>; done: (v: unknown) => void }) {
  const [d, setD] = useState(dayKey(r.date));
  const [view, setView] = useState<ViewKind>(r.view);
  const ok = () => {
    const t = parseDayKey(d);
    if (!isNaN(t.getTime())) done({ date: t.getTime(), view });
  };
  return (
    <Dialog title="Go To Date" onClose={() => done(null)} width={330} footer={<Buttons ok={ok} cancel={() => done(null)} />}>
      <div
        className="cm-cal-goto"
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            ok();
          }
        }}
      >
        <label>Date:</label>
        <input className="cm-input" type="date" value={d} onChange={(e) => setD(e.target.value)} autoFocus />
        <label>Show in:</label>
        <select className="cm-input" value={view} onChange={(e) => setView(e.target.value as ViewKind)}>
          <option value="day">Day Calendar</option>
          <option value="workweek">Work Week Calendar</option>
          <option value="week">Week Calendar</option>
          <option value="month">Month Calendar</option>
        </select>
        <span />
        <span className="cm-muted">
          {(() => {
            const t = parseDayKey(d);
            return isNaN(t.getTime()) ? "" : fmtLongDate(t);
          })()}
        </span>
      </div>
    </Dialog>
  );
}

function ProposeDlg({ r, done }: { r: Extract<Req, { kind: "propose" }>; done: (v: unknown) => void }) {
  const [start, setStart] = useState(r.start);
  const [end, setEnd] = useState(r.end);
  const [note, setNote] = useState("");
  const len = r.end - r.start;
  // moving the start keeps the length (as in the appointment window)
  const moveStart = (t: number) => {
    setStart(t);
    setEnd(t + len);
  };
  const bad = end <= start;
  const ok = () => !bad && done({ start, end, note: note.trim() });
  return (
    <Dialog title={`Propose New Time: ${r.summary || "Untitled"}`} onClose={() => done(null)} width={440} footer={<Buttons ok={ok} cancel={() => done(null)} />}>
      <div className="cm-cal-dlg-text">
        Current meeting time: {fmtLongDate(r.start)} {fmtTime(r.start)} - {fmtTime(r.end)}
      </div>
      <div className="cm-cal-goto" style={{ gridTemplateColumns: "80px auto auto 1fr" }}>
        <label>Start time:</label>
        <DateField value={dayKey(start)} onChange={(v) => moveStart(atMinutes(parseDayKey(v), minutesOf(start)))} />
        <TimeField value={minutesOf(start)} onChange={(m) => moveStart(atMinutes(start, m))} />
        <span />
        <label>End time:</label>
        <DateField value={dayKey(end)} onChange={(v) => setEnd(atMinutes(parseDayKey(v), minutesOf(end)))} />
        <TimeField value={minutesOf(end)} onChange={(m) => setEnd(atMinutes(end, m))} durationFrom={dayKey(start) === dayKey(end) ? minutesOf(start) : undefined} />
        <span />
      </div>
      {bad && <div className="cm-error-bar">The end time you entered occurs before the start time.</div>}
      <div className="cm-cal-dlg-text" style={{ margin: "10px 0 4px" }}>
        Note for the organizer (optional):
      </div>
      <textarea className="cm-input" rows={3} style={{ width: "100%" }} value={note} onChange={(e) => setNote(e.target.value)} />
    </Dialog>
  );
}
