// Calendar DTOs + rpc helpers (back end: src-tauri/src/calendar).
import { rpc } from "./rpc";
import { undoable } from "./undo";

/** With an undo `label` the action is recorded in the Undo history (Ctrl+Z). */
const via = <T>(label: string | undefined, method: string, params: object) => (label ? undoable<T>(label, method, params) : rpc<T>(method, params));

export type Response = "accepted" | "tentative" | "declined" | "needsAction";
export type BusyStatus = "busy" | "free" | "tentative" | "oof";
export type Scope = "occurrence" | "series";

/** The cross-domain CalEvent (docs/contracts.md) + a few calendar-module extras. */
export interface CalEvent {
  id: number;
  accountId: number;
  calendarId: number;
  calendarColor: string;
  summary: string;
  /** you renamed this invitation on your calendar: the organizer's subject (`summary` is yours) */
  originalSummary?: string;
  location: string;
  start: number;
  end: number;
  allDay: boolean;
  startDate?: string;
  endDate?: string;
  isRecurring: boolean;
  isMeeting: boolean;
  myResponse?: Response;
  busyStatus: BusyStatus;
  isPrivate: boolean;
  hasReminder: boolean;
  // extras
  isException: boolean;
  importance: "low" | "normal" | "high";
  isOrganizer: boolean;
  readOnly: boolean;
  /** Outlook colour categories (names) */
  categories: string[];
  /** Google event colour ("1".."11") */
  colorId?: string;
  /** display de-duplication: other calendars this item also appears in */
  alsoIn?: string[];
}

export interface CalendarInfo {
  id: number;
  accountId: number;
  accountEmail: string;
  accountName: string;
  name: string;
  color: string;
  selected: boolean;
  accessRole: string;
  isPrimary: boolean;
  canWrite: boolean;
  timeZone: string;
  /** My Calendars / Shared Calendars / Other Calendars */
  group: "mine" | "shared" | "other";
}

export interface AttendeeInfo {
  email: string;
  name: string;
  response: Response;
  organizer: boolean;
  optional: boolean;
  isSelf: boolean;
  comment?: string;
}

export interface EventDetail extends CalEvent {
  description: string;
  /** Zoom / Meet link, dial-ins and notes from Google's conference data (read-only) */
  conference?: string;
  attendees: AttendeeInfo[];
  organizerEmail?: string | null;
  organizerName?: string | null;
  reminderMinutes?: number | null;
  visibility: string;
  htmlLink?: string | null;
  timeZone?: string | null;
  calendarName: string;
  accountEmail: string;
  seriesId?: number | null;
  recurrence?: string[] | null;
  seriesStart?: number | null;
  seriesEnd?: number | null;
  originalStart?: number | null;
  isMaster: boolean;
  pending: boolean;
}

export interface EventInput {
  calendarId?: number;
  summary?: string;
  location?: string;
  description?: string;
  start?: number;
  end?: number;
  allDay?: boolean;
  startDate?: string;
  endDate?: string;
  attendees?: { email: string; name?: string; optional?: boolean }[];
  /** minutes before start; -1 = none */
  reminderMinutes?: number;
  busyStatus?: BusyStatus;
  isPrivate?: boolean;
  importance?: "low" | "normal" | "high";
  /** RRULE lines; [] removes recurrence */
  recurrence?: string[];
  sendUpdates?: boolean;
  categories?: string[];
  /** the time zone start / end were set in (IANA); default the calendar's */
  timeZone?: string;
}

export interface Category {
  name: string;
  /** Outlook colour preset 1..25, 0 = none */
  color: number;
  /** "F2".."F12" (with Ctrl) or "" */
  shortcut: string;
  /** colour items whose subject contains the name (or a match word); default true */
  autoMatch?: boolean;
  /** extra words that also match, e.g. "ACMX" for "Acme" */
  matchWords?: string[];
}

export interface CategoryList {
  categories: Category[];
  /** [preset, name, hex] */
  palette: [number, string, string][];
  /** [colorId, name, hex] */
  googleColors: [string, string, string][];
}

export interface DupItem {
  id: number;
  calendarId: number;
  summary: string;
  location: string;
  start: number;
  end: number;
  allDay: boolean;
  isSeries: boolean;
  hasAttendees: boolean;
  hasNotes: boolean;
  organizerSelf: boolean;
  organizer?: string | null;
  created?: number | null;
  suggestDelete: boolean;
  warn: boolean;
  readOnly: boolean;
}

export interface DupGroup {
  calendarId: number;
  summary: string;
  start: number;
  end: number;
  items: DupItem[];
}

export interface ReminderItem {
  /** "<eventId>:<start>" or "task:<taskId>:<reminderAt>" */
  key: string;
  kind: "event" | "task";
  eventId: number;
  summary: string;
  location: string;
  start: number;
  end: number;
  allDay: boolean;
  reminderAt: number;
  isMeeting: boolean;
  calendarColor: string;
}

export const cal = {
  listCalendars: () => rpc<CalendarInfo[]>("calendar.listCalendars"),
  setSelected: (id: number, selected: boolean) => rpc("calendar.setCalendarSelected", { id, selected }),
  listEvents: (start: number, end: number, calendarIds?: number[], dedupe?: boolean) => rpc<CalEvent[]>("calendar.listEvents", { start, end, calendarIds, dedupe }),
  search: (query: string) => rpc<CalEvent[]>("calendar.search", { query }),
  getEvent: (id: number) => rpc<EventDetail>("calendar.getEvent", { id }),
  create: (p: EventInput, label?: string) => via<EventDetail>(label, "calendar.createEvent", p),
  update: (id: number, p: EventInput, scope: Scope = "occurrence", label?: string) => via<EventDetail>(label, "calendar.updateEvent", { id, scope, ...p }),
  copy: (id: number, start: number, end: number, calendarId?: number, label?: string) => via<EventDetail>(label, "calendar.copyEvent", { id, start, end, calendarId }),
  /** move to another calendar (same account: Google events.move; other account: copy + delete), optionally retimed */
  moveTo: (id: number, calendarId: number, p: EventInput = {}, label?: string) => via<EventDetail>(label, "calendar.moveToCalendar", { ...p, id, calendarId }),
  listCategories: () => rpc<CategoryList>("calendar.listCategories"),
  saveCategories: (categories: Category[], renames: [string, string][] = []) => rpc<number>("calendar.saveCategories", { categories, renames }),
  setCategories: (ids: number[], categories: string[], label?: string) => via(label, "calendar.setEventCategories", { ids, categories }),
  /** your own subject for someone else's meeting, on your calendar only ("" = the organizer's) */
  setOwnSummary: (id: number, summary: string, label?: string) => via<EventDetail>(label, "calendar.setOwnSummary", { id, summary }),
  findDuplicates: (calendarIds?: number[]) => rpc<{ groups: DupGroup[]; calendars: Record<string, string>; scanned: number }>("calendar.findDuplicates", { calendarIds }),
  deleteEvents: (ids: number[]) => rpc<{ deleted: number; failed: string[] }>("calendar.deleteEvents", { ids }),
  remove: (id: number, scope: Scope = "occurrence", sendUpdates?: boolean, label?: string) => via(label, "calendar.deleteEvent", { id, scope, sendUpdates }),
  respond: (id: number, response: Exclude<Response, "needsAction">, opts: { scope?: Scope; comment?: string; sendResponse?: boolean } = {}) =>
    rpc("calendar.respond", { id, response, ...opts }),
  /** a meeting request e-mail to these attendees only (Responses › Resend Invitation): the addresses
   *  it went to, and why it stopped when a send failed part way. `series`: how a recurring meeting
   *  recurs ("Occurs every Tuesday…"), for the e-mail's subject and When line */
  resendInvite: (id: number, emails: string[], series?: string) => rpc<{ sent: string[]; error: string | null }>("calendar.resendInvite", { id, emails, series }),
  dueReminders: () => rpc<ReminderItem[]>("calendar.dueReminders"),
  dismiss: (keys: string[]) => rpc("calendar.dismissReminder", { keys }),
  snooze: (keys: string[], minutes: number) => rpc("calendar.snoozeReminder", { keys, minutes }),
};
