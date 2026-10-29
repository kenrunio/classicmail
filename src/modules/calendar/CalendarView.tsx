// The Calendar module view: Day / Week / Month tab strip, back/forward + date range title,
// "Search Calendar" box, and the active grid (or a search / recurrences list).
import { useEffect, useMemo, useRef } from "react";
import { useQueries } from "@tanstack/react-query";
import { useShallow } from "zustand/react/shallow";
import { useRpc } from "../../api/query";
import { rpc } from "../../api/rpc";
import type { CalEvent, CalendarInfo } from "../../api/calendar";
import { useDebounced, useHotkeys, Icon } from "../../ui";
import { useStatus } from "../../shell/store";
import { addMonths } from "date-fns";
import { dayStart, parseDayKey, plusDays, rangeTitle, viewDays, type ViewKind } from "./dates";
import { DayWeekGrid, type GridGroup } from "./DayWeekGrid";
import { buildGroups } from "./layout";
import { ColorCategoriesHost, shortcutCategory, toggled, useCategories, SHORTCUTS } from "./categories";
import { CleanUpHost } from "./cleanup";
import { MonthGrid } from "./MonthGrid";
import { ListView, type ListRow } from "./ListView";
import { CalDialogHost } from "./dialogs";
import { deleteItem, mailAboutItem, openItem, setCategories } from "./actions";
import { goToDate, goToToday, newFromSelection } from "./menus";
import { useCal, useCalSettings, type MonthDetail } from "./state";
import "./calendar.css";

/** `cm-calendar-goto` detail: a Date, epoch ms, "yyyy-MM-dd" / ISO string, or { date }. */
export function toDay(d: unknown): number | null {
  if (d && typeof d === "object" && !(d instanceof Date) && "date" in d) return toDay((d as { date: unknown }).date);
  if (d instanceof Date) return isNaN(d.getTime()) ? null : d.getTime();
  if (typeof d === "number") return d;
  if (typeof d === "string") {
    if (/^\d{4}-\d{2}-\d{2}$/.test(d)) return parseDayKey(d).getTime();
    const t = Date.parse(d);
    return isNaN(t) ? null : t;
  }
  return null;
}

/** Move the anchor one period back / forward. */
export function navigate(dir: 1 | -1) {
  const s = useCal.getState();
  const a = new Date(s.anchor);
  let next: Date;
  if (s.view === "month") next = addMonths(a, dir);
  else if (s.view === "day") next = plusDays(a, dir * Math.max(1, s.customDays || 1));
  else next = plusDays(a, dir * 7);
  s.set({ anchor: dayStart(next).getTime(), selection: null });
}

/** Each column group's events. A stable function: react-query then hands back the same array
 *  until some group's data changes (a memo can depend on it, whatever the number of groups). */
const dataOf = (results: { data?: CalEvent[] }[]) => results.map((r) => r.data);

export function CalendarView() {
  // everything but the selection: clicks and drags in the grid don't re-render the whole view
  const st = useCal(
    useShallow((s) => ({
      view: s.view,
      anchor: s.anchor,
      customDays: s.customDays,
      monthDetail: s.monthDetail,
      search: s.search,
      listMode: s.listMode,
      arrangement: s.arrangement,
      overlaid: s.overlaid,
      set: s.set,
      setView: s.setView,
    })),
  );
  const settings = useCalSettings();
  const setLeft = useStatus((s) => s.setLeft);
  const anchor = useMemo(() => new Date(st.anchor), [st.anchor]);
  const range = useMemo(
    () => viewDays(st.view, anchor, { weekStartsOn: settings.weekStartsOn, workDays: settings.workDays, customDays: st.customDays }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [st.view, st.anchor, st.customDays, settings.weekStartsOn, settings.workDays.join(",")],
  );
  const search = useDebounced(st.search.trim(), 300);
  const listing = search ? "search" : st.listMode === "recurring" ? "recurring" : "none";
  const cals = useRpc<CalendarInfo[]>(["calendar", "calendars"], "calendar.listCalendars");
  const selectedCals = useMemo(() => (cals.data ?? []).filter((c) => c.selected), [cals.data]);
  // Day / Week with several calendars: side by side (one request per column group, each merged
  // for duplicates on its own); Month and single calendars: one overlaid list
  const grouped = listing === "none" && st.view !== "month";
  const calGroups = useMemo(() => (grouped ? buildGroups(selectedCals, st.arrangement, st.overlaid) : [selectedCals]), [grouped, selectedCals, st.arrangement, st.overlaid]);
  const groupData = useQueries({
    queries: calGroups.map((g) => {
      const ids = g.map((c) => c.id);
      return {
        queryKey: ["calendar", "events", range.start, range.end, calGroups.length > 1 ? ids.join(",") : "all"],
        queryFn: () =>
          rpc<CalEvent[]>("calendar.listEvents", calGroups.length > 1 ? { start: range.start, end: range.end, calendarIds: ids } : { start: range.start, end: range.end }),
        placeholderData: (p: CalEvent[] | undefined) => p,
        enabled: listing === "none",
      };
    }),
    combine: dataOf,
  });
  const gridGroups: GridGroup[] = useMemo(() => calGroups.map((g, i) => ({ cals: g, events: groupData[i] ?? [] })), [calGroups, groupData]);
  // one array per data change, so the Month grid's layout memo holds between renders
  const list = useMemo(() => gridGroups.flatMap((g) => g.events), [gridGroups]);
  const results = useRpc<ListRow[]>(["calendar", "search", search], "calendar.search", { query: search }, { enabled: listing === "search" });
  const series = useRpc<ListRow[]>(["calendar", "series"], "calendar.listSeries", {}, { enabled: listing === "recurring" });
  const searchRef = useRef<HTMLInputElement>(null);
  const categories = useCategories();
  const onOverlay = (calendarId: number, overlay: boolean) => {
    const cur = useCal.getState().overlaid;
    st.set({ overlaid: overlay ? [...new Set([...cur, calendarId])] : cur.filter((x) => x !== calendarId) });
  };

  const count = listing === "search" ? (results.data?.length ?? 0) : listing === "recurring" ? (series.data?.length ?? 0) : list.length;
  useEffect(() => {
    setLeft(`Items: ${count.toLocaleString()}`);
  }, [count, setLeft]);
  useEffect(() => () => setLeft(""), [setLeft]);

  useEffect(() => {
    const f = () => searchRef.current?.focus();
    window.addEventListener("cm-focus-search", f);
    const p = () => window.print();
    window.addEventListener("cm-print", p);
    // the To-Do Bar's date navigator: show that day
    const g = (e: Event) => {
      const t = toDay((e as CustomEvent).detail);
      if (t !== null) useCal.getState().goTo(t, "day");
    };
    window.addEventListener("cm-calendar-goto", g);
    return () => {
      window.removeEventListener("cm-focus-search", f);
      window.removeEventListener("cm-print", p);
      window.removeEventListener("cm-calendar-goto", g);
    };
  }, []);

  const selected = () => (listing === "search" ? results.data : listing === "recurring" ? series.data : list)?.find((e) => e.id === useCal.getState().selectedId);
  const nDays = (k: number) => () => useCal.getState().set({ view: "day", customDays: k === 1 ? 0 : k, listMode: "none", search: "" });
  // Ctrl+F2..F12: the colour category with that shortcut key (toggle on the selected item)
  const catKeys: Record<string, () => void> = {};
  for (const k of SHORTCUTS) {
    catKeys[`ctrl+${k.toLowerCase()}`] = () => {
      const ev = selected();
      const name = shortcutCategory(k, categories);
      if (ev && name) setCategories(ev, toggled(ev.categories ?? [], name));
    };
  }
  useHotkeys({
    ...catKeys,
    "ctrl+g": () => goToDate(),
    "ctrl+alt+1": () => st.setView("day"),
    "ctrl+alt+2": () => st.setView("workweek"),
    "ctrl+alt+3": () => st.setView("week"),
    "ctrl+alt+4": () => st.setView("month"),
    "alt+=": () => st.setView("month"),
    "alt+-": () => st.setView(st.view === "week" ? "workweek" : "week"),
    "alt+1": nDays(1),
    "alt+2": nDays(2),
    "alt+3": nDays(3),
    "alt+4": nDays(4),
    "alt+5": nDays(5),
    "alt+6": nDays(6),
    "alt+7": nDays(7),
    "alt+8": nDays(8),
    "alt+9": nDays(9),
    "alt+0": nDays(10),
    "alt+arrowleft": () => navigate(-1),
    "alt+arrowright": () => navigate(1),
    pageup: () => navigate(-1),
    pagedown: () => navigate(1),
    "ctrl+t": () => goToToday(),
    "ctrl+e": () => searchRef.current?.focus(),
    "ctrl+p": () => window.print(),
    "ctrl+shift+a": () => newFromSelection(useCal.getState().selection),
    "ctrl+shift+q": () => newFromSelection(useCal.getState().selection, { meeting: true }),
    "ctrl+o": () => {
      const ev = selected();
      if (ev) openItem(ev);
    },
    // a meeting's Reply goes to everyone (as in mail)
    "ctrl+r": () => {
      const ev = selected();
      if (ev?.isMeeting) mailAboutItem(ev, "reply");
    },
    "ctrl+shift+r": () => {
      const ev = selected();
      if (ev?.isMeeting) mailAboutItem(ev, "reply");
    },
    "ctrl+f": () => {
      const ev = selected();
      if (ev) mailAboutItem(ev, "forward");
    },
    "ctrl+d": () => {
      const ev = selected();
      if (ev) deleteItem(ev).then((ok) => ok && useCal.getState().set({ selectedId: null }));
    },
    delete: () => {
      const ev = selected();
      if (ev) deleteItem(ev).then((ok) => ok && useCal.getState().set({ selectedId: null }));
    },
  });

  const weekTab = st.view === "week" || st.view === "workweek";
  const tab = (label: string, active: boolean, onClick: () => void) => (
    <button className={`cm-cal-tab${active ? " active" : ""}`} onClick={onClick}>
      {label}
    </button>
  );
  const title = listing === "search" ? "Search Results" : listing === "recurring" ? "Recurring Appointments" : rangeTitle(st.view, range.days, anchor);
  const setView = (v: ViewKind) => st.setView(v);

  return (
    <div className="cm-cal-view">
      <div className="cm-cal-tabs">
        {tab("Day", st.view === "day" && listing === "none", () => setView("day"))}
        {tab("Week", weekTab && listing === "none", () => setView(st.view === "week" ? "week" : "workweek"))}
        {tab("Month", st.view === "month" && listing === "none", () => setView("month"))}
        {weekTab && listing === "none" && (
          <div className="cm-cal-radios">
            <label>
              <input type="radio" checked={st.view === "workweek"} onChange={() => setView("workweek")} /> Show work week
            </label>
            <label>
              <input type="radio" checked={st.view === "week"} onChange={() => setView("week")} /> Show full week
            </label>
          </div>
        )}
        {st.view === "month" && listing === "none" && (
          <div className="cm-cal-radios">
            <span>Details:</span>
            {(["low", "medium", "high"] as MonthDetail[]).map((d) => (
              <label key={d}>
                <input type="radio" checked={st.monthDetail === d} onChange={() => st.set({ monthDetail: d })} /> {d[0].toUpperCase() + d.slice(1)}
              </label>
            ))}
          </div>
        )}
      </div>
      <div className="cm-cal-navrow">
        {listing === "none" ? (
          <>
            <button className="cm-cal-navbtn" title="Back" onClick={() => navigate(-1)}>
              <Icon name="arrow-left" size={14} />
            </button>
            <button className="cm-cal-navbtn" title="Forward" onClick={() => navigate(1)}>
              <Icon name="arrow-right" size={14} />
            </button>
          </>
        ) : (
          <button className="cm-cal-navbtn" title="Back to the calendar" onClick={() => st.set({ search: "", listMode: "none" })}>
            <Icon name="arrow-left" size={14} />
          </button>
        )}
        <div className="cm-cal-title cm-ellipsis">{title}</div>
        <div className="cm-cal-search">
          <input
            ref={searchRef}
            placeholder="Search Calendar"
            value={st.search}
            onChange={(e) => st.set({ search: e.target.value })}
            onKeyDown={(e) => e.key === "Escape" && st.set({ search: "" })}
          />
          {st.search ? (
            <button title="Clear search" onClick={() => st.set({ search: "" })}>
              <Icon name="close" size={10} />
            </button>
          ) : (
            <Icon name="find" size={14} />
          )}
        </div>
      </div>
      <div className="cm-cal-body">
        {listing === "search" ? (
          <ListView rows={results.data ?? []} emptyText={results.isFetching ? "Searching…" : "There are no items to show in this view."} />
        ) : listing === "recurring" ? (
          <ListView rows={series.data ?? []} series emptyText="There are no recurring appointments." />
        ) : st.view === "month" ? (
          <MonthGrid days={range.days} events={list} settings={settings} anchor={anchor} />
        ) : (
          <DayWeekGrid
            days={range.days}
            groups={gridGroups}
            settings={settings}
            showTabs={selectedCals.length > 1 && st.arrangement === "sideBySide"}
            allCalendars={cals.data ?? []}
            onOverlay={onOverlay}
          />
        )}
      </div>
      <CalDialogHost />
      <ColorCategoriesHost />
      <CleanUpHost />
    </div>
  );
}
