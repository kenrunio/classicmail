// Date helpers for the calendar views (all in the system's local time zone).
import { addDays, addMonths, differenceInCalendarDays, format, isValid, parse, startOfMonth, startOfWeek } from "date-fns";
import { dayKey } from "../../ui/datenav";

export const MIN = 60_000;
export const HOUR = 60 * MIN;
export const DAY = 24 * HOUR;

export function dayStart(d: Date | number): Date {
  const x = new Date(d);
  return new Date(x.getFullYear(), x.getMonth(), x.getDate());
}

/** Local midnight `n` days after `d` (DST-safe, unlike adding 24h). */
export function plusDays(d: Date | number, n: number): Date {
  return addDays(dayStart(d), n);
}

export function sameDay(a: Date | number, b: Date | number) {
  return dayStart(a).getTime() === dayStart(b).getTime();
}

// the one "yyyy-MM-dd" day key (the Date Navigator's), shared with the Tasks module
export { dayKey };

/** Local midnight of a "yyyy-MM-dd" day key. */
export function parseDayKey(s: string): Date {
  const [y, m, d] = s.split("-").map(Number);
  return new Date(y, m - 1, d);
}

/**
 * Whole days covering [start, end): the first day's midnight to the midnight after the last day,
 * at least one day. Counted by the calendar, not in 24 h steps (DST days have 23 or 25 hours).
 */
export function allDayRange(start: number, end: number): { start: number; end: number } {
  const s = dayStart(start).getTime();
  return { start: s, end: Math.max(plusDays(s, 1).getTime(), plusDays(end - 1, 1).getTime()) };
}

/** The last day ("yyyy-MM-dd") of an item moved to start on `newStart`, keeping its number of days. */
export function sameLengthEnd(start: string, end: string, newStart: string): string {
  return dayKey(addDays(parseDayKey(newStart), differenceInCalendarDays(parseDayKey(end), parseDayKey(start))));
}

export type ViewKind = "day" | "workweek" | "week" | "month";

export interface Range {
  /** the days shown (day/week views), each a local midnight */
  days: Date[];
  start: number;
  end: number;
}

/** Days shown by a view anchored on `anchor`. */
export function viewDays(view: ViewKind, anchor: Date, opts: { weekStartsOn: number; workDays: number[]; customDays?: number }): Range {
  const a = dayStart(anchor);
  let days: Date[];
  if (opts.customDays && opts.customDays > 0 && view === "day") {
    days = Array.from({ length: opts.customDays }, (_, i) => plusDays(a, i));
  } else if (view === "day") {
    days = [a];
  } else if (view === "week" || view === "workweek") {
    const ws = startOfWeek(a, { weekStartsOn: opts.weekStartsOn as 0 | 1 | 6 });
    days = Array.from({ length: 7 }, (_, i) => plusDays(ws, i));
    if (view === "workweek") {
      const wd = days.filter((d) => opts.workDays.includes(d.getDay()));
      if (wd.length) days = wd;
    }
  } else {
    const ms = startOfMonth(a);
    const gs = startOfWeek(ms, { weekStartsOn: opts.weekStartsOn as 0 | 1 | 6 });
    const nextMonth = addMonths(ms, 1);
    const weeks = Math.ceil((dayStart(nextMonth).getTime() - gs.getTime()) / (7 * DAY) - 0.01);
    days = Array.from({ length: Math.max(5, weeks) * 7 }, (_, i) => plusDays(gs, i));
  }
  return { days, start: days[0].getTime(), end: plusDays(days[days.length - 1], 1).getTime() };
}

// ---- locale-aware date formats (Windows' regional format, e.g. en-IL → 13/10/2026) ---------------

/** "dd/MM/yyyy", "M/d/yyyy", "dd.MM.yyyy"… derived from the system locale, as a date-fns pattern. */
export function shortDatePattern(locale?: string): string {
  try {
    const parts = new Intl.DateTimeFormat(locale, { day: "numeric", month: "numeric", year: "numeric" }).formatToParts(new Date(2026, 0, 5));
    let out = "";
    for (const p of parts) {
      if (p.type === "day") out += p.value.length === 2 ? "dd" : "d";
      else if (p.type === "month") out += p.value.length === 2 ? "MM" : "M";
      else if (p.type === "year") out += p.value.length === 2 ? "yy" : "yyyy";
      else if (p.type === "literal") out += p.value.replace(/[a-zA-Z']/g, "").replace(/‏|‎/g, "");
    }
    return /d/.test(out) && /M/.test(out) && /y/.test(out) ? out : "M/d/yyyy";
  } catch {
    return "M/d/yyyy";
  }
}

export const SHORT_DATE = shortDatePattern();
/** day before month (Israel, UK…) */
export const DAY_FIRST = SHORT_DATE.indexOf("d") < SHORT_DATE.indexOf("M");

const longFmt = new Intl.DateTimeFormat(undefined, { weekday: "long", day: "numeric", month: "long", year: "numeric" });
/** "Sunday, 11 October 2026" / "Sunday, October 11, 2026" */
export function fmtLongDate(d: number | Date): string {
  return longFmt.format(d);
}

/** Outlook-style range title: "Monday, 12 October 2026", "11 - 15 October 2026", "October 2026" (or US order). */
export function rangeTitle(view: ViewKind, days: Date[], anchor: Date): string {
  if (view === "month") return format(anchor, "MMMM yyyy");
  const a = days[0];
  const b = days[days.length - 1];
  if (days.length === 1) return fmtLongDate(a);
  if (DAY_FIRST) {
    if (a.getFullYear() !== b.getFullYear()) return `${format(a, "d MMMM yyyy")} - ${format(b, "d MMMM yyyy")}`;
    if (a.getMonth() !== b.getMonth()) return `${format(a, "d MMMM")} - ${format(b, "d MMMM yyyy")}`;
    return `${format(a, "d")} - ${format(b, "d MMMM yyyy")}`;
  }
  if (a.getFullYear() !== b.getFullYear()) return `${format(a, "MMMM d, yyyy")} - ${format(b, "MMMM d, yyyy")}`;
  if (a.getMonth() !== b.getMonth()) return `${format(a, "MMMM d")} - ${format(b, "MMMM d, yyyy")}`;
  return `${format(a, "MMMM d")} - ${format(b, "d, yyyy")}`;
}

/** Does this machine use a 12-hour clock? */
export const uses12h = (() => {
  try {
    const hc = (new Intl.DateTimeFormat(undefined, { hour: "numeric" }).resolvedOptions() as { hourCycle?: string }).hourCycle;
    return hc === "h12" || hc === "h11";
  } catch {
    return true;
  }
})();

export function fmtTime(ms: number | Date): string {
  return format(ms, uses12h ? "h:mm a" : "HH:mm");
}

/** Compact month-view time: "9:30am" / "09:30". */
export function fmtShortTime(ms: number | Date): string {
  return uses12h ? format(ms, "h:mmaaa") : format(ms, "HH:mm");
}

export function fmtDate(ms: number | Date): string {
  return format(ms, `EEE ${SHORT_DATE}`);
}

/** "30 minutes", "1 hour", "1.5 hours", "2 days", "1 week" */
export function fmtDuration(mins: number): string {
  if (mins < 60) return `${mins} minute${mins === 1 ? "" : "s"}`;
  if (mins < 24 * 60) {
    const h = mins / 60;
    return `${Number.isInteger(h) ? h : h.toFixed(1)} hour${h === 1 ? "" : "s"}`;
  }
  if (mins % (7 * 24 * 60) === 0) {
    const w = mins / (7 * 24 * 60);
    return `${w} week${w === 1 ? "" : "s"}`;
  }
  const d = mins / (24 * 60);
  return `${Number.isInteger(d) ? d : d.toFixed(1)} day${d === 1 ? "" : "s"}`;
}

/** Reminder choices as in Outlook's drop-down. */
export const REMINDER_CHOICES = [0, 5, 10, 15, 30, 60, 120, 180, 240, 300, 360, 420, 480, 540, 600, 660, 720, 1080, 1440, 2880, 4320, 5760, 10080, 20160];

export function fmtReminder(m: number | null | undefined): string {
  if (m === null || m === undefined || m < 0) return "None";
  return fmtDuration(m);
}

/** Parse "9", "9:30", "930", "9:30 pm", "21:15", "noon" into minutes after midnight. */
export function parseTime(s: string): number | null {
  const t = s.trim().toLowerCase();
  if (!t) return null;
  if (t === "noon") return 12 * 60;
  if (t === "midnight") return 0;
  const m = t.match(/^(\d{1,2})(?::?(\d{2}))?\s*(a|am|p|pm)?$/);
  if (!m) return null;
  let h = Number(m[1]);
  const mi = m[2] ? Number(m[2]) : 0;
  if (m[3]) {
    const pm = m[3].startsWith("p");
    if (h === 12) h = pm ? 12 : 0;
    else if (pm) h += 12;
  }
  if (h > 23 || mi > 59) return null;
  return h * 60 + mi;
}

/** Minutes after local midnight of `ms`. */
export function minutesOf(ms: number): number {
  const d = new Date(ms);
  return d.getHours() * 60 + d.getMinutes();
}

/**
 * Wall-clock minutes after midnight of `day` for an instant (0 before the day, 1440 after it).
 * Unlike (ms - midnight) / 60000 this stays right on the DST days, which have 23 or 25 hours.
 */
export function wallMinutes(ms: number, day: Date | number): number {
  const ds = dayStart(day).getTime();
  if (ms <= ds) return 0;
  if (ms >= plusDays(ds, 1).getTime()) return 1440;
  const d = new Date(ms);
  return d.getHours() * 60 + d.getMinutes() + d.getSeconds() / 60;
}

export function atMinutes(day: Date | number, minutes: number): number {
  const d = dayStart(day);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate(), Math.floor(minutes / 60), minutes % 60).getTime();
}

/** Next half hour from now (default for new appointments). */
export function nextSlot(now = Date.now(), slot = 30): number {
  const d = new Date(now);
  d.setSeconds(0, 0);
  const m = d.getMinutes();
  d.setMinutes(m - (m % slot) + slot);
  return d.getTime();
}

// ---- typed dates -----------------------------------------------------------------------------

// the system's own order first; the other order only as a fallback for unambiguous input
const NUMERIC = DAY_FIRST
  ? ["d/M/yyyy", "d/M/yy", "d.M.yyyy", "d.M.yy", "d-M-yyyy", "d-M-yy", "d/M", "d.M", "d-M"]
  : ["M/d/yyyy", "M/d/yy", "M-d-yyyy", "M-d-yy", "d.M.yyyy", "d.M.yy", "M/d", "M-d"];
const DATE_FORMATS = [SHORT_DATE, ...NUMERIC, "yyyy-M-d", "yyyy/M/d", "yyyy.M.d", "MMM d yyyy", "MMMM d yyyy", "d MMM yyyy", "d MMMM yyyy", "MMM d", "d MMM"];
const WEEKDAYS = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];

/**
 * A typed date, as Outlook's date boxes take it: "today", "tomorrow", "yesterday", a weekday
 * ("fri" = the next Friday), "+3" / "3 days", or a date in the system's order (year optional,
 * a leading weekday ignored). Local midnight, or null when not understood. Used by the
 * appointment windows' date fields and the Tasks date boxes.
 */
export function parseDateText(input: string, now = new Date()): Date | null {
  // "Tue 13/10/2026": the weekday is dropped, so editing the date part alone can't contradict it
  const s = input
    .trim()
    .toLowerCase()
    .replace(/^(sun|mon|tue|wed|thu|fri|sat)[a-z]*,?\s+(?=\d)/, "");
  const today = dayStart(now);
  if (!s) return null;
  if (s === "today" || s === "now") return today;
  if (s === "tomorrow") return plusDays(today, 1);
  if (s === "yesterday") return plusDays(today, -1);
  const rel = s.match(/^\+?(\d+)\s*(d|day|days)?$/);
  if (rel && (s.startsWith("+") || rel[2])) return plusDays(today, Number(rel[1]));
  const wd = WEEKDAYS.findIndex((w) => s.length >= 3 && w.startsWith(s));
  if (wd >= 0) return plusDays(today, (wd - today.getDay() + 7) % 7 || 7);
  for (const f of DATE_FORMATS) {
    const d = parse(s, f, today);
    // date-fns' "yyyy" also takes "26" as the year 26: leave two-digit years to the "yy" formats
    if (isValid(d) && (d.getFullYear() >= 1000 || !f.includes("yyyy"))) return dayStart(d);
  }
  return null;
}
