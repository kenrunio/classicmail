// Calendar view state (shared by nav pane, view, toolbar and menus) + calendar settings.
import { create } from "zustand";
import { persist } from "zustand/middleware";
import { useRpc } from "../../api/query";
import type { CalEvent } from "../../api/calendar";
import type { ViewKind } from "./dates";
import { dayStart } from "./dates";
import type { Arrangement } from "./layout";
import { useShell } from "../../shell/store";

export type MonthDetail = "low" | "medium" | "high";

export interface Selection {
  start: number;
  end: number;
  allDay: boolean;
  /** side by side: the calendar whose columns hold the selection */
  calendarId?: number;
}

interface CalState {
  view: ViewKind;
  /** ms of the anchor day */
  anchor: number;
  /** a Date Navigator range of N days shown as a multi-day "day" view (0 = off) */
  customDays: number;
  monthDetail: MonthDetail;
  /** empty-slot selection */
  selection: Selection | null;
  /** selected item */
  selectedId: number | null;
  /** more items picked with Ctrl+click beside the selected one (cleared with a new selection) */
  alsoSelected: CalEvent[];
  /** Copy (Ctrl+C): the items (one day's) Paste (Ctrl+V) puts in the selected slot */
  clip: CalEvent[];
  /** "Search Calendar" text; when non-empty the view shows results */
  search: string;
  /** "View Recurrences" list mode */
  listMode: "none" | "recurring";
  /** several calendars: side by side (Outlook 2007 default) or overlaid */
  arrangement: Arrangement;
  /** side by side: calendars viewed in overlay mode on the first one */
  overlaid: number[];
  set: (p: Partial<Omit<CalState, "set">>) => void;
  setView: (v: ViewKind) => void;
  goTo: (day: number, view?: ViewKind) => void;
  /** the work week view on this week, as the Calendar opens */
  showThisWeek: () => void;
}

export const useCal = create<CalState>()(
  persist(
    (set) => ({
      view: "workweek",
      anchor: dayStart(Date.now()).getTime(),
      customDays: 0,
      monthDetail: "high",
      selection: null,
      selectedId: null,
      alsoSelected: [],
      clip: [],
      search: "",
      listMode: "none",
      arrangement: "sideBySide",
      overlaid: [],
      // a new selection (an item or a slot) drops the Ctrl+click picks
      set: (p) => set(("selectedId" in p || "selection" in p) && !("alsoSelected" in p) ? { ...p, alsoSelected: [] } : p),
      setView: (view) => set({ view, customDays: 0, listMode: "none", search: "" }),
      goTo: (day, view) => set((s) => ({ anchor: dayStart(day).getTime(), view: view ?? s.view, customDays: view ? 0 : s.customDays, listMode: "none" })),
      showThisWeek: () => set({ view: "workweek", anchor: dayStart(Date.now()).getTime(), customDays: 0, listMode: "none", search: "", selection: null }),
    }),
    {
      name: "cm:calendar",
      // the Calendar always opens on this week (below), so neither the view nor the day is kept
      partialize: (s) => ({ monthDetail: s.monthDetail, arrangement: s.arrangement, overlaid: s.overlaid }),
      version: 1,
      migrate: (old) => {
        const { view: _view, ...rest } = (old ?? {}) as Record<string, unknown>;
        return rest as unknown as CalState;
      },
    },
  ),
);

/** Is the item selected (the selected one or a Ctrl+click pick)? */
export function isPicked(s: { selectedId: number | null; alsoSelected: CalEvent[] }, id: number): boolean {
  return s.selectedId === id || s.alsoSelected.some((x) => x.id === id);
}

/** Ctrl+click on an item while another is selected: add it to the selection, or take it out. */
export function togglePick(ev: CalEvent) {
  const s = useCal.getState();
  if (s.selectedId === null || s.selectedId === ev.id) return s.set({ selectedId: ev.id, selection: null });
  const has = s.alsoSelected.some((x) => x.id === ev.id);
  s.set({ alsoSelected: has ? s.alsoSelected.filter((x) => x.id !== ev.id) : [...s.alsoSelected, ev] });
}

// Clicking Calendar (Navigation Pane, Go menu, Ctrl+2) shows this work week. A date
// clicked in the To-Do Bar switches first, then goes to its day.
useShell.subscribe((s, prev) => {
  if (s.module === "calendar" && prev.module !== "calendar") useCal.getState().showThisWeek();
});

export interface CalSettings {
  workDays: number[];
  workDayStart: number;
  workDayEnd: number;
  weekStartsOn: 0 | 1 | 6;
  defaultReminderMinutes: number;
  timeScale: number;
  /** Day / Week: a second time column for this zone ("" = none), with the labels over both */
  secondZone: string;
  secondZoneLabel: string;
  zoneLabel: string;
}

export const DEFAULT_SETTINGS: CalSettings = {
  workDays: [0, 1, 2, 3, 4],
  workDayStart: 480,
  workDayEnd: 1080,
  weekStartsOn: 0,
  defaultReminderMinutes: 15,
  timeScale: 30,
  secondZone: "",
  secondZoneLabel: "",
  zoneLabel: "",
};

const KEYS = [
  "calendar.workDays",
  "calendar.workDayStart",
  "calendar.workDayEnd",
  "calendar.weekStartsOn",
  "calendar.defaultReminderMinutes",
  "calendar.timeScale",
  "calendar.secondZone",
  "calendar.secondZoneLabel",
  "calendar.zoneLabel",
];

/** Calendar options from Tools › Options (settings keys owned by the Options window). */
export function useCalSettings(): CalSettings {
  const q = useRpc<Record<string, unknown>>(["settings", "calendar"], "settings.getMany", { keys: KEYS }, { staleTime: 60_000 });
  const v = q.data ?? {};
  const num = (k: string, d: number) => (typeof v[k] === "number" ? (v[k] as number) : d);
  const wd = Array.isArray(v["calendar.workDays"]) && (v["calendar.workDays"] as unknown[]).length ? (v["calendar.workDays"] as number[]) : DEFAULT_SETTINGS.workDays;
  const ts = num("calendar.timeScale", 30);
  return {
    workDays: wd,
    workDayStart: num("calendar.workDayStart", 480),
    workDayEnd: num("calendar.workDayEnd", 1080),
    weekStartsOn: [0, 1, 6].includes(num("calendar.weekStartsOn", 0)) ? (num("calendar.weekStartsOn", 0) as 0 | 1 | 6) : 0,
    defaultReminderMinutes: num("calendar.defaultReminderMinutes", 15),
    timeScale: [5, 6, 10, 15, 30, 60].includes(ts) ? ts : 30,
    secondZone: typeof v["calendar.secondZone"] === "string" ? (v["calendar.secondZone"] as string) : "",
    secondZoneLabel: typeof v["calendar.secondZoneLabel"] === "string" ? (v["calendar.secondZoneLabel"] as string) : "",
    zoneLabel: typeof v["calendar.zoneLabel"] === "string" ? (v["calendar.zoneLabel"] as string) : "",
  };
}
