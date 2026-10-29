// Calendar module: Outlook 2007 calendar (Day / Work Week / Week / Month), Date Navigator,
// My Calendars, appointment / meeting items, reminders.
import type { ModuleDef, ModuleMenus } from "../../shell/types";
import { Toolbar, TbButton, TbDropdown, TbSplit, TbSep, type MenuItem } from "../../ui";
import { newItemMenu } from "../../shell/newItems";
import { openWindow } from "../../api/windows";
import { NavPane } from "./NavPane";
import { CalendarView, navigate } from "./CalendarView";
import { deleteItem, setCategories } from "./actions";
import { categorizeMenu, currentCategories, openColorCategories, useCategories } from "./categories";
import { openCleanUp } from "./cleanup";
import { goToDate, goToToday, newFromSelection, timeScaleItems, viewItems } from "./menus";
import { useCal, useCalSettings, type MonthDetail } from "./state";
import { useQueryClient } from "@tanstack/react-query";
import type { CalEvent } from "../../api/calendar";

/** Cached lists of calendar items (not the calendars or categories, whose ids are another table's). */
const ITEM_LISTS = [
  ["calendar", "events"],
  ["calendar", "search"],
  ["calendar", "series"],
];

/** The selected item, looked up in whatever calendar item lists are cached. */
function useSelectedItem(): () => CalEvent | undefined {
  const qc = useQueryClient();
  return () => {
    const id = useCal.getState().selectedId;
    if (id === null) return undefined;
    for (const [, data] of ITEM_LISTS.flatMap((queryKey) => qc.getQueriesData<CalEvent[]>({ queryKey }))) {
      if (Array.isArray(data)) {
        const hit = data.find((e) => e && e.id === id);
        if (hit) return hit;
      }
    }
    return undefined;
  };
}

/** "Categorize ▸" for the selected item (or "All Categories…" only, to edit the list). */
function categorizeItems(ev: CalEvent | undefined): MenuItem[] {
  if (!ev) return [{ label: "All Categories…", onClick: () => openColorCategories(null) }];
  return categorizeMenu(ev.categories ?? [], currentCategories(), (names) => setCategories(ev, names));
}

function arrangeItems(): MenuItem[] {
  const s = useCal.getState();
  return [
    { label: "Side by Side", radio: true, checked: s.arrangement === "sideBySide", onClick: () => useCal.getState().set({ arrangement: "sideBySide", overlaid: [] }) },
    { label: "Overlay", radio: true, checked: s.arrangement === "overlay", onClick: () => useCal.getState().set({ arrangement: "overlay" }) },
  ];
}

function ModToolbar() {
  const st = useCal();
  const selected = useSelectedItem();
  useCategories();
  const week = st.view === "week" || st.view === "workweek";
  return (
    <Toolbar>
      <TbSplit icon="appointment" label="New" title="New Appointment (Ctrl+N)" menu={newItemMenu} onClick={() => calendarModule.newItem()} />
      <TbSep />
      <TbButton icon="print" title="Print (Ctrl+P)" onClick={() => window.print()} />
      <TbButton
        icon="delete"
        title="Delete (Ctrl+D)"
        disabled={st.selectedId === null}
        onClick={() => {
          const ev = selected();
          if (ev) deleteItem(ev).then((ok) => ok && useCal.getState().set({ selectedId: null }));
        }}
      />
      <TbSep />
      <TbButton icon="today" label="Today" title="Go to Today" onClick={goToToday} />
      <TbSep />
      <TbButton icon="view-day" label="Day" pressed={st.view === "day" && st.listMode === "none"} onClick={() => st.setView("day")} />
      <TbButton icon="view-week" label="Week" pressed={week && st.listMode === "none"} onClick={() => st.setView(st.view === "week" ? "week" : "workweek")} />
      <TbButton icon="view-month" label="Month" pressed={st.view === "month" && st.listMode === "none"} onClick={() => st.setView("month")} />
      <TbButton
        label="Overlay"
        title="Side by Side / Overlay: show checked calendars next to each other or on top of each other"
        pressed={st.arrangement === "overlay"}
        onClick={() => st.set({ arrangement: st.arrangement === "overlay" ? "sideBySide" : "overlay", overlaid: [] })}
      />
      <TbSep />
      <TbDropdown icon="cm-cat-1" label="Categorize" title="Categorize" menu={() => categorizeItems(selected())} />
      <TbButton icon="delete" label="Clean Up" title="Clean Up Calendar: find and delete duplicate items" onClick={openCleanUp} />
      <TbSep />
      <TbButton icon="address-book" title="Address Book (Ctrl+Shift+B)" onClick={() => openWindow("address-book", {}, { label: "address-book", title: "Address Book" })} />
    </Toolbar>
  );
}

function useMenus(): ModuleMenus {
  const st = useCal();
  const settings = useCalSettings();
  const selected = useSelectedItem();
  const detail = (d: MonthDetail, label: string): MenuItem => ({ label, radio: true, checked: st.monthDetail === d, onClick: () => useCal.getState().set({ monthDetail: d }) });
  return {
    edit: [
      {
        label: "Delete",
        icon: "delete",
        shortcut: "Ctrl+D",
        disabled: st.selectedId === null,
        onClick: () => {
          const ev = selected();
          if (ev) deleteItem(ev);
        },
      },
      { type: "separator" },
      { label: "Go to Date…", shortcut: "Ctrl+G", onClick: goToDate },
    ],
    view: [
      {
        label: "Current View",
        submenu: [
          ...viewItems(),
          { type: "separator" },
          { label: "Recurring Appointments", radio: true, checked: st.listMode === "recurring", onClick: () => useCal.getState().set({ listMode: "recurring", search: "" }) },
        ],
      },
      { label: "Day", icon: "view-day", shortcut: "Ctrl+Alt+1", radio: true, checked: st.view === "day", onClick: () => st.setView("day") },
      { label: "Work Week", icon: "view-week", shortcut: "Ctrl+Alt+2", radio: true, checked: st.view === "workweek", onClick: () => st.setView("workweek") },
      { label: "Week", icon: "view-week", shortcut: "Ctrl+Alt+3", radio: true, checked: st.view === "week", onClick: () => st.setView("week") },
      { label: "Month", icon: "view-month", shortcut: "Ctrl+Alt+4", radio: true, checked: st.view === "month", onClick: () => st.setView("month") },
      { type: "separator" },
      { label: "Month Details", submenu: [detail("low", "Low"), detail("medium", "Medium"), detail("high", "High")] },
      { label: "Time Scale", submenu: timeScaleItems(settings.timeScale) },
      { label: "Arrange Calendars", submenu: arrangeItems() },
      { type: "separator" },
      { label: "Today", icon: "today", onClick: goToToday },
      { label: "Previous", icon: "arrow-left", shortcut: "Alt+Left", onClick: () => navigate(-1) },
      { label: "Next", icon: "arrow-right", shortcut: "Alt+Right", onClick: () => navigate(1) },
    ],
    actions: [
      { label: "New Appointment", icon: "appointment", shortcut: "Ctrl+N", onClick: () => newFromSelection(useCal.getState().selection) },
      { label: "New All Day Event", icon: "calendar", onClick: () => newFromSelection(useCal.getState().selection, { allDay: true }) },
      { label: "New Meeting Request", icon: "meeting-request", shortcut: "Ctrl+Shift+Q", onClick: () => newFromSelection(useCal.getState().selection, { meeting: true }) },
      { type: "separator" },
      { label: "View Recurrences…", icon: "recurring", onClick: () => useCal.getState().set({ listMode: "recurring", search: "", selectedId: null }) },
      { type: "separator" },
      { label: "Categorize", icon: "cm-cat-1", submenu: categorizeItems(selected()) },
      { label: "Clean Up Calendar…", onClick: openCleanUp },
      { type: "separator" },
      { label: "Go to Date…", shortcut: "Ctrl+G", onClick: goToDate },
    ],
  };
}

export const calendarModule: ModuleDef = {
  id: "calendar",
  label: "Calendar",
  icon: "calendar",
  NavPane,
  View: CalendarView,
  Toolbar: ModToolbar,
  useMenus,
  newItem: () => newFromSelection(useCal.getState().selection),
};
