// Outlook's "Appointment Recurrence" model ↔ RFC 5545 RRULE, plus the "Occurs every…" text.
import { format } from "date-fns";
import { fmtTime, parseDayKey, SHORT_DATE, dayKey } from "./dates";

export type Freq = "daily" | "weekly" | "monthly" | "yearly";
/** BYDAY-ish selector for "The [second] [Tuesday]": a weekday code or day / weekday / weekend day */
export type NthKind = "SU" | "MO" | "TU" | "WE" | "TH" | "FR" | "SA" | "day" | "weekday" | "weekendday";

export interface RecPattern {
  freq: Freq;
  interval: number;
  /** daily: "Every weekday" */
  everyWeekday: boolean;
  /** weekly: 0 = Sunday … 6 = Saturday */
  weekDays: number[];
  /** monthly / yearly: "Day 12" vs "The second Tuesday" */
  mode: "date" | "nth";
  monthDay: number;
  /** 1…4, -1 = last */
  nth: number;
  nthKind: NthKind;
  /** yearly: 1…12 */
  month: number;
  end: "none" | "count" | "until";
  count: number;
  /** yyyy-MM-dd */
  until: string;
}

export const CODES = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"] as const;
export const DAY_NAMES = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
export const NTH_NAMES: Record<number, string> = { 1: "first", 2: "second", 3: "third", 4: "fourth", [-1]: "last" };
export const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
export const NTH_KIND_LABEL: Record<NthKind, string> = {
  day: "day",
  weekday: "weekday",
  weekendday: "weekend day",
  SU: "Sunday",
  MO: "Monday",
  TU: "Tuesday",
  WE: "Wednesday",
  TH: "Thursday",
  FR: "Friday",
  SA: "Saturday",
};

/** Defaults for a start date (what Outlook pre-fills). */
export function defaultPattern(start: Date): RecPattern {
  const nth = Math.ceil(start.getDate() / 7);
  const until = new Date(start);
  until.setMonth(until.getMonth() + 3);
  return {
    freq: "weekly",
    interval: 1,
    everyWeekday: false,
    weekDays: [start.getDay()],
    mode: "date",
    monthDay: start.getDate(),
    nth: nth > 4 ? -1 : nth,
    nthKind: CODES[start.getDay()],
    month: start.getMonth() + 1,
    end: "none",
    count: 10,
    until: dayKey(until),
  };
}

function parseUntil(v: string): string {
  if (/^\d{8}$/.test(v)) return `${v.slice(0, 4)}-${v.slice(4, 6)}-${v.slice(6, 8)}`;
  const m = v.match(/^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})(Z?)$/);
  if (!m) return v;
  const d = m[7] ? new Date(Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6])) : new Date(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]);
  return dayKey(d);
}

/** Parse the RRULE line of a recurrence array into the dialog model. */
export function parseRecurrence(lines: string[] | null | undefined, start: Date): RecPattern {
  const p = defaultPattern(start);
  const line = lines?.find((l) => l.startsWith("RRULE"));
  if (!line) return p;
  const parts = Object.fromEntries(
    line
      .replace(/^RRULE:/, "")
      .split(";")
      .map((kv) => kv.split("=") as [string, string]),
  );
  p.freq = ({ DAILY: "daily", WEEKLY: "weekly", MONTHLY: "monthly", YEARLY: "yearly" } as Record<string, Freq>)[parts.FREQ] ?? "weekly";
  p.interval = Math.max(1, Number(parts.INTERVAL ?? 1));
  const byday: string[] = parts.BYDAY ? parts.BYDAY.split(",") : [];
  const setpos = parts.BYSETPOS ? Number(parts.BYSETPOS) : undefined;
  if (parts.COUNT) {
    p.end = "count";
    p.count = Number(parts.COUNT);
  } else if (parts.UNTIL) {
    p.end = "until";
    p.until = parseUntil(parts.UNTIL);
  }
  const plain = byday.map((d) => d.replace(/^[+-]?\d+/, ""));
  const isWeekdays = plain.length === 5 && ["MO", "TU", "WE", "TH", "FR"].every((c) => plain.includes(c));
  const isWeekend = plain.length === 2 && plain.includes("SA") && plain.includes("SU");
  if (p.freq === "daily") {
    p.everyWeekday = isWeekdays;
  } else if (p.freq === "weekly") {
    if (isWeekdays && p.interval === 1) {
      // Outlook shows FREQ=WEEKLY;BYDAY=MO..FR as "Daily: every weekday"
      p.freq = "daily";
      p.everyWeekday = true;
    }
    if (byday.length) p.weekDays = plain.map((c) => CODES.indexOf(c as (typeof CODES)[number])).filter((i) => i >= 0);
  }
  if (p.freq === "monthly" || p.freq === "yearly") {
    if (parts.BYMONTH) p.month = Number(parts.BYMONTH.split(",")[0]);
    if (parts.BYMONTHDAY) {
      const md = Number(parts.BYMONTHDAY.split(",")[0]);
      if (md === -1) {
        p.mode = "nth";
        p.nth = -1;
        p.nthKind = "day";
      } else {
        p.mode = "date";
        p.monthDay = md;
      }
    } else if (byday.length) {
      p.mode = "nth";
      if (setpos !== undefined) {
        p.nth = setpos;
        p.nthKind = isWeekdays ? "weekday" : isWeekend ? "weekendday" : plain.length === 7 ? "day" : (plain[0] as NthKind);
      } else {
        const m = byday[0].match(/^([+-]?\d+)?([A-Z]{2})$/);
        p.nth = m?.[1] ? Number(m[1]) : 1;
        p.nthKind = (m?.[2] ?? "MO") as NthKind;
      }
    }
  }
  return p;
}

function untilValue(p: RecPattern, allDay: boolean): string {
  const d = parseDayKey(p.until);
  if (allDay) return format(d, "yyyyMMdd");
  const end = new Date(d.getFullYear(), d.getMonth(), d.getDate(), 23, 59, 59);
  return end
    .toISOString()
    .replace(/[-:]/g, "")
    .replace(/\.\d{3}/, "");
}

function nthParts(p: RecPattern): string[] {
  switch (p.nthKind) {
    case "day":
      return p.nth === -1 ? ["BYMONTHDAY=-1"] : [`BYMONTHDAY=${p.nth}`];
    case "weekday":
      return ["BYDAY=MO,TU,WE,TH,FR", `BYSETPOS=${p.nth}`];
    case "weekendday":
      return ["BYDAY=SA,SU", `BYSETPOS=${p.nth}`];
    default:
      return [`BYDAY=${p.nth}${p.nthKind}`];
  }
}

/** Build the RRULE line. */
export function buildRRule(p: RecPattern, allDay: boolean): string {
  const parts: string[] = [];
  const interval = Math.max(1, Math.floor(p.interval || 1));
  switch (p.freq) {
    case "daily":
      if (p.everyWeekday) parts.push("FREQ=WEEKLY", "BYDAY=MO,TU,WE,TH,FR");
      else parts.push("FREQ=DAILY", ...(interval > 1 ? [`INTERVAL=${interval}`] : []));
      break;
    case "weekly": {
      const days = [...new Set(p.weekDays)].sort();
      parts.push("FREQ=WEEKLY", ...(interval > 1 ? [`INTERVAL=${interval}`] : []), `BYDAY=${(days.length ? days : [0]).map((d) => CODES[d]).join(",")}`);
      break;
    }
    case "monthly":
      parts.push("FREQ=MONTHLY", ...(interval > 1 ? [`INTERVAL=${interval}`] : []));
      parts.push(...(p.mode === "date" ? [`BYMONTHDAY=${p.monthDay}`] : nthParts(p)));
      break;
    case "yearly":
      parts.push("FREQ=YEARLY", ...(interval > 1 ? [`INTERVAL=${interval}`] : []), `BYMONTH=${p.month}`);
      parts.push(...(p.mode === "date" ? [`BYMONTHDAY=${p.monthDay}`] : nthParts(p)));
      break;
  }
  if (p.end === "count") parts.push(`COUNT=${Math.max(1, Math.floor(p.count || 1))}`);
  if (p.end === "until") parts.push(`UNTIL=${untilValue(p, allDay)}`);
  return `RRULE:${parts.join(";")}`;
}

function listJoin(items: string[]): string {
  if (items.length <= 1) return items.join("");
  return `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

/** Outlook's "Occurs every Monday effective 10/12/2026 until 12/31/2026 from 9:00 AM to 10:00 AM." */
export function describeRecurrence(lines: string[] | null | undefined, start: number, end: number, allDay: boolean): string {
  if (!lines?.some((l) => l.startsWith("RRULE"))) return "";
  const s = new Date(start);
  const p = parseRecurrence(lines, s);
  let what: string;
  const every = (unit: string) => (p.interval > 1 ? `every ${p.interval} ${unit}s` : `every ${unit}`);
  const nthText = () => `the ${NTH_NAMES[p.nth] ?? p.nth} ${NTH_KIND_LABEL[p.nthKind]}`;
  switch (p.freq) {
    case "daily":
      what = p.everyWeekday ? "every weekday" : every("day");
      break;
    case "weekly":
      {
        const days = listJoin(
          p.weekDays
            .slice()
            .sort()
            .map((d) => DAY_NAMES[d]),
        );
        what = p.interval > 1 ? `every ${p.interval} weeks on ${days}` : `every ${days}`;
      }
      break;
    case "monthly":
      what =
        p.mode === "date" ? `day ${p.monthDay} of every ${p.interval} month${p.interval > 1 ? "s" : ""}` : `${nthText()} of every ${p.interval} month${p.interval > 1 ? "s" : ""}`;
      break;
    default:
      what = p.mode === "date" ? `every ${MONTHS[p.month - 1]} ${p.monthDay}` : `${nthText()} of ${MONTHS[p.month - 1]}`;
  }
  let text = `Occurs ${what} effective ${format(s, SHORT_DATE)}`;
  if (p.end === "until") text += ` until ${format(parseDayKey(p.until), SHORT_DATE)}`;
  if (p.end === "count") text += ` for ${p.count} occurrence${p.count === 1 ? "" : "s"}`;
  if (!allDay) text += ` from ${fmtTime(start)} to ${fmtTime(end)}`;
  return `${text}.`;
}
