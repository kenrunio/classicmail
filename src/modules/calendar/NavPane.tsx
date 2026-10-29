// Calendar Navigation Pane: Date Navigator (two months when there's room) + "My Calendars".
import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { addMonths, startOfMonth, startOfWeek } from "date-fns";
import { useRpc } from "../../api/query";
import { cal, type CalendarInfo, type CalEvent } from "../../api/calendar";
import { alertBox, DateNavigator, dayKey, errorBox } from "../../ui";
import { DAY, dayStart, plusDays, viewDays } from "./dates";
import { useCal, useCalSettings } from "./state";

/** "Calendar" / "Calendar - you@example.com" (several accounts) / the calendar's own name. */
export function calLabel(c: CalendarInfo, all: CalendarInfo[]) {
  if (!c.isPrimary) return c.name;
  const primaries = all.filter((x) => x.isPrimary);
  return primaries.length > 1 ? `Calendar - ${c.accountEmail}` : "Calendar";
}

const SECTIONS: { group: CalendarInfo["group"]; title: string }[] = [
  { group: "mine", title: "My Calendars" },
  { group: "shared", title: "Shared Calendars" },
  { group: "other", title: "Other Calendars" },
];

export function NavPane() {
  const st = useCal();
  const settings = useCalSettings();
  const wrapRef = useRef<HTMLDivElement>(null);
  const [months, setMonths] = useState(1);
  const [navMonth, setNavMonth] = useState(() => startOfMonth(st.anchor));
  const cals = useRpc<CalendarInfo[]>(["calendar", "calendars"], "calendar.listCalendars");

  // follow the view: keep the anchor's month visible in the navigator
  const anchorMonth = startOfMonth(st.anchor).getTime();
  const lastAnchor = useRef(anchorMonth);
  if (lastAnchor.current !== anchorMonth) {
    lastAnchor.current = anchorMonth;
    const first = navMonth.getTime();
    const last = addMonths(navMonth, months - 1).getTime();
    if (anchorMonth < first || anchorMonth > last) setNavMonth(new Date(anchorMonth));
  }

  useLayoutEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setMonths(el.clientHeight > 470 ? 2 : 1));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const navStart = startOfWeek(navMonth, { weekStartsOn: settings.weekStartsOn }).getTime();
  const navEnd = plusDays(addMonths(navMonth, months), 14).getTime();
  const navEvents = useRpc<CalEvent[]>(["calendar", "events", navStart, navEnd], "calendar.listEvents", { start: navStart, end: navEnd }, { placeholderData: (p) => p });
  const bold = useMemo(() => {
    const s = new Set<string>();
    for (const e of navEvents.data ?? []) {
      const endT = Math.min(Math.max(e.end, e.start + 1), navEnd);
      for (let d = dayStart(Math.max(e.start, navStart)); d.getTime() < endT; d = plusDays(d, 1)) s.add(dayKey(d));
    }
    return s;
  }, [navEvents.data, navStart, navEnd]);

  const range = viewDays(st.view, new Date(st.anchor), { weekStartsOn: settings.weekStartsOn, workDays: settings.workDays, customDays: st.customDays });
  const selected =
    st.listMode !== "none" || st.search
      ? undefined
      : st.view === "month"
        ? { start: startOfMonth(st.anchor), end: plusDays(addMonths(startOfMonth(st.anchor), 1), -1) }
        : { start: range.days[0], end: range.days[range.days.length - 1] };

  const onSelect = (a: Date, b: Date) => {
    const days = Math.round((dayStart(b).getTime() - dayStart(a).getTime()) / DAY) + 1;
    const s = useCal.getState();
    const base = { anchor: dayStart(a).getTime(), selection: null, search: "", listMode: "none" as const };
    if (days === 1) {
      s.set({ ...base, customDays: 0, view: s.view === "month" ? "month" : s.view });
    } else if (days === 7 && dayStart(a).getDay() === settings.weekStartsOn) {
      s.set({ ...base, customDays: 0, view: "week" });
    } else if (days <= 14) {
      s.set({ ...base, customDays: days, view: "day" });
    } else {
      s.set({ ...base, customDays: 0, view: "month" });
    }
  };

  const list = cals.data ?? [];
  const multi = new Set(list.map((c) => c.accountId)).size > 1;
  const toggle = (c: CalendarInfo) => cal.setSelected(c.id, !c.selected).catch(errorBox);
  const row = (c: CalendarInfo) => (
    <label key={c.id} className="cm-cal-calrow" title={`${c.name} (${c.accountEmail}${c.canWrite ? "" : ", read-only"})`}>
      <span
        className={`cm-cal-check${c.selected ? " on" : ""}`}
        style={{ background: c.selected ? c.color : undefined, borderColor: c.color }}
        onClick={(e) => {
          e.preventDefault();
          toggle(c);
        }}
      >
        {c.selected && "✓"}
      </span>
      <span
        className="cm-ellipsis"
        onClick={(e) => {
          e.preventDefault();
          toggle(c);
        }}
      >
        {calLabel(c, list)}
        {multi && !c.isPrimary && c.group === "mine" && <span className="cm-cal-acct"> - {c.accountEmail}</span>}
      </span>
    </label>
  );

  return (
    <div className="cm-cal-nav" ref={wrapRef}>
      <DateNavigator month={navMonth} onMonthChange={setNavMonth} months={months} selected={selected} onSelect={onSelect} boldDays={bold} weekStartsOn={settings.weekStartsOn} />
      <div className="cm-cal-nav-lists">
        {list.length === 0 && (
          <>
            <div className="cm-nav-section">My Calendars</div>
            <div className="cm-muted" style={{ padding: "4px 10px" }}>
              {cals.isLoading ? "Loading…" : "No calendars yet"}
            </div>
          </>
        )}
        {SECTIONS.map(({ group, title }) => {
          const items = list.filter((c) => c.group === group);
          if (!items.length) return null;
          return (
            <div key={group} className="cm-cal-navgroup">
              <div className="cm-nav-section">{title}</div>
              {items.map(row)}
            </div>
          );
        })}
        <div className="cm-cal-links">
          <a
            className="cm-nav-link disabled"
            onClick={() =>
              alertBox(
                "Opening other people's calendars isn't available yet. Share calendars from Google Calendar on the web; they then appear here automatically.",
                "Open a Shared Calendar",
              )
            }
          >
            Open a Shared Calendar…
          </a>
          <a
            className="cm-nav-link disabled"
            onClick={() => alertBox("Share your calendar from Google Calendar on the web (Settings › Share with specific people).", "Share My Calendar")}
          >
            Share My Calendar…
          </a>
        </div>
      </div>
    </div>
  );
}
