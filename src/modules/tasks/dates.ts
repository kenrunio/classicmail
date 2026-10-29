// Date logic shared by the Tasks module, the To-Do Bar and the task window: Outlook's due-date
// groups, Follow Up flag dates, display formats and lenient parsing of typed dates.
import { addDays, differenceInCalendarDays, startOfWeek } from "date-fns";
import { dayKey, fmtDate as fmtDay, fmtTime, parseDateText, parseDayKey } from "../calendar/dates";

// the day key, its parser and the clock format are the calendar's (one set for both modules)
export { dayKey, fmtTime, parseDayKey };

export type DueGroup = "today" | "tomorrow" | "thisWeek" | "nextWeek" | "later" | "noDate";

export const DUE_GROUPS: { id: DueGroup; label: string }[] = [
  { id: "today", label: "Today" },
  { id: "tomorrow", label: "Tomorrow" },
  { id: "thisWeek", label: "This Week" },
  { id: "nextWeek", label: "Next Week" },
  { id: "later", label: "Later" },
  { id: "noDate", label: "No Date" },
];

export type FlagKind = "today" | "tomorrow" | "thisWeek" | "nextWeek" | "noDate";

/** Week starts on Sunday (Outlook's default for Israel and the US). */
export const WEEK_STARTS_ON = 0 as const;

export function todayKey(now = new Date()): string {
  return dayKey(now);
}

/** Outlook's "Arranged By: Due Date" group. Overdue items sit in Today (shown in red). */
export function dueGroup(due: string | null | undefined, now = new Date()): DueGroup {
  if (!due) return "noDate";
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const d = parseDayKey(due);
  const diff = differenceInCalendarDays(d, today);
  if (diff <= 0) return "today";
  if (diff === 1) return "tomorrow";
  const weekStart = startOfWeek(today, { weekStartsOn: WEEK_STARTS_ON });
  const nextWeek = addDays(weekStart, 7);
  const weekAfter = addDays(weekStart, 14);
  if (d < nextWeek) return "thisWeek";
  if (d < weekAfter) return "nextWeek";
  return "later";
}

export function isOverdue(due: string | null | undefined, complete: boolean, now = new Date()): boolean {
  return !!due && !complete && due < todayKey(now);
}

/** Start / due dates Outlook sets for a Follow Up flag (work week Mon–Fri). */
export function flagDates(kind: FlagKind, now = new Date()): { start: string | null; due: string | null } {
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  switch (kind) {
    case "today":
      return { start: dayKey(today), due: dayKey(today) };
    case "tomorrow": {
      const t = addDays(today, 1);
      return { start: dayKey(t), due: dayKey(t) };
    }
    case "thisWeek": {
      const friday = addDays(startOfWeek(today, { weekStartsOn: 0 }), 5);
      const due = friday < today ? today : friday;
      const start = addDays(today, 2) > due ? due : addDays(today, 2);
      return { start: dayKey(start), due: dayKey(due) };
    }
    case "nextWeek": {
      const sunday = addDays(startOfWeek(today, { weekStartsOn: 0 }), 7);
      return { start: dayKey(addDays(sunday, 1)), due: dayKey(addDays(sunday, 5)) };
    }
    case "noDate":
      return { start: null, due: null };
  }
}

/** Flag shade for a due date: darker the sooner it's due (Outlook 2007 Quick Flags). */
export function flagColor(due: string | null | undefined, now = new Date()): string {
  switch (dueGroup(due, now)) {
    case "today":
      return "#c4302b";
    case "tomorrow":
      return "#d8534a";
    case "thisWeek":
      return "#e2766c";
    case "nextWeek":
      return "#ea9990";
    case "later":
      return "#efb0a9";
    case "noDate":
      return "#f3c6c1";
  }
}

// ---- display ----

/** "Sun 10/11/2026" (numeric part in the system's order), or "None". */
export function fmtDate(key: string | null | undefined, none = "None"): string {
  return key ? fmtDay(parseDayKey(key)) : none;
}

/**
 * Lenient date entry like Outlook's date boxes (the calendar's parser: "today", "fri", "+3",
 * dates in the system order...), plus "None". Returns `null` for None, `undefined` when unparseable.
 */
export function parseDateInput(input: string, now = new Date()): string | null | undefined {
  const s = input.trim().toLowerCase();
  if (!s || s === "none") return null;
  const d = parseDateText(s, now);
  return d ? dayKey(d) : undefined;
}

/** Info-bar text for the task window ("Due today.", "Overdue by 3 days." ...). */
export function dueInfo(due: string | null, complete: boolean, completedAt: number | null, now = new Date()): string | null {
  if (complete) return completedAt ? `Completed on ${fmtDate(dayKey(new Date(completedAt)))}.` : "Completed.";
  if (!due) return null;
  const diff = differenceInCalendarDays(parseDayKey(due), new Date(now.getFullYear(), now.getMonth(), now.getDate()));
  if (diff < 0) return diff === -1 ? "Due yesterday." : `Overdue by ${-diff} days.`;
  if (diff === 0) return "Due today.";
  if (diff === 1) return "Due tomorrow.";
  return `Due in ${diff} days.`;
}
