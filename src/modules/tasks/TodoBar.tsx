// Outlook 2007 To-Do Bar (right side of the main window, in every module): Date Navigator,
// upcoming Appointments, and the task list "Arranged By: Due Date" with "Type a new task",
// tasks + flagged mail grouped Today / Tomorrow / This Week / Next Week / Later / No Date.
import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { addDays, addMonths, differenceInCalendarDays, endOfMonth, startOfDay, startOfMonth, startOfWeek } from "date-fns";
import { useRpc } from "../../api/query";
import { listEvents, listFlaggedMail, tasksApi, type CalEvent, type Task, type TaskList } from "../../api/tasks";
import { openWindow } from "../../api/windows";
import { DateNavigator, Icon, contextMenu, dayKey, errorBox, showMenu, usePersistentState, type MenuItem } from "../../ui";
import { useShell } from "../../shell/store";
import { useCal, useCalSettings } from "../calendar/state";
import { ColorCategoriesHost } from "../calendar/categories";
import { DUE_GROUPS, dueGroup, flagColor, fmtTime, parseDayKey, isOverdue, todayKey, type DueGroup, type FlagKind } from "./dates";
import { CategoryChips, FlagGlyph, MailGlyph, TaskGlyph } from "./icons";
import { mailItem, taskItem, type TodoItem } from "./store";
import { deleteItems, followUp, itemMenu, openItem, setComplete, toggleComplete } from "./actions";
import "./tasks.css";

interface Sections {
  dateNav: boolean;
  appts: boolean;
  tasks: boolean;
}

const IMPORTANCE = { high: 0, normal: 1, low: 2 } as const;

export function TodoBar({ onClose }: { onClose: () => void }) {
  const [sections, setSections] = usePersistentState<Sections>("todobar.sections", { dateNav: true, appts: true, tasks: true });
  const ref = useRef<HTMLDivElement>(null);
  const [height, setHeight] = useState(600);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setHeight(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const sectionMenu = (): MenuItem[] => [
    { label: "Date Navigator", checked: sections.dateNav, onClick: () => setSections((s) => ({ ...s, dateNav: !s.dateNav })) },
    { label: "Appointments", checked: sections.appts, onClick: () => setSections((s) => ({ ...s, appts: !s.appts })) },
    { label: "Task List", checked: sections.tasks, onClick: () => setSections((s) => ({ ...s, tasks: !s.tasks })) },
    { type: "separator" },
    { label: "Minimize the To-Do Bar", icon: "chevron-right-double", onClick: onClose },
  ];

  return (
    <div className="cm-todo" ref={ref} onContextMenu={contextMenu(sectionMenu)}>
      <div className="cm-todo-title">
        <span className="cm-fill">To-Do Bar</span>
        <button className="cm-todo-close" title="Minimize the To-Do Bar" onClick={onClose}>
          <Icon name="chevron-right-double" size={12} />
        </button>
      </div>
      <div className="cm-todo-body">
        {sections.dateNav && <TodoDateNav months={height > 720 ? 2 : 1} />}
        {sections.appts && <Appointments />}
        {sections.tasks && <TodoTasks />}
        {!sections.dateNav && !sections.appts && !sections.tasks && <div className="cm-todo-empty">Right-click here to choose what the To-Do Bar shows.</div>}
      </div>
      {/* Categorize ▸ All Categories… from the task list's menus, in any module */}
      <ColorCategoriesHost />
    </div>
  );
}

// ---- date navigator ----------------------------------------------------------------------------

function TodoDateNav({ months }: { months: number }) {
  const [month, setMonth] = useState(() => startOfMonth(new Date()));
  // the first day of the week from the calendar's options, as in the Calendar's own navigator
  const { weekStartsOn } = useCalSettings();
  const start = startOfWeek(month, { weekStartsOn }).getTime();
  const end = addDays(endOfMonth(addMonths(month, months - 1)), 7).getTime();
  const ev = useQuery({ queryKey: ["todo", "events", start, end], queryFn: () => listEvents(start, end), staleTime: 60_000 });
  const bold = useMemo(() => {
    const s = new Set<string>();
    for (const e of ev.data ?? []) {
      let d = startOfDay(new Date(e.allDay && e.startDate ? `${e.startDate}T00:00` : e.start));
      const last = e.allDay && e.endDate ? addDays(new Date(`${e.endDate}T00:00`), -1) : new Date(Math.max(e.start, e.end - 1));
      for (let i = 0; d <= last && i < 62; i++, d = addDays(d, 1)) s.add(dayKey(d));
    }
    return s;
  }, [ev.data]);
  return (
    <div className="cm-todo-datenav">
      <DateNavigator
        month={month}
        onMonthChange={setMonth}
        months={months}
        boldDays={bold}
        weekStartsOn={weekStartsOn}
        compact
        onSelect={(d) => {
          // like Outlook: clicking a date opens the Calendar on that day (set the calendar's state
          // directly — the Calendar view isn't mounted yet when coming from another module)
          useCal.getState().goTo(startOfDay(d).getTime(), "day");
          useShell.getState().setModule("calendar");
        }}
      />
    </div>
  );
}

// ---- appointments ------------------------------------------------------------------------------

/** "Tuesday, 13 October" (regional order) */
const DAY_LABEL = new Intl.DateTimeFormat(undefined, { weekday: "long", day: "numeric", month: "long" });

function dayLabel(d: Date, now: Date) {
  const diff = differenceInCalendarDays(d, now);
  if (diff === 0) return "Today";
  if (diff === 1) return "Tomorrow";
  return DAY_LABEL.format(d);
}

/** The time now, refreshed every minute (finished appointments drop off, dates move on at midnight). */
function useNow(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(t);
  }, []);
  return now;
}

function Appointments() {
  const now = useNow();
  const from = startOfDay(new Date(now)).getTime();
  const to = addDays(new Date(from), 8).getTime();
  const q = useQuery({ queryKey: ["todo", "appts", from], queryFn: () => listEvents(from, to), staleTime: 60_000 });
  const appts = useMemo(
    () =>
      (q.data ?? [])
        .filter((e) => (e.allDay ? (e.endDate ?? "") > todayKey() : e.end > now) && e.myResponse !== "declined")
        .sort((a, b) => a.start - b.start || Number(b.allDay) - Number(a.allDay))
        .slice(0, 8),
    [q.data, now],
  );
  let lastDay = "";
  return (
    <div className="cm-todo-appts">
      {appts.length === 0 && <div className="cm-todo-empty">{q.isLoading ? "" : "No upcoming appointments."}</div>}
      {appts.map((e) => {
        const d = e.allDay && e.startDate ? new Date(`${e.startDate}T00:00`) : new Date(e.start);
        const dk = dayKey(d < new Date(now) ? new Date(now) : d);
        const header = dk !== lastDay ? dayLabel(d < new Date(now) ? new Date(now) : d, new Date(now)) : null;
        lastDay = dk;
        return (
          <div key={`${e.id}-${e.start}`}>
            {header && <div className="cm-todo-day">{header}</div>}
            <Appt e={e} />
          </div>
        );
      })}
    </div>
  );
}

function Appt({ e }: { e: CalEvent }) {
  const time = e.allDay ? "All Day" : `${fmtTime(e.start)} - ${fmtTime(e.end)}`;
  return (
    <div
      className="cm-todo-appt"
      style={{ borderLeftColor: e.calendarColor || undefined }}
      title={`${e.summary}\n${time}${e.location ? `\n${e.location}` : ""}`}
      // same window label as the Calendar uses, so opening it twice focuses the open window
      onClick={(ev) =>
        ev.detail === 1 &&
        openWindow(
          "appointment",
          { id: e.id, scope: "occurrence" },
          { label: `appointment-${e.id}-occurrence`, title: `${e.summary || "Untitled"} - ${e.isMeeting ? "Meeting" : "Appointment"}` },
        )
      }
    >
      <div className="cm-todo-appt-subject cm-ellipsis" dir="auto">
        {e.summary || "(No subject)"}
      </div>
      <div className="cm-todo-appt-meta cm-ellipsis" dir="auto">
        {time}
        {e.location ? ` · ${e.location}` : ""}
      </div>
    </div>
  );
}

// ---- task list ---------------------------------------------------------------------------------

const DROP_FLAG: Partial<Record<DueGroup, FlagKind>> = {
  today: "today",
  tomorrow: "tomorrow",
  thisWeek: "thisWeek",
  nextWeek: "nextWeek",
  noDate: "noDate",
};

function TodoTasks() {
  const today = todayKey(new Date(useNow()));
  const tasks = useRpc<Task[]>(["tasks", "todo", today], "tasks.list", { view: "todo", today });
  const lists = useRpc<TaskList[]>(["tasks", "lists"], "tasks.lists");
  const flagged = useQuery({ queryKey: ["todo", "flagged"], queryFn: listFlaggedMail, staleTime: 30_000 });
  const [sel, setSel] = useState<string | null>(null);
  const [text, setText] = useState("");
  const [dropOn, setDropOn] = useState<string | null>(null);
  const [collapsed, setCollapsed] = usePersistentState<string[]>("todobar.collapsed", []);

  const groups = useMemo(() => {
    const items: TodoItem[] = [
      ...(tasks.data ?? []).map(taskItem).map((i) => ({ ...i, depth: 0 })),
      ...(flagged.data ?? []).filter((m) => m.flagStatus !== "complete").map(mailItem),
    ];
    items.sort((a, b) => (a.due ?? "9999").localeCompare(b.due ?? "9999") || IMPORTANCE[a.importance] - IMPORTANCE[b.importance] || a.title.localeCompare(b.title));
    // regrouped when the day changes too (Tomorrow's items move to Today at midnight)
    const now = parseDayKey(today);
    return DUE_GROUPS.map((g) => ({ ...g, items: items.filter((i) => dueGroup(i.due, now) === g.id) })).filter((g) => g.items.length);
  }, [tasks.data, flagged.data, today]);
  const all = groups.flatMap((g) => g.items);
  const selected = all.find((i) => i.key === sel);

  const add = async () => {
    const title = text.trim();
    if (!title) return;
    setText("");
    try {
      await tasksApi.create({ title, due: todayKey() }, `New Task "${title}"`);
    } catch (e) {
      setText(title);
      errorBox(e);
    }
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (!selected) return;
    const i = all.findIndex((x) => x.key === sel);
    if (e.key === "ArrowDown" && i < all.length - 1) setSel(all[i + 1].key);
    else if (e.key === "ArrowUp" && i > 0) setSel(all[i - 1].key);
    else if (e.key === "Enter") openItem(selected);
    else if (e.key === "Delete") deleteItems([selected]);
    else if (e.key === "Insert") toggleComplete([selected]);
    else return;
    e.preventDefault();
  };

  return (
    <div className="cm-todo-tasks">
      <div className="cm-todo-arranged" title="Arranged By: Due Date">
        <span className="cm-fill">Arranged By: Due Date</span>
      </div>
      <div className="cm-todo-new">
        <input
          placeholder="Type a new task"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") add();
            if (e.key === "Escape") setText("");
          }}
          dir="auto"
        />
      </div>
      <div className="cm-todo-list" tabIndex={0} onKeyDown={onKey}>
        {groups.length === 0 && !tasks.isLoading && <div className="cm-todo-empty">There are no items to show in this view.</div>}
        {groups.map((g) => {
          const isCollapsed = collapsed.includes(g.id);
          const flag = DROP_FLAG[g.id];
          return (
            <div key={g.id}>
              <div
                className={`cm-todo-group${dropOn === g.id ? " drop" : ""}`}
                onClick={() => setCollapsed((c) => (c.includes(g.id) ? c.filter((x) => x !== g.id) : [...c, g.id]))}
                onDragOver={(e) => {
                  if (!flag || !e.dataTransfer.types.includes("application/x-cm-items")) return;
                  e.preventDefault();
                  setDropOn(g.id);
                }}
                onDragLeave={() => setDropOn(null)}
                onDrop={(e) => {
                  setDropOn(null);
                  const raw = e.dataTransfer.getData("application/x-cm-items");
                  if (!raw || !flag) return;
                  e.preventDefault();
                  try {
                    const p = JSON.parse(raw) as { kind?: string; ids?: number[]; keys?: string[] };
                    const keys = p.keys ?? (p.kind === "tasks" ? (p.ids ?? []).map((id) => `t${id}`) : []);
                    const items = all.filter((x) => keys.includes(x.key));
                    if (items.length) followUp(items, flag);
                  } catch {
                    /* not ours */
                  }
                }}
              >
                <span className="cm-tasks-group-toggle">{isCollapsed ? "+" : "−"}</span>
                <span className="cm-fill">{g.label}</span>
              </div>
              {!isCollapsed &&
                g.items.map((it) => {
                  const overdue = isOverdue(it.due, it.complete);
                  return (
                    <div
                      key={it.key}
                      className={`cm-todo-item${sel === it.key ? " selected" : ""}${overdue ? " overdue" : ""}${it.complete ? " done" : ""}`}
                      title={`${it.title}\n${it.kind === "mail" ? `From: ${it.mail?.from ?? ""}` : `In Folder: ${it.folder}`}`}
                      draggable
                      onDragStart={(e) => {
                        e.dataTransfer.setData(
                          "application/x-cm-items",
                          JSON.stringify(it.kind === "task" ? { kind: "tasks", ids: [it.id], keys: [it.key] } : { kind: "todo", keys: [it.key] }),
                        );
                        e.dataTransfer.effectAllowed = "move";
                      }}
                      onMouseDown={() => setSel(it.key)}
                      onDoubleClick={() => openItem(it)}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        setSel(it.key);
                        showMenu(e.clientX, e.clientY, itemMenu([it], lists.data ?? []));
                      }}
                    >
                      {it.kind === "mail" ? <MailGlyph /> : <TaskGlyph done={it.complete} />}
                      <span className="cm-todo-item-title cm-ellipsis" dir="auto">
                        {it.title || "(no subject)"}
                      </span>
                      <CategoryChips names={it.categories} compact />
                      <span
                        className="cm-tasks-flag"
                        title="Click to mark complete"
                        onMouseDown={(e) => e.stopPropagation()}
                        onClick={(e) => {
                          e.stopPropagation();
                          setComplete([it], !it.complete);
                        }}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          e.stopPropagation();
                          showMenu(e.clientX, e.clientY, [
                            ...(["today", "tomorrow", "thisWeek", "nextWeek", "noDate"] as FlagKind[]).map((k, i) => ({
                              label: ["Today", "Tomorrow", "This Week", "Next Week", "No Date"][i],
                              icon: "flag",
                              onClick: () => followUp([it], k),
                            })),
                            { type: "separator" },
                            { label: "Mark Complete", icon: "flag-complete", onClick: () => setComplete([it], true) },
                          ]);
                        }}
                      >
                        <FlagGlyph complete={it.complete} color={flagColor(it.due)} />
                      </span>
                    </div>
                  );
                })}
            </div>
          );
        })}
      </div>
    </div>
  );
}
