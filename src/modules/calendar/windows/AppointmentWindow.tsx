// Outlook 2007 Appointment / Meeting form.
// Params: { id, scope?: "occurrence"|"series" } to open an item, or { start, end, allDay?, meeting?, summary? } for a new one.
// Recurrence: opens the `recurrence` window with a request id and listens for its answer on the
// Tauri event "cm-recurrence-result" ({ req, action: "ok"|"remove"|"cancel", recurrence, start, end, allDay }).
import { useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { listen } from "@tauri-apps/api/event";
import { cal, type BusyStatus, type CalendarInfo, type CalEvent, type EventDetail, type EventInput, type Scope } from "../../../api/calendar";
import { rpc } from "../../../api/rpc";
import { useRpc } from "../../../api/query";
import { closeCurrentWindow, openWindow, setWindowTitle, showAtNormalSize, windowParams } from "../../../api/windows";
import { confirmBox, errorBox, Icon, messageBox, Ribbon, RibbonBig, RibbonSmall, RibbonStack, useHotkeys } from "../../../ui";
import { SelectNamesDialog, type Addr } from "../../contacts/SelectNames";
import { AddressTokens } from "../../mail/AddressTokens";
import { RecipientField, toRecipients, type Recipient, type RecipientFieldHandle } from "../../mail/compose/RecipientField";
import { isValidEmail, parseAddresses } from "../../../api/mailCompose";
import "../../mail/compose/compose.css";
import { useUsualGroup } from "../../mail/compose/usualGroup";
import { fmtLongDate, fmtReminder, fmtTime, minutesOf, nextSlot, parseDayKey, plusDays, REMINDER_CHOICES, sameLengthEnd, dayKey, MIN } from "../dates";
import { describeRecurrence } from "../recurrence";
import { CalDialogHost } from "../dialogs";
import { askScope, deleteItem, mailAboutItem, proposeNewTime, quickPrint, respondTo, SHOW_AS } from "../actions";
import { useCalSettings } from "../state";
import { categorizeMenu, categoryColor, ColorCategoriesHost, useCategories } from "../categories";
import { DateField, TimeField } from "./fields";
import { ResponsesDialog } from "./ResponsesDialog";
import { notesParts, tableKinds, tidyNotes, type TidyStyle } from "../notesTable";
import { isHtmlNotes, notesText } from "../itemMail";
import { firstUrl, hasUrl, Linkified, openLink } from "../links";
import "../calendar.css";
import { epochOf, localZone, wallOf, zoneList, zoneName } from "../timeZones";

interface Form {
  calendarId?: number;
  summary: string;
  location: string;
  description: string;
  allDay: boolean;
  startDate: string;
  startMin: number;
  /** for all-day items: the inclusive last day, as Outlook shows it */
  endDate: string;
  endMin: number;
  attendees: Addr[];
  reminder: number;
  busyStatus: BusyStatus;
  isPrivate: boolean;
  importance: "low" | "normal" | "high";
  recurrence: string[] | null;
  meeting: boolean;
  categories: string[];
  /** the zone the start / end times are shown and typed in (Time Zones); default your own */
  timeZone: string;
}

function fromDetail(d: EventDetail, series: boolean): Form {
  const start = series && d.seriesStart != null ? d.seriesStart : d.start;
  const end = series && d.seriesEnd != null ? d.seriesEnd : d.end;
  const others = d.attendees.filter((a) => !a.isSelf || !d.isOrganizer).filter((a) => !(a.organizer && d.isOrganizer));
  const timeZone = d.timeZone || localZone();
  const s = wallOf(start, timeZone);
  const e = wallOf(end, timeZone);
  return {
    calendarId: d.calendarId,
    summary: d.summary,
    location: d.location,
    // HTML notes (Google Calendar, an Outlook series) read as text; saved back unchanged unless edited
    description: notesText(d.description),
    allDay: d.allDay,
    startDate: d.allDay ? dayKey(start) : s.date,
    startMin: d.allDay ? 0 : s.min,
    endDate: d.allDay ? dayKey(end - 1) : e.date,
    endMin: d.allDay ? 0 : e.min,
    attendees: others.map((a) => ({ name: a.name, email: a.email })),
    reminder: d.reminderMinutes ?? -1,
    busyStatus: d.busyStatus,
    isPrivate: d.isPrivate,
    importance: d.importance,
    recurrence: d.recurrence ?? null,
    meeting: d.isMeeting,
    categories: d.categories ?? [],
    timeZone,
  };
}

function toMs(f: Form) {
  if (f.allDay) return { start: parseDayKey(f.startDate).getTime(), end: plusDays(parseDayKey(f.endDate), 1).getTime() };
  return { start: epochOf(f.startDate, f.startMin, f.timeZone), end: epochOf(f.endDate, f.endMin, f.timeZone) };
}

/** "Name <a@b>; c@d" → attendees, names resolved against the ones already known. */
function parseAddrs(text: string, known: Addr[]): Addr[] {
  return parseAddresses(text).map((a) => {
    if (a.email.includes("@")) return a;
    const k = known.find((x) => x.name.toLowerCase() === a.email.toLowerCase());
    return k ?? a;
  });
}

const RESP_LABEL: Record<string, string> = { accepted: "Accepted", tentative: "Tentative", declined: "Declined", needsAction: "None" };

export default function AppointmentWindow() {
  const params = useMemo(() => windowParams(), []);
  const id = params.get("id") ? Number(params.get("id")) : null;
  const settings = useCalSettings();
  const cals = useRpc<CalendarInfo[]>(["calendar", "calendars"], "calendar.listCalendars");
  const [scope, setScope] = useState<Scope | null>((params.get("scope") as Scope) || null);
  const [detail, setDetail] = useState<EventDetail | null>(null);
  const [form, setForm] = useState<Form | null>(null);
  const [initial, setInitial] = useState<string>("");
  const [recips, setRecips] = useState<Recipient[]>([]);
  const [showNames, setShowNames] = useState(false);
  // Time Zones: the zone list beside the start time (on by itself for an item set in another zone)
  const [showZones, setShowZones] = useState(false);
  // labelled with each zone's offset on the item's day (New York is UTC-5 in December, -4 in June)
  const zoneNow = form?.timeZone;
  const zoneDay = form?.startDate;
  const zones = useMemo(() => (showZones ? zoneList(zoneDay ? epochOf(zoneDay, 12 * 60, "UTC") : Date.now(), zoneNow ? [zoneNow] : []) : []), [showZones, zoneNow, zoneDay]);
  const [busy, setBusy] = useState(false);
  const closing = useRef(false);
  const subjectRef = useRef<HTMLInputElement>(null);

  // ---- load --------------------------------------------------------------------------------
  // like the last meeting window you closed, but never maximized (it opened hidden: windows.ts)
  useEffect(() => {
    void showAtNormalSize();
  }, []);
  useEffect(() => {
    (async () => {
      if (id === null) {
        const now = nextSlot();
        const start = params.get("start") ? Number(params.get("start")) : now;
        const allDay = params.get("allDay") === "1";
        const end = params.get("end") ? Number(params.get("end")) : allDay ? plusDays(start, 1).getTime() : start + 30 * MIN;
        const f: Form = {
          summary: params.get("summary") ?? "",
          location: "",
          description: "",
          allDay,
          startDate: dayKey(start),
          startMin: allDay ? 0 : minutesOf(start),
          endDate: dayKey(allDay ? Math.max(start, end - 1) : end),
          endMin: allDay ? 0 : minutesOf(end),
          attendees: [],
          reminder: allDay ? 18 * 60 : -2, // -2 = "use the Options default" (resolved below)
          busyStatus: allDay ? "free" : "busy",
          isPrivate: false,
          importance: "normal",
          recurrence: null,
          meeting: params.get("meeting") === "1" || !!params.get("to"),
          categories: [],
          timeZone: localZone(),
        };
        // side by side: the calendar whose columns the slot was picked in
        if (params.get("calendarId")) f.calendarId = Number(params.get("calendarId"));
        // New Meeting Request to Contact: attendees come in the `to` param ("Name <a@b>; c@d")
        const to = params.get("to");
        if (to) {
          f.attendees = parseAddrs(to, []);
          setRecips(toRecipients(f.attendees));
        }
        setForm(f);
        setInitial(JSON.stringify(f));
        return;
      }
      try {
        let d = await cal.getEvent(id);
        let sc = scope;
        if (!sc) sc = d.isMaster ? "series" : d.isRecurring ? await askScope(d, "open") : "occurrence";
        if (!sc) {
          closing.current = true;
          closeCurrentWindow();
          return;
        }
        if (sc === "series" && !d.isMaster && d.seriesId) d = { ...(await cal.getEvent(d.seriesId)), seriesStart: d.seriesStart, seriesEnd: d.seriesEnd };
        setScope(sc);
        setDetail(d);
        const f = fromDetail(d, sc === "series");
        if (!d.allDay && f.timeZone !== localZone()) setShowZones(true);
        setForm(f);
        setRecips(toRecipients(f.attendees));
        setInitial(JSON.stringify(f));
      } catch (e) {
        await errorBox(e);
        closing.current = true;
        closeCurrentWindow();
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // resolve the default reminder for new appointments once settings are in
  useEffect(() => {
    if (form && form.reminder === -2) {
      const f = { ...form, reminder: settings.defaultReminderMinutes };
      setForm(f);
      setInitial(JSON.stringify(f));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [form?.reminder, settings.defaultReminderMinutes]);

  const isNew = id === null;
  const organizer = isNew || !!detail?.isOrganizer;
  const meeting = !!form && (form.meeting || form.attendees.length > 0);
  /** invitations already went out (a saved meeting), as opposed to attendees just being added */
  const sentMeeting = !!detail?.isMeeting;
  const toRef = useRef<RecipientFieldHandle>(null);
  const setAttendees = (list: Recipient[]) => {
    setRecips(list);
    setForm((f) => (f ? { ...f, attendees: list.filter((a) => !a.suggested).map((a) => ({ name: a.name, email: a.email })) } : f));
  };
  /** the attendee list including whatever is still being typed in the To box (a usual group's
   *  people only once kept) */
  const attendeeList = (): Addr[] => (toRef.current?.commit() ?? recips).filter((a) => !a.suggested).map((a) => ({ name: a.name, email: a.email }));
  // Tab after a name: the people usually with them (usualGroup.ts), all into this one box
  const group = useUsualGroup({ to: recips, setTo: setAttendees, next: () => subjectRef.current?.focus() });

  const prefilledTo = !!params.get("to");
  const readOnly = !!detail?.readOnly || (!!detail && detail.isMeeting && !detail.isOrganizer);
  const series = scope === "series";
  const kind = meeting ? "Meeting" : form?.allDay ? "Event" : "Appointment";
  const title = `${form?.summary || "Untitled"} - ${kind}`;
  useEffect(() => {
    setWindowTitle(title);
  }, [title]);
  const dirty = !!form && JSON.stringify(form) !== initial;
  // an invitation can't be edited, but your own subject / reminder / Show As / Private on it can
  // (like Outlook: the subject is renamed on your calendar only)
  const inviteEditable = !!detail && !detail.readOnly && detail.isMeeting && !detail.isOrganizer;
  const init0 = useMemo(() => (initial ? (JSON.parse(initial) as Form) : null), [initial]);
  const catsDirty = !!form && !!init0 && JSON.stringify(form.categories) !== JSON.stringify(init0.categories ?? []);
  const inviteDirty =
    (inviteEditable &&
      !!form &&
      !!init0 &&
      (form.reminder !== init0.reminder || form.busyStatus !== init0.busyStatus || form.isPrivate !== init0.isPrivate || form.summary !== init0.summary)) ||
    (readOnly && catsDirty);
  const catList = useCategories();

  const set = (p: Partial<Form>) => setForm((f) => (f ? { ...f, ...p } : f));
  // tables in the notes (../notesTable.ts): a pasted table can be tidied into a list (reads well
  // on a phone) or a fixed-width grid, which ClassicMail shows as a real table
  const notesRef = useRef<HTMLTextAreaElement>(null);
  const [editingNotes, setEditingNotes] = useState(false);
  const [showResponses, setShowResponses] = useState(false);
  // "converted" message with its Undo, shown while the notes are still exactly as converted
  const [notesInfo, setNotesInfo] = useState<{ text: string; notes: string; before: string } | null>(null);
  const tidyTable = (style: TidyStyle) => {
    if (!form) return;
    const before = form.description;
    const r = tidyNotes(before, style);
    if (!r.count) return;
    set({ description: r.text });
    const what = style === "grid" ? "a grid (shown as a table here)" : "a list for phone viewing";
    setNotesInfo({ text: `Table${r.count === 1 ? "" : "s"} converted to ${what}.`, notes: r.text, before });
    setEditingNotes(false);
  };
  const editNotes = () => {
    setEditingNotes(true);
    setTimeout(() => notesRef.current?.focus(), 0);
  };

  // keep the duration when the start moves (Outlook behaviour)
  const setStart = (date: string, min: number) => {
    if (!form) return;
    // an all-day item keeps its number of days, not its hours (DST days have 23 or 25 hours)
    if (form.allDay) return set({ startDate: date, startMin: min, endDate: sameLengthEnd(form.startDate, form.endDate, date), endMin: 0 });
    const old = toMs(form);
    const ne = wallOf(epochOf(date, min, form.timeZone) + (old.end - old.start), form.timeZone);
    set({ startDate: date, startMin: min, endDate: ne.date, endMin: ne.min });
  };

  // ---- conflicts ----------------------------------------------------------------------------
  const range = form ? toMs(form) : null;
  const conflicts = useRpc<CalEvent[]>(
    ["calendar", "conflicts", range?.start, range?.end],
    "calendar.listEvents",
    { start: range?.start ?? 0, end: range?.end ?? 0 },
    { enabled: !!range && !readOnly },
  );
  const conflict =
    scope !== "series" && !form?.allDay && form?.busyStatus !== "free" && (conflicts.data ?? []).some((e) => e.id !== detail?.id && e.busyStatus !== "free" && !e.allDay);

  // ---- recurrence dialog round trip ---------------------------------------------------------
  const recReq = useRef<string | null>(null);
  useEffect(() => {
    const un = listen<{ req: string; action: string; recurrence?: string[]; start?: number; end?: number; allDay?: boolean }>("cm-recurrence-result", ({ payload }) => {
      if (payload.req !== recReq.current) return;
      recReq.current = null;
      if (payload.action === "remove") set({ recurrence: [] });
      else if (payload.action === "ok" && payload.recurrence && payload.start != null && payload.end != null) {
        const allDay = !!payload.allDay;
        setForm((f) => {
          if (!f) return f;
          const s = wallOf(payload.start!, f.timeZone);
          const e = wallOf(payload.end!, f.timeZone);
          return {
            ...f,
            recurrence: payload.recurrence!,
            allDay,
            startDate: allDay ? dayKey(payload.start!) : s.date,
            startMin: allDay ? 0 : s.min,
            endDate: allDay ? dayKey(payload.end! - 1) : e.date,
            endMin: allDay ? 0 : e.min,
          };
        });
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, []);
  const openRecurrence = () => {
    if (!form) return;
    const req = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
    recReq.current = req;
    const { start, end } = toMs(form);
    const me = getCurrentWebviewWindow().label;
    openWindow(
      "recurrence",
      { req, start, end, allDay: form.allDay ? 1 : 0, rule: form.recurrence?.length ? form.recurrence.join("\n") : undefined, owner: me },
      { title: "Appointment Recurrence", label: `recurrence-${me}`, resizable: false, width: 540, height: 385, alwaysOnTop: true },
    );
  };

  // ---- save ---------------------------------------------------------------------------------
  const buildInput = (send: boolean): EventInput => {
    const f = form!;
    const { start, end } = toMs(f);
    const input: EventInput = {
      summary: f.summary,
      location: f.location,
      description: detail && isHtmlNotes(detail.description) && f.description === notesText(detail.description) ? detail.description : f.description,
      allDay: f.allDay,
      reminderMinutes: f.reminder < 0 ? -1 : f.reminder,
      busyStatus: f.busyStatus,
      isPrivate: f.isPrivate,
      importance: f.importance,
      sendUpdates: send,
      categories: f.categories,
    };
    if (f.allDay) {
      input.startDate = f.startDate;
      input.endDate = dayKey(plusDays(parseDayKey(f.endDate), 1));
    } else {
      input.start = start;
      input.end = end;
      input.timeZone = f.timeZone;
    }
    if (organizer) input.attendees = meeting ? attendeeList() : [];
    const before = init0?.recurrence ?? null;
    if (JSON.stringify(before) !== JSON.stringify(f.recurrence) && f.recurrence) input.recurrence = f.recurrence;
    if (isNew) input.calendarId = writable.some((c) => c.id === f.calendarId) ? f.calendarId : defaultCal?.id;
    return input;
  };

  const validate = async (): Promise<boolean> => {
    const f = form!;
    const { start, end } = toMs(f);
    if (end < start) {
      await messageBox({ title: "ClassicMail", text: "The end date you entered occurs before the start date.", icon: "warning", buttons: ["OK"] });
      return false;
    }
    if (meeting && organizer) {
      const list = attendeeList();
      const bad = list.find((a) => !isValidEmail(a.email));
      if (bad) {
        await messageBox({ title: "ClassicMail", text: `ClassicMail does not recognize "${bad.email}".`, icon: "warning", buttons: ["OK"] });
        return false;
      }
    }
    return true;
  };

  const save = async (send: boolean, close = true): Promise<boolean> => {
    if (!form || busy) return false;
    if (readOnly) {
      if (inviteDirty && detail) {
        try {
          // categories are allowed on every item (also Gmail-created ones Google won't let us edit)
          if (catsDirty) await cal.setCategories([detail.id], form.categories, form.categories.length ? `Categorize "${detail.summary || "Untitled"}"` : "Clear Categories");
          // the window then shows the organizer's subject under yours
          if (inviteEditable && init0 && form.summary !== init0.summary) setDetail(await cal.setOwnSummary(detail.id, form.summary, `Rename "${detail.summary || "Untitled"}"`));
          if (inviteEditable)
            await cal.update(
              detail.id,
              { reminderMinutes: form.reminder < 0 ? -1 : form.reminder, busyStatus: form.busyStatus, isPrivate: form.isPrivate },
              series ? "series" : "occurrence",
            );
          setInitial(JSON.stringify(form));
        } catch (e) {
          errorBox(e);
          return false;
        }
      }
      if (close) doClose();
      return true;
    }
    if (!(await validate())) return false;
    setBusy(true);
    try {
      const input = buildInput(send);
      if (isNew) {
        const d = await cal.create(input, `New ${meeting ? "Meeting" : "Appointment"} "${form.summary || "Untitled"}"`);
        if (!close) {
          setDetail(d);
          setScope("occurrence");
        }
      } else {
        await cal.update(detail!.id, input, series ? "series" : "occurrence", `Edit "${form.summary || detail!.summary || "Untitled"}"`);
      }
      setInitial(JSON.stringify(form));
      if (close) doClose();
      return true;
    } catch (e) {
      errorBox(e);
      return false;
    } finally {
      setBusy(false);
    }
  };

  const doClose = () => {
    closing.current = true;
    closeCurrentWindow();
  };

  const askClose = async () => {
    if (readOnly ? !inviteDirty : !dirty) return doClose();
    const r = await messageBox({ title: "ClassicMail", text: "Do you want to save changes?", icon: "warning", buttons: ["Yes", "No", "Cancel"], defaultButton: "Yes" });
    if (r === "Yes") save(false);
    else if (r === "No") doClose();
  };
  const askRef = useRef(askClose);
  askRef.current = askClose;
  const dirtyRef = useRef(dirty);
  dirtyRef.current = (dirty && !readOnly) || inviteDirty;
  useEffect(() => {
    const un = getCurrentWebviewWindow().onCloseRequested((e) => {
      if (closing.current || !dirtyRef.current) return;
      e.preventDefault();
      askRef.current();
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  const del = async () => {
    if (isNew) return doClose();
    if (!detail) return;
    const ev = { ...detail, isRecurring: series ? false : detail.isRecurring };
    if (series) {
      if (!(await confirmBox(`Delete all occurrences of "${detail.summary || "Untitled"}"?`, "Confirm Delete", "warning", "calendar.deleteSeries"))) return;
      try {
        await cal.remove(detail.id, "series", detail.isMeeting && detail.isOrganizer, `Delete "${detail.summary || "Untitled"}"`);
        doClose();
      } catch (e) {
        errorBox(e);
      }
      return;
    }
    if (await deleteItem(ev)) doClose();
  };

  const respond = async (r: "accepted" | "tentative" | "declined") => {
    if (!detail) return;
    if (inviteDirty && !(await save(false, false))) return;
    await respondTo({ ...detail, isRecurring: series ? false : detail.isRecurring }, r);
    doClose();
  };

  const propose = async (r: "tentative" | "declined") => {
    if (!detail) return;
    if (inviteDirty && !(await save(false, false))) return;
    await proposeNewTime({ ...detail, isRecurring: series ? false : detail.isRecurring }, r);
    doClose();
  };

  // the saved item on paper (not the window)
  const print = () => (detail ? quickPrint(detail.id) : window.print());

  useHotkeys(
    {
      "ctrl+s": () => save(false, false),
      "alt+s": () => save(meeting && organizer),
      escape: () => askClose(),
      "ctrl+d": () => del(),
      "ctrl+g": () => !readOnly && !(scope === "occurrence" && detail?.isRecurring) && openRecurrence(),
      "ctrl+k": () => checkNames(),
      "ctrl+p": () => print(),
    },
    { allowInInputs: true },
  );

  // ---- attendees ----------------------------------------------------------------------------
  const checkNames = async () => {
    if (!form) return;
    const list = toRef.current?.commit() ?? recips;
    const resolved: Recipient[] = [];
    for (const a of list) {
      if (isValidEmail(a.email)) {
        resolved.push({ ...a, resolved: true });
        continue;
      }
      try {
        const r = await rpc<{ name: string; email: string }[]>("contacts.autocomplete", { query: a.name || a.email, limit: 1 });
        resolved.push(r[0] ? { name: r[0].name, email: r[0].email, resolved: true } : a);
      } catch {
        resolved.push(a);
      }
    }
    setAttendees(resolved);
  };

  // a new meeting starts in the To box (unless the attendees came with it)
  // the notes' tables and links, worked out again only when the notes change (not on every keystroke)
  const description = form?.description ?? "";
  const parts = useMemo(() => notesParts(description), [description]);
  const kinds = useMemo(() => tableKinds(description), [description]);

  const formReady = !!form;
  useEffect(() => {
    if (formReady && isNew && meeting && !prefilledTo) setTimeout(() => toRef.current?.focus(), 0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [formReady]);

  const writable = (cals.data ?? []).filter((c) => c.canWrite);
  const defaultCal = writable.find((c) => c.isPrimary) ?? writable[0];

  if (!form) {
    return (
      <div className="cm-itemwin">
        <div className="cm-empty">Loading…</div>
        <CalDialogHost />
      </div>
    );
  }

  const ms = toMs(form);
  const past = ms.end < Date.now();
  // the series' pattern, as Outlook words it: its first date and its times, even on a moved occurrence
  const seriesSpan = !series && detail?.seriesStart != null && detail.seriesEnd != null ? { start: detail.seriesStart, end: detail.seriesEnd } : ms;
  const recurrenceText = form.recurrence?.length ? describeRecurrence(form.recurrence, seriesSpan.start, seriesSpan.end, form.allDay) : "";
  const counts = (detail?.attendees ?? []).filter((a) => !a.organizer).reduce<Record<string, number>>((m, a) => ((m[a.response] = (m[a.response] ?? 0) + 1), m), {});
  const notesTables = parts.some((x) => x.kind === "table");
  // tables show as real tables, links as links; a click on the rest edits the text
  const notesView = notesTables || hasUrl(form.description) ? parts : null;
  const locationUrl = firstUrl(form.location);
  const infoLines: { icon: string; text: string; actions?: { label: string; onClick: () => void }[] }[] = [];
  if (meeting && organizer && !isNew && detail?.attendees.length) {
    const got = (counts.accepted ?? 0) + (counts.tentative ?? 0) + (counts.declined ?? 0);
    infoLines.push({
      icon: "info",
      text: got
        ? `${counts.accepted ?? 0} attendee${counts.accepted === 1 ? "" : "s"} accepted, ${counts.tentative ?? 0} tentatively accepted, ${counts.declined ?? 0} declined.`
        : "No responses have been received for this meeting.",
      actions: [{ label: "Responses…", onClick: () => setShowResponses(true) }],
    });
  }
  if (meeting && organizer && !sentMeeting) infoLines.push({ icon: "info", text: "Invitations have not been sent for this meeting." });
  if (detail && !organizer && detail.isMeeting) {
    const r = detail.myResponse;
    infoLines.push({
      icon: r === "needsAction" ? "warning" : r === "declined" ? "decline" : r === "tentative" ? "tentative" : "accept",
      text:
        r === "needsAction" || !r
          ? "Please respond."
          : r === "accepted"
            ? "You accepted this meeting."
            : r === "tentative"
              ? "You tentatively accepted this meeting."
              : "You declined this meeting.",
    });
  }
  if (detail?.originalSummary) infoLines.push({ icon: "info", text: `You renamed this meeting on your calendar. The organizer's subject: "${detail.originalSummary}".` });
  if (detail?.readOnly) infoLines.push({ icon: "info", text: `This item is on the read-only calendar "${detail.calendarName}".` });
  if (form && !form.allDay && range && form.timeZone !== localZone()) {
    const t = (ms: number) => {
      const w = wallOf(ms, localZone());
      return `${String(Math.floor(w.min / 60)).padStart(2, "0")}:${String(w.min % 60).padStart(2, "0")}`;
    };
    const day = wallOf(range.start, localZone()).date !== form.startDate ? ` on ${fmtLongDate(range.start)}` : "";
    infoLines.push({
      icon: "time-zones",
      text: `Times are in ${zoneName(form.timeZone)}: ${t(range.start)}–${t(range.end)}${day} in your time (${zoneName(localZone())}).`,
    });
  }
  if (conflict && !readOnly) infoLines.push({ icon: "warning", text: "Conflicts with another appointment on your Calendar." });
  if (past && !recurrenceText) infoLines.push({ icon: "info", text: `This ${kind.toLowerCase()} occurs in the past.` });
  if (recurrenceText && !series) infoLines.push({ icon: "recurring", text: recurrenceText });
  if (detail?.pending) infoLines.push({ icon: "send-receive", text: "Changes to this item will be sent to Google at the next Send/Receive." });
  // tables in the notes: the choice is worked out from the text itself, so it is there whenever the
  // item is opened (a pasted table → list or grid; a grid ↔ a list that has its "Table header:")
  if (!readOnly) {
    const justConverted = !!notesInfo && notesInfo.notes === form.description;
    const toList = { label: "Convert to list (phone)", onClick: () => tidyTable("list") };
    const toGrid = { label: "Convert to grid (desktop)", onClick: () => tidyTable("grid") };
    const actions: { label: string; onClick: () => void }[] = [];
    let text = "";
    if (kinds.raw) {
      text = "These notes contain a table that won't read well everywhere.";
      actions.push({ label: "Tidy as list (phone)", onClick: () => tidyTable("list") }, { label: "Tidy as grid (desktop)", onClick: () => tidyTable("grid") });
    } else if (kinds.grid || kinds.list) {
      text = kinds.grid ? "The table in the notes is a grid (shown as a table here)." : "The table in the notes is a list (reads well on a phone).";
      actions.push(kinds.grid ? toList : toGrid);
      if (kinds.grid && kinds.list) actions.push(toGrid);
    }
    if (justConverted && notesInfo) {
      const before = notesInfo.before;
      text = notesInfo.text;
      actions.unshift({
        label: "Undo",
        onClick: () => {
          set({ description: before });
          setNotesInfo(null);
        },
      });
    }
    if (text) infoLines.push({ icon: "info", text, actions });
  }

  const invitee = !!detail?.isMeeting && !detail.isOrganizer;
  const recurrenceDisabled = readOnly || (scope === "occurrence" && !!detail?.isRecurring);

  const appointmentTab = {
    id: "appt",
    label: meeting ? "Meeting" : "Appointment",
    groups: [
      {
        label: "Actions",
        content: (
          <>
            {readOnly && !organizer ? (
              <>
                <RibbonBig icon="accept" label="Accept" onClick={() => respond("accepted")} disabled={detail?.readOnly} />
                <RibbonBig icon="tentative" label="Tentative" onClick={() => respond("tentative")} disabled={detail?.readOnly} />
                <RibbonBig icon="decline" label="Decline" onClick={() => respond("declined")} disabled={detail?.readOnly} />
              </>
            ) : (
              <RibbonBig icon="save-close" label="Save & Close" onClick={() => save(false)} disabled={busy} title="Save & Close (Alt+S)" />
            )}
            <RibbonStack>
              <RibbonSmall icon="delete" label={sentMeeting && organizer ? "Cancel Meeting" : "Delete"} onClick={del} disabled={!!detail?.readOnly} />
              <RibbonSmall icon="print" label="Print" onClick={print} />
              {!readOnly && <RibbonSmall icon="save" label="Save" onClick={() => save(false, false)} />}
            </RibbonStack>
          </>
        ),
      },
      ...(detail && !isNew
        ? [
            {
              label: "Respond",
              content: (
                <>
                  {invitee && (
                    <RibbonBig
                      icon="propose-time"
                      label="Propose New Time"
                      disabled={detail.readOnly}
                      menu={() => [
                        { label: "Tentative and Propose New Time…", onClick: () => propose("tentative") },
                        { label: "Decline and Propose New Time…", onClick: () => propose("declined") },
                      ]}
                    />
                  )}
                  <RibbonStack>
                    {detail.isMeeting && <RibbonSmall icon="reply" label="Reply" onClick={() => mailAboutItem(detail, "reply")} />}
                    <RibbonSmall icon="forward" label="Forward" onClick={() => mailAboutItem(detail, "forward")} />
                  </RibbonStack>
                </>
              ),
            },
          ]
        : []),
      ...(organizer
        ? [
            {
              label: "Attendees",
              content: (
                <>
                  <RibbonBig
                    icon={meeting ? "close" : "meeting-request"}
                    label={meeting ? "Cancel Invitation" : "Invite Attendees"}
                    disabled={readOnly || (meeting && sentMeeting)}
                    onClick={() => {
                      if (meeting) {
                        setAttendees([]);
                        set({ meeting: false });
                      } else {
                        set({ meeting: true });
                        setTimeout(() => toRef.current?.focus(), 0);
                      }
                    }}
                  />
                  {meeting && (
                    <RibbonStack>
                      <RibbonSmall icon="address-book" label="Address Book" onClick={() => setShowNames(true)} />
                      <RibbonSmall icon="check-names" label="Check Names" onClick={checkNames} />
                      <RibbonSmall icon="accept" label="Responses" disabled={isNew || !detail?.attendees.length} onClick={() => setShowResponses(true)} />
                    </RibbonStack>
                  )}
                </>
              ),
            },
          ]
        : []),
      {
        label: "Options",
        content: (
          <>
            <div className="cm-cal-rb-fields">
              <label>
                <Icon name="appointment" size={14} />
                <span>Show As:</span>
                <select className="cm-input" value={form.busyStatus} disabled={readOnly && !!detail?.readOnly} onChange={(e) => set({ busyStatus: e.target.value as BusyStatus })}>
                  {SHOW_AS.map((s) => (
                    <option key={s.value} value={s.value}>
                      {s.label}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                <Icon name="reminder" size={14} />
                <span>Reminder:</span>
                <select className="cm-input" value={form.reminder} disabled={!!detail?.readOnly} onChange={(e) => set({ reminder: Number(e.target.value) })}>
                  <option value={-1}>None</option>
                  {[...new Set([...REMINDER_CHOICES, ...(form.reminder >= 0 ? [form.reminder] : [])])]
                    .sort((a, b) => a - b)
                    .map((m) => (
                      <option key={m} value={m}>
                        {fmtReminder(m)}
                      </option>
                    ))}
                </select>
              </label>
            </div>
            <RibbonBig icon="recurring" label="Recurrence" pressed={!!form.recurrence?.length} disabled={recurrenceDisabled} onClick={openRecurrence} title="Recurrence (Ctrl+G)" />
            <RibbonBig
              icon="time-zones"
              label="Time Zones"
              pressed={showZones}
              disabled={form.allDay}
              title="Set the start and end times in another time zone (for example a meeting in New York)"
              onClick={() => setShowZones(!showZones)}
            />
            <RibbonBig
              icon={`cm-cat-${catList.find((c) => form.categories[0] && c.name.toLowerCase() === form.categories[0].toLowerCase())?.color ?? 0}`}
              label="Categorize"
              title="Categorize"
              menu={() => categorizeMenu(form.categories, catList, (names) => set({ categories: names }))}
            />
            <RibbonStack>
              <RibbonSmall icon="private" label="Private" pressed={form.isPrivate} disabled={!!detail?.readOnly} onClick={() => set({ isPrivate: !form.isPrivate })} />
              <RibbonSmall
                icon="importance-high"
                label="High Importance"
                pressed={form.importance === "high"}
                disabled={readOnly}
                onClick={() => set({ importance: form.importance === "high" ? "normal" : "high" })}
              />
              <RibbonSmall
                icon="importance-low"
                label="Low Importance"
                pressed={form.importance === "low"}
                disabled={readOnly}
                onClick={() => set({ importance: form.importance === "low" ? "normal" : "low" })}
              />
            </RibbonStack>
          </>
        ),
      },
      ...(detail?.htmlLink
        ? [
            {
              label: "Google",
              content: (
                <RibbonBig
                  icon="globe"
                  label="Open in Google"
                  title="Open this item in Google Calendar on the web"
                  onClick={() => rpc("app.openUrl", { url: detail.htmlLink }).catch(errorBox)}
                />
              ),
            },
          ]
        : []),
    ],
  };
  const insertTab = {
    id: "insert",
    label: "Insert",
    groups: [
      {
        label: "Include",
        content: (
          <>
            <RibbonBig icon="attach-file" label="Attach File" disabled title="Attachments on calendar items aren't supported yet" />
            <RibbonStack>
              <RibbonSmall
                icon="calendar"
                label="Date & Time"
                disabled={readOnly}
                onClick={() =>
                  set({ description: `${form.description}${form.description && !form.description.endsWith("\n") ? "\n" : ""}${fmtLongDate(Date.now())} ${fmtTime(Date.now())}` })
                }
              />
            </RibbonStack>
          </>
        ),
      },
    ],
  };

  const sendButton = meeting && organizer && !readOnly;
  return (
    <div className="cm-itemwin cm-cal-apptwin">
      <Ribbon
        tabs={[appointmentTab, insertTab]}
        orbMenu={[
          { label: "Save", icon: "save", shortcut: "Ctrl+S", disabled: readOnly, onClick: () => save(false, false) },
          { label: "Save & Close", icon: "save-close", shortcut: "Alt+S", onClick: () => save(false) },
          { label: "Delete", icon: "delete", shortcut: "Ctrl+D", disabled: !!detail?.readOnly, onClick: del },
          { type: "separator" },
          { label: "Print", icon: "print", onClick: print },
          { type: "separator" },
          { label: "Close", icon: "close", onClick: askClose },
        ]}
        quickAccess={
          <button className="cm-cal-qat" title="Save (Ctrl+S)" onClick={() => save(false, false)} disabled={readOnly}>
            <Icon name="save" size={14} />
          </button>
        }
      />
      {form.categories.length > 0 && (
        <div className="cm-cal-catbar">
          {form.categories.map((n) => (
            <span key={n} className="cm-cal-catchip" style={{ background: categoryColor(n, catList) ?? "#fff" }}>
              {n}
            </span>
          ))}
        </div>
      )}
      {infoLines.length > 0 && (
        <div className="cm-cal-infobar">
          {infoLines.map((l, i) => (
            <div key={i}>
              <Icon name={l.icon} size={14} />
              <span>{l.text}</span>
              {l.actions?.map((a) => (
                <button key={a.label} className="cm-linkbutton" onClick={a.onClick}>
                  {a.label}
                </button>
              ))}
            </div>
          ))}
        </div>
      )}
      <div className={`cm-cal-apptfields${sendButton ? " with-send" : ""}`}>
        {sendButton && (
          <button className="cm-cal-send" onClick={() => save(true)} disabled={busy} title="Send (Alt+S)">
            <Icon name="send" size={28} />
            <span>{sentMeeting ? "Send Update" : "Send"}</span>
          </button>
        )}
        <div className="cm-itemwin-fields">
          {!organizer && detail && (
            <>
              <span className="cm-label">Organizer:</span>
              <span className="cm-ellipsis">
                <AddressTokens list={[{ name: detail.organizerName ?? "", email: detail.organizerEmail ?? "" }]} showEmail />
              </span>
              <span className="cm-label">Required:</span>
              <span className="cm-ellipsis" title={detail.attendees.map((a) => `${a.name || a.email}: ${RESP_LABEL[a.response]}`).join("\n")}>
                <AddressTokens list={detail.attendees.filter((a) => !a.organizer)} />
              </span>
            </>
          )}
          {meeting && organizer && (
            <>
              <button className="cm-field-label-btn" onClick={() => setShowNames(true)} disabled={readOnly}>
                To...
              </button>
              <div className="cm-cal-to">
                <RecipientField ref={toRef} label="To" value={recips} onChange={setAttendees} group={group} />
              </div>
            </>
          )}
          <span className="cm-label">Subject:</span>
          <input
            ref={subjectRef}
            className="cm-input"
            value={form.summary}
            readOnly={readOnly && !inviteEditable}
            title={inviteEditable ? "Renames this meeting on your calendar only" : undefined}
            onChange={(e) => set({ summary: e.target.value })}
            autoFocus={!meeting || !isNew || prefilledTo}
          />
          <span className="cm-label">Location:</span>
          <div className="cm-cal-location">
            <input className="cm-input" value={form.location} readOnly={readOnly} onChange={(e) => set({ location: e.target.value })} />
            {locationUrl && (
              <button className="cm-button" title={`Open ${locationUrl}`} onClick={() => openLink(locationUrl)}>
                Join
              </button>
            )}
          </div>
          {isNew && writable.length > 1 && (
            <>
              <span className="cm-label">Calendar:</span>
              <select className="cm-input" style={{ width: 300 }} value={form.calendarId ?? defaultCal?.id} onChange={(e) => set({ calendarId: Number(e.target.value) })}>
                {writable.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.isPrimary ? `Calendar (${c.accountEmail})` : `${c.name} (${c.accountEmail})`}
                  </option>
                ))}
              </select>
            </>
          )}
        </div>
      </div>
      <div className="cm-cal-times">
        {series ? (
          <div className="cm-cal-series-times">
            <Icon name="recurring" size={14} />
            <span>{recurrenceText || "Recurring"}</span>
          </div>
        ) : (
          <>
            <span className="cm-label">Start time:</span>
            <div className="cm-cal-timerow">
              <DateField value={form.startDate} disabled={readOnly} onChange={(d) => setStart(d, form.startMin)} />
              {!form.allDay && <TimeField value={form.startMin} disabled={readOnly} onChange={(m) => setStart(form.startDate, m)} />}
              {!form.allDay && showZones && (
                <select
                  className="cm-input cm-cal-tz"
                  value={form.timeZone}
                  disabled={readOnly}
                  title="The start and end times are in this time zone; the calendar shows them in yours"
                  onChange={(e) => set({ timeZone: e.target.value })}
                >
                  {zones.map((z) => (
                    <option key={z.tz} value={z.tz}>
                      {z.label}
                    </option>
                  ))}
                </select>
              )}
              <label className="cm-cal-allday">
                <input
                  type="checkbox"
                  checked={form.allDay}
                  disabled={readOnly}
                  onChange={(e) => {
                    const allDay = e.target.checked;
                    if (allDay)
                      set({
                        allDay,
                        endDate: form.endDate < form.startDate ? form.startDate : form.endDate,
                        busyStatus: form.busyStatus === "busy" ? "free" : form.busyStatus,
                        reminder: form.reminder === settings.defaultReminderMinutes ? 18 * 60 : form.reminder,
                      });
                    else {
                      const s = Math.max(settings.workDayStart, 8 * 60);
                      set({ allDay, startMin: s, endMin: s + 30, endDate: form.startDate, busyStatus: form.busyStatus === "free" ? "busy" : form.busyStatus });
                    }
                  }}
                />
                All day event
              </label>
            </div>
            <span className="cm-label">End time:</span>
            <div className="cm-cal-timerow">
              <DateField value={form.endDate} disabled={readOnly} onChange={(d) => set({ endDate: d })} />
              {!form.allDay && (
                <TimeField
                  value={form.endMin}
                  disabled={readOnly}
                  durationFrom={form.endDate === form.startDate ? form.startMin : undefined}
                  onChange={(m) => set({ endMin: m })}
                />
              )}
            </div>
          </>
        )}
      </div>
      {/* someone else's meeting with no notes: the video-call details fill the area */}
      {detail?.conference && <ConferenceBox text={detail.conference} full={readOnly && !form.description.trim()} />}
      {detail?.conference && readOnly && !form.description.trim() ? null : notesView && !editingNotes ? (
        <div
          className={`cm-cal-notes cm-cal-notes-view${readOnly ? "" : " editable"}`}
          title={readOnly ? undefined : "Click to edit the notes"}
          onClick={readOnly ? undefined : editNotes}
        >
          {notesView.map((part, i) =>
            part.kind === "text" ? (
              <div key={i} className="cm-cal-notes-text">
                <Linkified text={part.text} />
              </div>
            ) : (
              <table key={i} className="cm-cal-notes-table">
                <thead>
                  <tr>
                    {part.grid[0].map((h, j) => (
                      <th key={j} className={part.numeric[j] ? "num" : undefined}>
                        <Linkified text={h} />
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {part.grid.slice(1).map((r, k) => (
                    <tr key={k}>
                      {r.map((c, j) => (
                        <td key={j} className={part.numeric[j] ? "num" : undefined}>
                          <Linkified text={c} />
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            ),
          )}
        </div>
      ) : (
        <textarea
          ref={notesRef}
          className={`cm-cal-notes${notesTables ? " mono" : ""}`}
          value={form.description}
          readOnly={readOnly}
          onChange={(e) => set({ description: e.target.value })}
          onBlur={() => setEditingNotes(false)}
          spellCheck
        />
      )}
      {showNames && (
        <SelectNamesDialog
          initial={{ to: recips.map((a) => ({ name: a.name, email: a.email })), cc: [], bcc: [] }}
          onOk={(v) => {
            const all = [...v.to, ...v.cc, ...v.bcc];
            setAttendees(toRecipients(all));
            set({ meeting: true });
            setShowNames(false);
          }}
          onClose={() => setShowNames(false)}
        />
      )}
      {showResponses && detail && <ResponsesDialog detail={detail} onClose={() => setShowResponses(false)} />}
      <CalDialogHost />
      <ColorCategoriesHost />
    </div>
  );
}

/** A meeting's Zoom / Meet details from Google (not part of the notes; read-only), links clickable. */
function ConferenceBox({ text, full }: { text: string; full: boolean }) {
  return (
    <div className={`cm-cal-conference${full ? " full" : ""}`} dir="auto">
      <Linkified text={text} />
    </div>
  );
}
