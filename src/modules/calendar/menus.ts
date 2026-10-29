// Right-click menus and menu-bar contributions of the calendar.
import type { CalEvent } from "../../api/calendar";
import { rpc } from "../../api/rpc";
import type { MenuItem } from "../../ui";
import { allDayRange, dayStart } from "./dates";
import { goToDateBox } from "./dialogs";
import { deleteItem, mailAboutItem, newAppointment, openItem, proposeNewTime, quickPrint, respondTo, setCategories, setPrivate, setShowAs, SHOW_AS } from "./actions";
import { categorizeMenu, currentCategories } from "./categories";
import { useCal, type Selection } from "./state";

export function goToToday() {
  const now = Date.now();
  useCal.getState().set({ anchor: dayStart(now).getTime(), selection: null, listMode: "none", search: "" });
}

export async function goToDate() {
  const s = useCal.getState();
  const r = await goToDateBox(s.anchor, s.view);
  if (r) useCal.getState().set({ anchor: dayStart(r.date).getTime(), view: r.view, customDays: 0, listMode: "none", search: "" });
}

export function setTimeScale(minutes: number) {
  rpc("settings.set", { key: "calendar.timeScale", value: minutes }).catch(() => {});
}

export function newFromSelection(sel: Selection | null, opts: { allDay?: boolean; meeting?: boolean } = {}) {
  if (!sel) return newAppointment(opts);
  if (opts.allDay || sel.allDay) {
    const { start, end } = allDayRange(sel.start, sel.end);
    return newAppointment({ start, end, allDay: true, meeting: opts.meeting, calendarId: sel.calendarId });
  }
  return newAppointment({ start: sel.start, end: sel.end, meeting: opts.meeting, calendarId: sel.calendarId });
}

export function viewItems(): MenuItem[] {
  const s = useCal.getState();
  const v = (label: string, view: typeof s.view, icon: string, shortcut: string): MenuItem => ({
    label,
    icon,
    shortcut,
    radio: true,
    checked: s.view === view && !s.customDays && s.listMode === "none",
    onClick: () => useCal.getState().setView(view),
  });
  return [
    v("Day", "day", "view-day", "Ctrl+Alt+1"),
    v("Work Week", "workweek", "view-week", "Ctrl+Alt+2"),
    v("Week", "week", "view-week", "Ctrl+Alt+3"),
    v("Month", "month", "view-month", "Ctrl+Alt+4"),
  ];
}

export function timeScaleItems(current: number): MenuItem[] {
  return [5, 6, 10, 15, 30, 60].map((m) => ({ label: `${m} Minutes`, radio: true, checked: current === m, onClick: () => setTimeScale(m) }));
}

export function slotMenu(sel: Selection | null, timeScale: number): MenuItem[] {
  return [
    { label: "New Appointment", icon: "appointment", bold: true, onClick: () => newFromSelection(sel) },
    { label: "New All Day Event", icon: "calendar", onClick: () => newFromSelection(sel, { allDay: true }) },
    { label: "New Meeting Request", icon: "meeting-request", onClick: () => newFromSelection(sel, { meeting: true }) },
    { type: "separator" },
    { label: "Go to Today", icon: "today", onClick: goToToday },
    { label: "Go to Date…", shortcut: "Ctrl+G", onClick: goToDate },
    { type: "separator" },
    { label: "View", submenu: viewItems() },
    { label: "Time Scale", submenu: timeScaleItems(timeScale) },
  ];
}

export function itemMenu(ev: CalEvent): MenuItem[] {
  const canRespond = ev.isMeeting && !ev.isOrganizer;
  const editable = !ev.readOnly;
  // Accept ▸ / Tentative ▸ / Decline ▸ — Outlook's three ways of answering
  const respond = (response: "accepted" | "tentative" | "declined"): MenuItem[] => [
    { label: "Edit the Response before Sending…", onClick: () => respondTo(ev, response, "edit") },
    { label: "Send the Response Now", bold: true, onClick: () => respondTo(ev, response, "send") },
    { label: "Do Not Send a Response", onClick: () => respondTo(ev, response, "none") },
  ];
  return [
    { label: "Open", icon: "appointment", bold: true, onClick: () => openItem(ev, ev.isRecurring ? "occurrence" : undefined) },
    ...(ev.isRecurring ? [{ label: "Open Series", icon: "recurring", onClick: () => openItem(ev, "series") } as MenuItem] : []),
    { label: "Quick Print", icon: "print", onClick: () => quickPrint(ev.id) },
    ...(canRespond
      ? ([
          { type: "separator" },
          { label: "Accept", icon: "accept", checked: ev.myResponse === "accepted", submenu: respond("accepted") },
          { label: "Tentative", icon: "tentative", checked: ev.myResponse === "tentative", submenu: respond("tentative") },
          { label: "Decline", icon: "decline", submenu: respond("declined") },
          {
            label: "Propose New Time",
            icon: "propose-time",
            submenu: [
              { label: "Tentative and Propose New Time…", onClick: () => proposeNewTime(ev, "tentative") },
              { label: "Decline and Propose New Time…", onClick: () => proposeNewTime(ev, "declined") },
            ],
          },
        ] as MenuItem[])
      : []),
    { type: "separator" },
    // Reply goes to everyone; the message window's Reply to One narrows it to the organizer
    ...(ev.isMeeting ? [{ label: "Reply", icon: "reply", shortcut: "Ctrl+R", onClick: () => mailAboutItem(ev, "reply") } as MenuItem] : []),
    { label: "Forward", icon: "forward", shortcut: "Ctrl+F", onClick: () => mailAboutItem(ev, "forward") },
    { type: "separator" },
    { label: "Private", icon: "private", checked: ev.isPrivate, disabled: !editable, onClick: () => setPrivate(ev, !ev.isPrivate) },
    {
      label: "Show As",
      icon: "show-as",
      disabled: !editable,
      submenu: SHOW_AS.map((s) => ({ label: s.label, radio: true, checked: ev.busyStatus === s.value, onClick: () => setShowAs(ev, s.value) })),
    },
    { label: "Categorize", icon: "cm-cat-1", submenu: categorizeMenu(ev.categories ?? [], currentCategories(), (names) => setCategories(ev, names)) },
    { type: "separator" },
    { label: "Delete", icon: "delete", disabled: !editable, onClick: () => deleteItem(ev) },
  ];
}
