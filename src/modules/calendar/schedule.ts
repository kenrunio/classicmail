// The meeting form, and filling a new one from Schedule with AI (src/api/aiSchedule.ts).
import type { BusyStatus } from "../../api/calendar";
import type { Prepared, ScheduleBusy } from "../../api/aiSchedule";
import type { Addr } from "../contacts/SelectNames";
import { wallOf } from "./timeZones";

export interface Form {
  calendarId?: number;
  summary: string;
  location: string;
  description: string;
  allDay: boolean;
  startDate: string;
  startMin: number;
  /** for all-day items: the inclusive last day, as Outlook shows it */
  endDate: string;
  endMin: number;
  /** the To people, then the Cc ones (Google's optional attendees) */
  attendees: Attendee[];
  reminder: number;
  busyStatus: BusyStatus;
  isPrivate: boolean;
  importance: "low" | "normal" | "high";
  recurrence: string[] | null;
  meeting: boolean;
  categories: string[];
  /** the zone the start / end times are shown and typed in (Time Zones); default your own */
  timeZone: string;
}

export type Attendee = Addr & { optional?: boolean };

/** A new meeting form filled in from the prepared result; the base form's time stays when no time was found. */
export function prefillForm(base: Form, p: Prepared): Form {
  const f: Form = {
    ...base,
    calendarId: p.calendarId,
    summary: p.subject,
    location: p.location,
    description: p.notes,
    meeting: true,
    allDay: false,
    attendees: p.attendees.map((a) => (a.optional ? { name: a.name, email: a.email, optional: true } : { name: a.name, email: a.email })),
  };
  if (p.found && p.start != null && p.end != null) {
    const s = wallOf(p.start, p.timeZone);
    const e = wallOf(p.end, p.timeZone);
    Object.assign(f, { startDate: s.date, startMin: s.min, endDate: e.date, endMin: e.min, timeZone: p.timeZone });
  }
  return f;
}

/** The question when the calendar already has something at that time. */
export function clashText(c: ScheduleBusy[]): string {
  const one = (b: ScheduleBusy) => `“${b.subject}” (${b.when})`;
  if (c.length === 1) return `You already have ${one(c[0])} at that time. Set up this meeting anyway?`;
  return `You already have at that time:\n${c.map(one).join("\n")}\nSet up this meeting anyway?`;
}
