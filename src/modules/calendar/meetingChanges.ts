// What changed on a meeting whose invitations already went out, for the question of who to tell
// when it's saved (AppointmentWindow) — as Outlook asks.

export interface MeetingShape {
  summary: string;
  location: string;
  description: string;
  allDay: boolean;
  startDate: string;
  startMin: number;
  endDate: string;
  endMin: number;
  timeZone: string;
  recurrence: string[] | null;
  attendees: { email: string }[];
}

export interface MeetingChanges {
  /** something the attendees see: subject, location, notes, times or recurrence */
  content: boolean;
  /** addresses (lower case) added to the meeting / taken off it */
  added: string[];
  removed: string[];
}

export function meetingChanges(before: MeetingShape, after: MeetingShape): MeetingChanges {
  const content =
    before.summary !== after.summary ||
    before.location !== after.location ||
    before.description.replace(/\r\n/g, "\n") !== after.description.replace(/\r\n/g, "\n") ||
    before.allDay !== after.allDay ||
    before.startDate !== after.startDate ||
    before.endDate !== after.endDate ||
    (!after.allDay && (before.startMin !== after.startMin || before.endMin !== after.endMin || before.timeZone !== after.timeZone)) ||
    JSON.stringify(before.recurrence ?? null) !== JSON.stringify(after.recurrence ?? null);
  const emails = (s: MeetingShape) => [...new Set(s.attendees.map((a) => a.email.trim().toLowerCase()).filter(Boolean))];
  const was = emails(before);
  const now = emails(after);
  return { content, added: now.filter((e) => !was.includes(e)), removed: was.filter((e) => !now.includes(e)) };
}

/** Who hears about a save: everyone (Google's update), only the people added or removed, or nobody. */
export type Tell = "all" | "changes" | "none";

/** The choices to offer for these changes, the first the default; none when there's nothing to tell. */
export function tellChoices(c: MeetingChanges): { value: Tell; label: string }[] {
  const people = c.added.length + c.removed.length > 0;
  const dont = { value: "none" as const, label: "Save changes but don't send." };
  const changes = { value: "changes" as const, label: "Save changes and send update only to added or removed attendees." };
  if (c.content && people) return [{ value: "all", label: "Save changes and send update to all attendees." }, changes, dont];
  if (c.content) return [{ value: "all", label: "Save changes and send update." }, dont];
  if (people) return [changes, dont];
  return [];
}
