// Outlook 2007 task form: Ribbon (Task tab) + Subject, Start/Due date, Status, Priority,
// % Complete, Reminder, Owner, In folder, Categories and the notes body.
// Params: ?id=<task id> to open one, or none (optionally listId / due / title) for a new task.
import { useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { format } from "date-fns";
import { useRpc } from "../../../api/query";
import { STATUS_LABELS, tasksApi, type Importance, type Task, type TaskList, type TaskStatus } from "../../../api/tasks";
import { setWindowTitle, windowParams } from "../../../api/windows";
import { undoableMany } from "../../../api/undo";
import { Icon, Ribbon, RibbonBig, RibbonSmall, RibbonStack, errorBox, messageBox, useHotkeys, type MenuItem } from "../../../ui";
import { DatePicker } from "../DatePicker";
import { dueInfo, flagDates, parseDayKey, isOverdue, todayKey, dayKey, type FlagKind } from "../dates";
import { CategoryChips } from "../icons";
import { categoryItems, customFlagDialog, printItems } from "../actions";
import { ColorCategoriesHost, useCategories } from "../../calendar/categories";
import { TimeField } from "../../calendar/windows/fields";
import { taskItem } from "../store";
import "../tasks.css";
// the time box's own styles
import "../../calendar/calendar.css";

interface Form {
  title: string;
  notes: string;
  start: string | null;
  due: string | null;
  status: TaskStatus;
  importance: Importance;
  percent: number;
  reminderOn: boolean;
  reminderDate: string | null;
  reminderTime: string; // "HH:mm"
  categories: string[];
  isPrivate: boolean;
  listId: number | null;
}

// the reminder time is kept as "HH:mm"; the calendar's time box (any minute, e.g. 08:15) works in minutes
const hhmmToMinutes = (hhmm: string) => Number(hhmm.slice(0, 2)) * 60 + Number(hhmm.slice(3));
const minutesToHhmm = (m: number) => `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;

function formFromTask(t: Task): Form {
  const r = t.reminderAt ? new Date(t.reminderAt) : null;
  return {
    title: t.title,
    notes: t.notes,
    start: t.start,
    due: t.due,
    status: t.status,
    importance: t.importance,
    percent: t.percent,
    reminderOn: !!r,
    reminderDate: r ? dayKey(r) : null,
    reminderTime: r ? format(r, "HH:mm") : "08:00",
    categories: t.categories,
    isPrivate: t.isPrivate,
    listId: t.listId,
  };
}

function reminderMs(f: Form): number | null {
  if (!f.reminderOn || !f.reminderDate) return null;
  const d = parseDayKey(f.reminderDate);
  d.setHours(Number(f.reminderTime.slice(0, 2)), Number(f.reminderTime.slice(3)), 0, 0);
  return d.getTime();
}

export default function TaskWindow() {
  const params = windowParams();
  const id = params.get("id") ? Number(params.get("id")) : null;
  const lists = useRpc<TaskList[]>(["tasks", "lists"], "tasks.lists");
  const [task, setTask] = useState<Task | null>(null);
  const [form, setForm] = useState<Form | null>(null);
  const [initial, setInitial] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const closing = useRef(false);
  const subjectRef = useRef<HTMLInputElement>(null);

  // load
  useEffect(() => {
    if (id) {
      tasksApi
        .get(id)
        .then((t) => {
          setTask(t);
          const f = formFromTask(t);
          setForm(f);
          setInitial(JSON.stringify(f));
        })
        .catch(async (e) => {
          await errorBox(e);
          getCurrentWebviewWindow().destroy();
        });
    }
  }, [id]);
  useEffect(() => {
    if (id || form || !lists.data) return;
    const want = params.get("listId") ? Number(params.get("listId")) : null;
    const def = lists.data.find((l) => l.id === want) ?? lists.data.find((l) => l.isDefault) ?? lists.data[0];
    const f: Form = {
      title: params.get("title") ?? "",
      notes: "",
      start: null,
      due: params.get("due") ?? null,
      status: "notStarted",
      importance: "normal",
      percent: 0,
      reminderOn: false,
      reminderDate: null,
      reminderTime: "08:00",
      categories: [],
      isPrivate: false,
      listId: def?.id ?? null,
    };
    setForm(f);
    setInitial(JSON.stringify(f));
  }, [id, form, lists.data]); // eslint-disable-line react-hooks/exhaustive-deps

  const dirty = !!form && JSON.stringify(form) !== initial;
  const title = form?.title.trim() || "Untitled";
  useEffect(() => {
    setWindowTitle(`${title} - Task`);
  }, [title]);
  useEffect(() => {
    if (form && !id) subjectRef.current?.focus();
  }, [!!form]); // eslint-disable-line react-hooks/exhaustive-deps

  const set = (p: Partial<Form>) => setForm((f) => (f ? { ...f, ...p } : f));

  // Outlook keeps Status and % Complete in step
  const setStatus = (s: TaskStatus) =>
    setForm((f) => f && { ...f, status: s, percent: s === "completed" ? 100 : s === "notStarted" ? 0 : f.status === "completed" ? 0 : f.percent });
  const setPercent = (n: number) =>
    setForm((f) => {
      if (!f) return f;
      const p = Math.max(0, Math.min(100, Math.round(n)));
      let s = f.status;
      if (p === 100) s = "completed";
      else if (s === "completed") s = p > 0 ? "inProgress" : "notStarted";
      else if (p > 0 && s === "notStarted") s = "inProgress";
      else if (p === 0 && s === "inProgress") s = "notStarted";
      return { ...f, percent: p, status: s };
    });

  const save = async (): Promise<boolean> => {
    if (!form || busy) return false;
    setBusy(true);
    try {
      const fields = {
        title: form.title.trim(),
        notes: form.notes,
        start: form.start,
        due: form.due,
        status: form.status,
        importance: form.importance,
        percent: form.percent,
        reminderAt: reminderMs(form),
        categories: form.categories,
        isPrivate: form.isPrivate,
      };
      if (task) {
        const label = `Edit "${fields.title || "Untitled"}"`;
        let t: Task;
        if (form.listId && form.listId !== task.listId) {
          // edit + move to another folder: one Undo step
          const [, moved] = await undoableMany<unknown>(label, [
            { method: "tasks.update", params: { id: task.id, ...fields } },
            { method: "tasks.move", params: { ids: [task.id], listId: form.listId } },
          ]);
          t = await tasksApi.get((moved as number[])[0] ?? task.id);
        } else t = await tasksApi.update({ id: task.id, ...fields }, label);
        setTask(t);
      } else {
        const t = await tasksApi.create({ listId: form.listId ?? undefined, ...fields }, `New Task "${fields.title || "Untitled"}"`);
        setTask(t);
      }
      setInitial(JSON.stringify(form));
      return true;
    } catch (e) {
      errorBox(e);
      return false;
    } finally {
      setBusy(false);
    }
  };

  const close = () => {
    closing.current = true;
    getCurrentWebviewWindow().destroy();
  };
  const saveAndClose = async () => {
    if (!dirty && task) return close();
    if (!dirty && !task && !form?.title.trim()) return close();
    if (await save()) close();
  };
  const askClose = async () => {
    if (!dirty) return close();
    const r = await messageBox({ title: "ClassicMail", text: "Do you want to save changes?", icon: "warning", buttons: ["Yes", "No", "Cancel"], defaultButton: "Yes" });
    if (r === "Yes") {
      if (await save()) close();
    } else if (r === "No") close();
  };
  const del = async () => {
    if (task) {
      try {
        await tasksApi.remove([task.id], `Delete "${task.title || "Untitled"}"`);
      } catch (e) {
        return errorBox(e);
      }
    }
    close();
  };
  const markComplete = async () => {
    if (!form) return;
    const done = { ...form, status: "completed" as TaskStatus, percent: 100 };
    setForm(done);
    setBusy(true);
    try {
      const { reminderOn: _r, reminderDate: _d, reminderTime: _t, listId, ...rest } = done;
      const fields = { ...rest, title: done.title.trim(), reminderAt: reminderMs(done) };
      if (task) await tasksApi.update({ id: task.id, ...fields }, `Mark "${fields.title || "Untitled"}" Complete`);
      else await tasksApi.create({ listId: listId ?? undefined, ...fields }, `New Task "${fields.title || "Untitled"}"`);
      close();
    } catch (e) {
      errorBox(e);
    } finally {
      setBusy(false);
    }
  };

  // window close button (X) asks to save
  const askRef = useRef(askClose);
  askRef.current = askClose;
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;
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

  useHotkeys(
    {
      "ctrl+s": () => save(),
      "alt+s": () => saveAndClose(),
      escape: () => askClose(),
      "ctrl+d": () => del(),
      "ctrl+p": () => form && task && printItems([taskItem(task)], `${title} - Task`),
    },
    { allowInInputs: true },
  );

  const follow = async (k: FlagKind | "custom") => {
    if (!form) return;
    if (k === "custom") {
      const r = await customFlagDialog(form.start, form.due);
      if (r) set(r);
    } else set(flagDates(k));
  };
  const followMenu: MenuItem[] = [
    { label: "Today", icon: "flag", onClick: () => follow("today") },
    { label: "Tomorrow", icon: "flag", onClick: () => follow("tomorrow") },
    { label: "This Week", icon: "flag", onClick: () => follow("thisWeek") },
    { label: "Next Week", icon: "flag", onClick: () => follow("nextWeek") },
    { label: "No Date", icon: "flag", onClick: () => follow("noDate") },
    { label: "Custom…", icon: "flag", onClick: () => follow("custom") },
    { type: "separator" },
    {
      label: "Add Reminder…",
      icon: "reminder",
      onClick: () => set({ reminderOn: true, reminderDate: form?.reminderDate ?? form?.due ?? todayKey() }),
    },
    { label: "Mark Complete", icon: "flag-complete", onClick: markComplete },
  ];
  // the user's Color Categories; the menu is built when it opens, from the list loaded above
  useCategories();
  const catMenu = (): MenuItem[] => categoryItems(form?.categories ?? [], (names) => set({ categories: names }));

  const owner = useMemo(() => {
    const l = lists.data?.find((x) => x.id === form?.listId);
    return l ? l.accountName || l.accountEmail : "";
  }, [lists.data, form?.listId]);
  const multiAccount = new Set((lists.data ?? []).map((l) => l.accountId)).size > 1;

  if (!form)
    return (
      <div className="cm-itemwin">
        <div className="cm-empty">Loading…</div>
      </div>
    );

  const complete = form.status === "completed";
  const info = dueInfo(form.due, complete, complete ? (task?.completedAt ?? Date.now()) : null);
  const overdue = isOverdue(form.due, complete);

  return (
    <div className="cm-itemwin">
      <Ribbon
        orbMenu={[
          { label: "Save", icon: "save", shortcut: "Ctrl+S", onClick: () => save() },
          { label: "Delete", icon: "delete", shortcut: "Ctrl+D", onClick: del },
          { label: "Print", icon: "print", shortcut: "Ctrl+P", disabled: !task, onClick: () => task && printItems([taskItem(task)], `${title} - Task`) },
          { type: "separator" },
          { label: "Close", icon: "close", onClick: askClose },
        ]}
        quickAccess={
          <button className="cm-rb-small" title="Save (Ctrl+S)" onClick={() => save()} onMouseDown={(e) => e.preventDefault()}>
            <Icon name="save" />
          </button>
        }
        tabs={[
          {
            id: "task",
            label: "Task",
            groups: [
              {
                label: "Actions",
                content: (
                  <>
                    <RibbonBig icon="save-close" label="Save & Close" onClick={saveAndClose} disabled={busy} />
                    <RibbonBig icon="delete-red" label="Delete" onClick={del} />
                    <RibbonStack>
                      <RibbonSmall icon="forward" label="Forward" disabled />
                    </RibbonStack>
                  </>
                ),
              },
              {
                label: "Show",
                content: (
                  <>
                    <RibbonBig icon="task" label="Task" pressed />
                    <RibbonBig icon="info" label="Details" disabled />
                  </>
                ),
              },
              {
                label: "Manage Task",
                content: (
                  <>
                    <RibbonBig icon="complete-task" label="Mark Complete" onClick={markComplete} disabled={busy} />
                    <RibbonBig icon="contact" label="Assign Task" disabled />
                    <RibbonBig icon="send" label="Send Status Report" disabled />
                  </>
                ),
              },
              {
                label: "Options",
                content: (
                  <>
                    <RibbonBig icon="recurring" label="Recurrence" disabled />
                    <RibbonStack>
                      <RibbonSmall icon="categorize" label="Categorize" menu={catMenu} />
                      <RibbonSmall icon="flag" label="Follow Up" menu={followMenu} />
                      <RibbonSmall icon="private" label="Private" pressed={form.isPrivate} onClick={() => set({ isPrivate: !form.isPrivate })} />
                    </RibbonStack>
                    <RibbonStack>
                      <RibbonSmall
                        icon="importance-high"
                        title="High Importance"
                        pressed={form.importance === "high"}
                        onClick={() => set({ importance: form.importance === "high" ? "normal" : "high" })}
                      />
                      <RibbonSmall
                        icon="importance-low"
                        title="Low Importance"
                        pressed={form.importance === "low"}
                        onClick={() => set({ importance: form.importance === "low" ? "normal" : "low" })}
                      />
                    </RibbonStack>
                  </>
                ),
              },
              {
                label: "Proofing",
                content: <RibbonBig icon="spell" label="Spelling" disabled />,
              },
            ],
          },
        ]}
      />
      {info && <div className={`cm-taskwin-info${overdue ? " overdue" : ""}`}>{info}</div>}
      {form.isPrivate && <div className="cm-taskwin-info">This task is marked Private.</div>}
      <div className="cm-taskwin-fields">
        <label className="cm-label" htmlFor="tw-subject">
          Subject:
        </label>
        <input id="tw-subject" ref={subjectRef} className="cm-input wide" value={form.title} onChange={(e) => set({ title: e.target.value })} dir="auto" />

        <span className="cm-label">Start date:</span>
        <DatePicker value={form.start} onChange={(v) => set({ start: v, ...(v && form.due && form.due < v ? { due: v } : {}) })} width={160} />
        <span />
        <span className="cm-label">Status:</span>
        <select className="cm-input" value={form.status} onChange={(e) => setStatus(e.target.value as TaskStatus)} style={{ width: 190 }}>
          {(Object.keys(STATUS_LABELS) as TaskStatus[]).map((s) => (
            <option key={s} value={s}>
              {STATUS_LABELS[s]}
            </option>
          ))}
        </select>

        <span className="cm-label">Due date:</span>
        <DatePicker value={form.due} onChange={(v) => set({ due: v, ...(v && form.start && form.start > v ? { start: v } : {}) })} width={160} />
        <span />
        <span className="cm-label">Priority:</span>
        <div className="cm-taskwin-inline">
          <select className="cm-input" value={form.importance} onChange={(e) => set({ importance: e.target.value as Importance })} style={{ width: 80 }}>
            <option value="low">Low</option>
            <option value="normal">Normal</option>
            <option value="high">High</option>
          </select>
          <span className="cm-label">% Complete:</span>
          <input className="cm-input cm-taskwin-percent" type="number" min={0} max={100} step={25} value={form.percent} onChange={(e) => setPercent(Number(e.target.value))} />
        </div>

        <label className="cm-label cm-taskwin-reminder">
          <input type="checkbox" checked={form.reminderOn} onChange={(e) => set({ reminderOn: e.target.checked, reminderDate: form.reminderDate ?? form.due ?? todayKey() })} />
          Reminder:
        </label>
        <DatePicker value={form.reminderOn ? form.reminderDate : null} disabled={!form.reminderOn} allowNone={false} onChange={(v) => set({ reminderDate: v })} width={160} />
        <span />
        <TimeField value={hhmmToMinutes(form.reminderTime)} disabled={!form.reminderOn} onChange={(m) => set({ reminderTime: minutesToHhmm(m) })} width={90} />
        <div className="cm-taskwin-inline">
          <span className="cm-label">Owner:</span>
          <span className="cm-ellipsis">{owner}</span>
        </div>

        <span className="cm-label">In folder:</span>
        <select className="cm-input wide" style={{ maxWidth: 360 }} value={form.listId ?? ""} onChange={(e) => set({ listId: Number(e.target.value) })}>
          {(lists.data ?? []).map((l) => (
            <option key={l.id} value={l.id}>
              {multiAccount ? `${l.title} (${l.accountEmail})` : l.title}
            </option>
          ))}
        </select>

        {form.categories.length > 0 && (
          <>
            <span className="cm-label">Categories:</span>
            <div className="wide">
              <CategoryChips names={form.categories} />
            </div>
          </>
        )}
      </div>
      <div className="cm-taskwin-body">
        <textarea className="cm-taskwin-notes" value={form.notes} onChange={(e) => set({ notes: e.target.value })} dir="auto" spellCheck />
      </div>
      <div className="cm-taskwin-statusline">
        <span>{task?.googleId ? "Synchronized with Google Tasks" : task ? "Waiting to be sent to Google Tasks" : "New task"}</span>
        <span className="cm-fill" />
        {dirty && <span>Unsaved changes</span>}
      </div>
      <ColorCategoriesHost />
    </div>
  );
}
