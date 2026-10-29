// Calendar commands shared by the view, toolbar, menus and item windows.
import { cal, type BusyStatus, type CalEvent, type Response, type Scope } from "../../api/calendar";
import { openWindow } from "../../api/windows";
import { errorBox } from "../../ui";
import { nextSlot, dayStart, plusDays, HOUR, fmtLongDate, fmtTime } from "./dates";
import { promptBox, proposeTimeBox, radioBox } from "./dialogs";
import type { ItemMailMode } from "./itemMail";

type Item = Pick<CalEvent, "id" | "summary" | "isRecurring" | "isMeeting" | "isOrganizer"> & Partial<CalEvent>;

export function itemTitle(summary: string, meeting: boolean) {
  return `${summary || "Untitled"} - ${meeting ? "Meeting" : "Appointment"}`;
}

/** Ask "this occurrence or the series?" for a recurring item. */
export async function askScope(ev: Item, action: "open" | "delete" | "respond"): Promise<Scope | null> {
  if (!ev.isRecurring) return "occurrence";
  const name = ev.summary || "Untitled";
  if (action === "open") {
    const v = await radioBox("Open Recurring Item", `"${name}" is a recurring appointment. Do you want to open only this occurrence or the series?`, [
      { value: "occurrence", label: "Open this occurrence." },
      { value: "series", label: "Open the series." },
    ]);
    return v as Scope | null;
  }
  if (action === "delete") {
    const v = await radioBox(
      "Confirm Delete",
      `Do you want to delete all occurrences of the recurring appointment "${name}", or just this one?`,
      [
        { value: "occurrence", label: "Delete this occurrence." },
        { value: "series", label: "Delete the series." },
      ],
      "occurrence",
      "warning",
    );
    return v as Scope | null;
  }
  const v = await radioBox("Respond to Recurring Meeting", `"${name}" is a recurring meeting. Do you want to respond to this occurrence or to the whole series?`, [
    { value: "series", label: "Respond to the series." },
    { value: "occurrence", label: "Respond to this occurrence only." },
  ]);
  return v as Scope | null;
}

export async function openItem(ev: Item, scope?: Scope) {
  const s = scope ?? (await askScope(ev, "open"));
  if (!s) return;
  await openWindow("appointment", { id: ev.id, scope: s }, { title: itemTitle(ev.summary, ev.isMeeting), label: `appointment-${ev.id}-${s}` });
}

export function newAppointment(opts: { start?: number; end?: number; allDay?: boolean; meeting?: boolean; summary?: string; calendarId?: number } = {}) {
  let { start, end } = opts;
  if (start === undefined) {
    start = opts.allDay ? dayStart(Date.now()).getTime() : nextSlot();
    end = opts.allDay ? plusDays(start, 1).getTime() : start + HOUR / 2;
  }
  return openWindow(
    "appointment",
    { start, end: end ?? start + HOUR / 2, allDay: opts.allDay ? 1 : undefined, meeting: opts.meeting ? 1 : undefined, summary: opts.summary, calendarId: opts.calendarId },
    { title: opts.meeting ? "Untitled - Meeting" : opts.allDay ? "Untitled - Event" : "Untitled - Appointment" },
  );
}

/** Delete with Outlook's confirmations. Returns true when something was deleted. */
export async function deleteItem(ev: Item): Promise<boolean> {
  if (ev.readOnly) {
    errorBox("You do not have sufficient permission to perform this operation on this object.");
    return false;
  }
  try {
    if (ev.isMeeting && !ev.isOrganizer && ev.myResponse !== "declined") {
      const v = await radioBox(
        "Confirm Delete",
        `The meeting "${ev.summary || "Untitled"}" will be removed from your calendar.`,
        [
          { value: "send", label: "Delete and send a response to the meeting organizer." },
          { value: "silent", label: "Delete without sending a response." },
        ],
        "send",
        "warning",
      );
      if (!v) return false;
      const scope = await askScope(ev, "respond");
      if (!scope) return false;
      if (v === "send") await cal.respond(ev.id, "declined", { scope, sendResponse: true });
      else await cal.remove(ev.id, scope);
      return true;
    }
    const scope = await askScope(ev, "delete");
    if (!scope) return false;
    let send = false;
    if (ev.isMeeting && ev.isOrganizer) {
      const v = await radioBox(
        "Confirm Delete",
        `"${ev.summary || "Untitled"}" is a meeting with attendees.`,
        [
          { value: "send", label: "Send cancellation and delete meeting." },
          { value: "silent", label: "Delete without sending a cancellation." },
        ],
        "send",
        "warning",
      );
      if (!v) return false;
      send = v === "send";
    }
    await cal.remove(ev.id, scope, send, `Delete "${ev.summary || "Untitled"}"`);
    return true;
  } catch (e) {
    errorBox(e);
    return false;
  }
}

export type RespondHow = "edit" | "send" | "none";

/**
 * Accept / Tentative / Decline with "Edit the response before sending / Send now / Don't send"
 * (asked here unless the right-click submenu already chose).
 */
export async function respondTo(ev: Item, response: Exclude<Response, "needsAction">, chosen?: RespondHow) {
  try {
    const scope = await askScope(ev, "respond");
    if (!scope) return;
    const word = response === "accepted" ? "Accepting" : response === "tentative" ? "Tentatively accepting" : "Declining";
    const how =
      chosen ??
      (await radioBox(
        `${word} Meeting`,
        `${word} "${ev.summary || "Untitled"}".`,
        [
          { value: "edit", label: "Edit the response before sending" },
          { value: "send", label: "Send the response now" },
          { value: "none", label: "Don't send a response" },
        ],
        "send",
      ));
    if (!how) return;
    let comment: string | undefined;
    if (how === "edit") {
      const c = await promptBox("Response", "Add a note for the organizer:", "", true);
      if (c === null) return;
      comment = c;
    }
    await cal.respond(ev.id, response, { scope, comment, sendResponse: how !== "none" });
  } catch (e) {
    errorBox(e);
  }
}

/**
 * Tentative / Decline and Propose New Time. Google has no proposals in its API, so the new time
 * goes to the organizer as the note on the response (shown in the reply they get).
 */
export async function proposeNewTime(ev: Item & Pick<CalEvent, "start" | "end">, response: "tentative" | "declined") {
  try {
    const scope = await askScope(ev, "respond");
    if (!scope) return;
    const p = await proposeTimeBox(ev.summary, ev.start, ev.end);
    if (!p) return;
    const same = new Date(p.start).toDateString() === new Date(p.end).toDateString();
    const when = `${fmtLongDate(p.start)} ${fmtTime(p.start)} - ${same ? "" : `${fmtLongDate(p.end)} `}${fmtTime(p.end)}`;
    const comment = `New time proposed: ${when}${p.note ? `\n\n${p.note}` : ""}`;
    await cal.respond(ev.id, response, { scope, comment, sendResponse: true });
  } catch (e) {
    errorBox(e);
  }
}

/** Reply (to everyone) / Forward: a new message with the meeting's details (Outlook's meeting replies). */
export function mailAboutItem(ev: Item & Pick<CalEvent, "accountId">, mode: ItemMailMode) {
  const prefix = mode === "forward" ? "FW" : "RE";
  return openWindow("compose", { mode: "new", event: ev.id, eventMode: mode, accountId: ev.accountId }, { title: `${prefix}: ${ev.summary || "Untitled"} - Message` });
}

/** Quick Print: the item on paper, without opening it. */
export async function quickPrint(id: number) {
  try {
    const [d, m] = await Promise.all([cal.getEvent(id), import("./itemMail")]);
    m.printItem(d);
  } catch (e) {
    errorBox(e);
  }
}

/**
 * Moving / resizing a meeting you organise (drag in the calendar): Outlook asks whether to tell
 * the attendees. Returns true = send an update, false = save without sending, null = don't save.
 */
export async function askSendUpdate(ev: Item): Promise<boolean | null> {
  if (!(ev.isMeeting && ev.isOrganizer)) return false;
  const v = await radioBox(
    "Send Update to Attendees",
    "Would you like to send an update to attendees regarding changes to your meeting?",
    [
      { value: "send", label: "Save changes and send update." },
      { value: "save", label: "Save changes but don't send." },
    ],
    "send",
  );
  return v === null ? null : v === "send";
}

export async function setShowAs(ev: Item, busyStatus: BusyStatus) {
  try {
    await cal.update(ev.id, { busyStatus }, "occurrence", `Show As ${SHOW_AS.find((s) => s.value === busyStatus)?.label ?? busyStatus}`);
  } catch (e) {
    errorBox(e);
  }
}

export async function setPrivate(ev: Item, isPrivate: boolean) {
  try {
    await cal.update(ev.id, { isPrivate }, "occurrence", isPrivate ? "Mark Private" : "Clear Private");
  } catch (e) {
    errorBox(e);
  }
}

/** Set an item's colour categories (an occurrence: its series, as in Outlook). */
export async function setCategories(ev: Item, categories: string[]) {
  try {
    await cal.setCategories([ev.id], categories, categories.length ? `Categorize "${ev.summary || "Untitled"}"` : "Clear Categories");
  } catch (e) {
    errorBox(e);
  }
}

export const SHOW_AS: { value: BusyStatus; label: string }[] = [
  { value: "free", label: "Free" },
  { value: "tentative", label: "Tentative" },
  { value: "busy", label: "Busy" },
  { value: "oof", label: "Out of Office" },
];
