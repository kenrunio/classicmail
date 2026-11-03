// A meeting request's "View in Calendar": the meeting's day from the calendar, in place of the
// message text, so you see what's around it while you answer.
import { useEffect, useMemo } from "react";
import { useRpc } from "../../api/query";
import type { CalEvent, CalendarInfo } from "../../api/calendar";
import { DayWeekGrid } from "./DayWeekGrid";
import { CalDialogHost } from "./dialogs";
import { ColorCategoriesHost } from "./categories";
import { plusDays } from "./dates";
import { useCal, useCalSettings } from "./state";
import "./calendar.css";

/** The local day a meeting falls on (an all-day one is a UTC date). */
export function inviteDay(start: number, allDay?: boolean): Date {
  const d = new Date(start);
  return allDay ? new Date(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()) : new Date(d.getFullYear(), d.getMonth(), d.getDate());
}

export function InviteDay({ start, allDay, eventId }: { start: number; allDay?: boolean; eventId?: number }) {
  const settings = useCalSettings();
  const day = useMemo(() => inviteDay(start, allDay), [start, allDay]);
  const from = day.getTime();
  const to = plusDays(day, 1).getTime();
  const cals = useRpc<CalendarInfo[]>(["calendar", "calendars"], "calendar.listCalendars");
  const selected = useMemo(() => (cals.data ?? []).filter((c) => c.selected), [cals.data]);
  // the Calendar module's own key for one overlaid group, so the two share what's fetched
  const evs = useRpc<CalEvent[]>(["calendar", "events", from, to, "all"], "calendar.listEvents", { start: from, end: to }, { placeholderData: (p: CalEvent[] | undefined) => p });
  const groups = useMemo(() => [{ cals: selected, events: evs.data ?? [] }], [selected, evs.data]);
  // the meeting itself selected
  useEffect(() => {
    if (eventId !== undefined) useCal.getState().set({ selectedId: eventId, selection: null });
  }, [eventId]);
  return (
    <div className="cm-cal-view cm-invite-day">
      <div className="cm-cal-body">
        <DayWeekGrid days={[day]} groups={groups} settings={settings} showTabs={false} allCalendars={cals.data ?? []} onOverlay={() => {}} scrollTo={allDay ? undefined : start} />
      </div>
      <CalDialogHost />
      <ColorCategoriesHost />
    </div>
  );
}
