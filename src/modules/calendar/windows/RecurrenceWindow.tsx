// Outlook "Appointment Recurrence" dialog (a separate window).
// Params: req (request id), start, end (ms), allDay (0/1), rule (existing RRULE lines, "\n"-joined).
// Answers the opener with the global Tauri event "cm-recurrence-result":
//   { req, action: "ok", recurrence: string[], start, end, allDay } | { req, action: "remove" } | { req, action: "cancel" }
import { useEffect, useMemo, useRef, useState } from "react";
import { emit } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { closeCurrentWindow, setWindowTitle, windowParams } from "../../../api/windows";
import { useHotkeys } from "../../../ui";
import { atMinutes, fmtDuration, minutesOf, parseDayKey, plusDays, dayKey, MIN } from "../dates";
import { buildRRule, CODES, DAY_NAMES, MONTHS, NTH_KIND_LABEL, parseRecurrence, type NthKind, type RecPattern } from "../recurrence";
import { DateField, TimeField } from "./fields";
import "../calendar.css";

const DURATIONS = [0, 5, 10, 15, 30, 60, 90, 120, 150, 180, 240, 300, 360, 420, 480, 540, 600, 660, 720, 1080, 1440, 2880, 4320, 5760, 10080];
const NTH_OPTS = [1, 2, 3, 4, -1];
const NTH_LABEL: Record<number, string> = { 1: "first", 2: "second", 3: "third", 4: "fourth", [-1]: "last" };
const KIND_OPTS: NthKind[] = ["day", "weekday", "weekendday", "SU", "MO", "TU", "WE", "TH", "FR", "SA"];

export default function RecurrenceWindow() {
  const params = useMemo(() => windowParams(), []);
  const req = params.get("req") ?? "";
  const start0 = Number(params.get("start") ?? Date.now());
  const end0 = Number(params.get("end") ?? start0 + 30 * MIN);
  const allDay = params.get("allDay") === "1";
  const rule = params.get("rule");
  const [startMin, setStartMin] = useState(allDay ? 0 : minutesOf(start0));
  const [duration, setDuration] = useState(Math.max(0, Math.round((end0 - start0) / MIN)));
  const [rangeStart, setRangeStart] = useState(dayKey(start0));
  const [p, setP] = useState<RecPattern>(() => parseRecurrence(rule ? rule.split("\n") : null, new Date(start0)));
  const set = (x: Partial<RecPattern>) => setP((o) => ({ ...o, ...x }));

  useEffect(() => {
    setWindowTitle("Appointment Recurrence");
  }, []);

  const answered = useRef(false);
  const answer = async (payload: Record<string, unknown>) => {
    answered.current = true;
    await emit("cm-recurrence-result", { req, ...payload });
    closeCurrentWindow();
  };
  useEffect(() => {
    const un = getCurrentWebviewWindow().onCloseRequested(() => {
      if (!answered.current) emit("cm-recurrence-result", { req, action: "cancel" });
    });
    return () => {
      un.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const ok = () => {
    const day = parseDayKey(rangeStart);
    // the first occurrence must match the pattern: move the start to the first matching day
    const first = firstMatch(day, p);
    const start = allDay ? first.getTime() : atMinutes(first, startMin);
    const end = allDay ? plusDays(first, Math.max(1, Math.round(duration / 1440))).getTime() : start + duration * MIN;
    answer({ action: "ok", recurrence: [buildRRule(p, allDay)], start, end, allDay });
  };

  useHotkeys({ enter: ok, escape: () => answer({ action: "cancel" }) }, { allowInInputs: true });

  const num = (v: number, on: (n: number) => void, w = 36) => (
    <input className="cm-input cm-cal-num" style={{ width: w }} value={v} onChange={(e) => on(Math.max(1, Math.min(999, Number(e.target.value.replace(/\D/g, "")) || 1)))} />
  );
  const nthSel = (
    <>
      <select className="cm-input" value={p.nth} onChange={(e) => set({ nth: Number(e.target.value) })}>
        {NTH_OPTS.map((n) => (
          <option key={n} value={n}>
            {NTH_LABEL[n]}
          </option>
        ))}
      </select>
      <select className="cm-input" value={p.nthKind} onChange={(e) => set({ nthKind: e.target.value as NthKind })}>
        {KIND_OPTS.map((k) => (
          <option key={k} value={k}>
            {NTH_KIND_LABEL[k]}
          </option>
        ))}
      </select>
    </>
  );
  const monthSel = (
    <select className="cm-input" value={p.month} onChange={(e) => set({ month: Number(e.target.value) })}>
      {MONTHS.map((m, i) => (
        <option key={m} value={i + 1}>
          {m}
        </option>
      ))}
    </select>
  );

  return (
    <div className="cm-cal-recwin">
      <fieldset className="cm-fieldset">
        <legend>Appointment time</legend>
        <div className="cm-cal-rec-time">
          <span>Start:</span>
          <TimeField value={startMin} disabled={allDay} onChange={setStartMin} width={100} />
          <span>End:</span>
          <TimeField value={(startMin + duration) % 1440} disabled={allDay} onChange={(m) => setDuration((m - startMin + 1440) % 1440 || 1440)} width={100} />
          <span>Duration:</span>
          <select className="cm-input" value={duration} disabled={allDay} onChange={(e) => setDuration(Number(e.target.value))}>
            {[...new Set([...DURATIONS, duration])]
              .sort((a, b) => a - b)
              .map((d) => (
                <option key={d} value={d}>
                  {fmtDuration(d)}
                </option>
              ))}
          </select>
        </div>
      </fieldset>
      <fieldset className="cm-fieldset">
        <legend>Recurrence pattern</legend>
        <div className="cm-cal-rec-pattern">
          <div className="cm-cal-rec-freq">
            {(["daily", "weekly", "monthly", "yearly"] as const).map((f) => (
              <label key={f}>
                <input type="radio" checked={p.freq === f} onChange={() => set({ freq: f })} /> {f[0].toUpperCase() + f.slice(1)}
              </label>
            ))}
          </div>
          <div className="cm-cal-rec-opts">
            {p.freq === "daily" && (
              <>
                <label>
                  <input type="radio" checked={!p.everyWeekday} onChange={() => set({ everyWeekday: false })} /> Every{" "}
                  {num(p.interval, (n) => set({ interval: n, everyWeekday: false }))} day(s)
                </label>
                <label>
                  <input type="radio" checked={p.everyWeekday} onChange={() => set({ everyWeekday: true, interval: 1 })} /> Every weekday
                </label>
              </>
            )}
            {p.freq === "weekly" && (
              <>
                <div>Recur every {num(p.interval, (n) => set({ interval: n }))} week(s) on:</div>
                <div className="cm-cal-rec-days">
                  {DAY_NAMES.map((d, i) => (
                    <label key={d}>
                      <input
                        type="checkbox"
                        checked={p.weekDays.includes(i)}
                        onChange={(e) => set({ weekDays: e.target.checked ? [...p.weekDays, i] : p.weekDays.filter((x) => x !== i) })}
                      />{" "}
                      {d}
                    </label>
                  ))}
                </div>
              </>
            )}
            {p.freq === "monthly" && (
              <>
                <label>
                  <input type="radio" checked={p.mode === "date"} onChange={() => set({ mode: "date" })} /> Day{" "}
                  <input
                    className="cm-input cm-cal-num"
                    value={p.monthDay}
                    onChange={(e) => set({ mode: "date", monthDay: Math.max(1, Math.min(31, Number(e.target.value.replace(/\D/g, "")) || 1)) })}
                  />{" "}
                  of every {num(p.interval, (n) => set({ interval: n }))} month(s)
                </label>
                <label>
                  <input type="radio" checked={p.mode === "nth"} onChange={() => set({ mode: "nth" })} /> The {nthSel} of every {num(p.interval, (n) => set({ interval: n }))}{" "}
                  month(s)
                </label>
              </>
            )}
            {p.freq === "yearly" && (
              <>
                <div>Recur every {num(p.interval, (n) => set({ interval: n }))} year(s)</div>
                <label>
                  <input type="radio" checked={p.mode === "date"} onChange={() => set({ mode: "date" })} /> On: {monthSel}{" "}
                  <input
                    className="cm-input cm-cal-num"
                    value={p.monthDay}
                    onChange={(e) => set({ mode: "date", monthDay: Math.max(1, Math.min(31, Number(e.target.value.replace(/\D/g, "")) || 1)) })}
                  />
                </label>
                <label>
                  <input type="radio" checked={p.mode === "nth"} onChange={() => set({ mode: "nth" })} /> On the: {nthSel} of {monthSel}
                </label>
              </>
            )}
          </div>
        </div>
      </fieldset>
      <fieldset className="cm-fieldset">
        <legend>Range of recurrence</legend>
        <div className="cm-cal-rec-range">
          <div className="cm-cal-rec-range-start">
            <span>Start:</span>
            <DateField value={rangeStart} onChange={setRangeStart} />
          </div>
          <div className="cm-cal-rec-range-end">
            <label>
              <input type="radio" checked={p.end === "none"} onChange={() => set({ end: "none" })} /> No end date
            </label>
            <label>
              <input type="radio" checked={p.end === "count"} onChange={() => set({ end: "count" })} /> End after: {num(p.count, (n) => set({ count: n, end: "count" }), 42)}{" "}
              occurrences
            </label>
            <label>
              <input type="radio" checked={p.end === "until"} onChange={() => set({ end: "until" })} /> End by:{" "}
              <DateField value={p.until} onChange={(v) => set({ until: v, end: "until" })} />
            </label>
          </div>
        </div>
      </fieldset>
      <div className="cm-cal-rec-buttons">
        <button className="cm-button default" onClick={ok}>
          OK
        </button>
        <button className="cm-button" onClick={() => answer({ action: "cancel" })}>
          Cancel
        </button>
        <button className="cm-button" disabled={!rule} onClick={() => answer({ action: "remove" })}>
          Remove Recurrence
        </button>
      </div>
    </div>
  );
}

/** First day on or after `d` that the pattern produces (so the series starts on a real occurrence). */
function firstMatch(d: Date, p: RecPattern): Date {
  for (let i = 0; i < 366 * 2; i++) {
    const x = plusDays(d, i);
    if (matches(x, p)) return x;
  }
  return d;
}

function matches(x: Date, p: RecPattern): boolean {
  const dow = x.getDay();
  const nthOf = (pred: (y: Date) => boolean) => {
    const all: number[] = [];
    const m = x.getMonth();
    for (let day = 1; day <= 31; day++) {
      const y = new Date(x.getFullYear(), m, day);
      if (y.getMonth() !== m) break;
      if (pred(y)) all.push(day);
    }
    const idx = p.nth === -1 ? all.length - 1 : p.nth - 1;
    return all[idx] === x.getDate();
  };
  const nthMatch = () => {
    switch (p.nthKind) {
      case "day":
        return nthOf(() => true);
      case "weekday":
        return nthOf((y) => y.getDay() >= 1 && y.getDay() <= 5);
      case "weekendday":
        return nthOf((y) => y.getDay() === 0 || y.getDay() === 6);
      default:
        return nthOf((y) => CODES[y.getDay()] === p.nthKind);
    }
  };
  switch (p.freq) {
    case "daily":
      return !p.everyWeekday || (dow >= 1 && dow <= 5);
    case "weekly":
      return (p.weekDays.length ? p.weekDays : [dow]).includes(dow);
    case "monthly":
      return p.mode === "date" ? x.getDate() === p.monthDay : nthMatch();
    case "yearly":
      return x.getMonth() + 1 === p.month && (p.mode === "date" ? x.getDate() === p.monthDay : nthMatch());
  }
}
