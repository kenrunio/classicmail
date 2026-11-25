// Schedule with AI (docs/superpowers/specs/2026-11-25-schedule-with-ai-design.md): a new meeting
// prepared from an email by the back end (src-tauri/src/ai/schedule).
import { rpc } from "./rpc";

export interface SchedulePerson {
  name: string;
  email: string;
  title?: string | null;
  company?: string | null;
  /** a Cc person: an optional attendee */
  optional: boolean;
}

/** A meeting on the calendar the new one goes into. */
export interface ScheduleBusy {
  eventId: number;
  subject: string;
  /** "09:30–10:00", in your time */
  when: string;
}

export interface Prepared {
  /** false: no agreed time was found (start / end are null) */
  found: boolean;
  /** the primary calendar of the account the email arrived in */
  calendarId: number;
  start: number | null;
  end: number | null;
  /** the zone the email's time was written in (IANA) */
  timeZone: string;
  subject: string;
  /** in order: company A–Z, then first name; you left out */
  attendees: SchedulePerson[];
  /** the call link chosen (main or backup), or "" */
  location: string;
  notes: string;
  /** meetings at that time with someone from the email on them */
  alreadyInvited: ScheduleBusy[];
  /** other busy meetings at that time */
  clashes: ScheduleBusy[];
  info: { icon: "info" | "warning"; text: string }[];
}

export const fromMessage = (messageId: number) => rpc<Prepared>("ai.schedule.fromMessage", { messageId });
