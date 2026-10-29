// Tasks module UI state (folder, current view, selection, sort, search) and the unified item
// model shared by the task table, the To-Do Bar and the toolbar/menu commands.
import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { FlaggedMail, Importance, Task, TaskView } from "../../api/tasks";
import { STATUS_LABELS } from "../../api/tasks";

export type Folder = { kind: "todo" } | { kind: "list"; listId: number } | { kind: "account"; accountId: number };

export type ColId = "icon" | "priority" | "complete" | "subject" | "status" | "start" | "due" | "percent" | "categories" | "folder" | "flag";

export interface Sort {
  col: ColId;
  dir: 1 | -1;
}

export const VIEWS: { id: TaskView; label: string }[] = [
  { id: "simple", label: "Simple List" },
  { id: "detailed", label: "Detailed List" },
  { id: "active", label: "Active Tasks" },
  { id: "next7", label: "Next Seven Days" },
  { id: "overdue", label: "Overdue Tasks" },
  { id: "category", label: "By Category" },
  { id: "completed", label: "Completed Tasks" },
  { id: "todo", label: "To-Do List" },
];

interface TasksUi {
  folder: Folder;
  view: TaskView;
  sort: Sort | null;
  search: string;
  selection: string[];
  anchor: string | null;
  collapsed: string[];
  setFolder: (f: Folder) => void;
  setView: (v: TaskView) => void;
  setSort: (s: Sort | null) => void;
  setSearch: (s: string) => void;
  select: (keys: string[], anchor?: string | null) => void;
  toggleGroup: (g: string) => void;
}

export const useTasksUi = create<TasksUi>()(
  persist(
    (set) => ({
      folder: { kind: "todo" },
      view: "todo",
      sort: null,
      search: "",
      selection: [],
      anchor: null,
      collapsed: [],
      setFolder: (folder) =>
        set((s) => ({
          folder,
          view: folder.kind === "todo" ? "todo" : s.view === "todo" ? "simple" : s.view,
          selection: [],
          search: "",
        })),
      setView: (view) => set({ view, sort: null, selection: [] }),
      setSort: (sort) => set({ sort }),
      setSearch: (search) => set({ search }),
      select: (selection, anchor) => set((s) => ({ selection, anchor: anchor === undefined ? s.anchor : anchor })),
      toggleGroup: (g) => set((s) => ({ collapsed: s.collapsed.includes(g) ? s.collapsed.filter((x) => x !== g) : [...s.collapsed, g] })),
    }),
    { name: "cm:tasks", partialize: (s) => ({ folder: s.folder, view: s.view, collapsed: s.collapsed }) },
  ),
);

/** One row of the task table / To-Do Bar: a task or a flagged message. */
export interface TodoItem {
  key: string;
  kind: "task" | "mail";
  id: number;
  title: string;
  due: string | null;
  start: string | null;
  complete: boolean;
  folder: string;
  status: string;
  percent: number;
  categories: string[];
  importance: Importance;
  depth: number;
  task?: Task;
  mail?: FlaggedMail;
}

export function taskItem(t: Task): TodoItem {
  return {
    key: `t${t.id}`,
    kind: "task",
    id: t.id,
    title: t.title,
    due: t.due,
    start: t.start,
    complete: t.complete,
    folder: t.listTitle,
    status: STATUS_LABELS[t.status] ?? t.status,
    percent: t.percent,
    categories: t.categories,
    importance: t.importance,
    depth: t.parentId ? 1 : 0,
    task: t,
  };
}

export function mailItem(m: FlaggedMail): TodoItem {
  return {
    key: `m${m.id}`,
    kind: "mail",
    id: m.id,
    title: m.subject || "(no subject)",
    due: m.flagDue ?? null,
    start: null,
    complete: m.flagStatus === "complete",
    folder: "Inbox",
    status: m.flagStatus === "complete" ? "Completed" : "Not Started",
    percent: m.flagStatus === "complete" ? 100 : 0,
    categories: [],
    importance: "normal",
    depth: 0,
    mail: m,
  };
}

const IMPORTANCE_RANK: Record<Importance, number> = { low: 0, normal: 1, high: 2 };

export function compareItems(a: TodoItem, b: TodoItem, s: Sort): number {
  const str = (x: string | null | undefined) => x ?? "";
  let r = 0;
  switch (s.col) {
    case "subject":
      r = a.title.localeCompare(b.title);
      break;
    case "status":
      r = a.status.localeCompare(b.status);
      break;
    case "start":
      r = (a.start ? 0 : 1) - (b.start ? 0 : 1) || str(a.start).localeCompare(str(b.start));
      break;
    case "due":
    case "flag":
      r = (a.due ? 0 : 1) - (b.due ? 0 : 1) || str(a.due).localeCompare(str(b.due));
      break;
    case "percent":
      r = a.percent - b.percent;
      break;
    case "categories":
      r = a.categories.join(",").localeCompare(b.categories.join(","));
      break;
    case "folder":
      r = a.folder.localeCompare(b.folder);
      break;
    case "priority":
      r = IMPORTANCE_RANK[a.importance] - IMPORTANCE_RANK[b.importance];
      break;
    case "complete":
    case "icon":
      r = Number(a.complete) - Number(b.complete);
      break;
  }
  return r * s.dir || a.title.localeCompare(b.title);
}

/** The rows currently shown by the Tasks view (for the toolbar and menus). */
export const useViewItems = create<{ items: TodoItem[]; set: (items: TodoItem[]) => void }>((set) => ({
  items: [],
  set: (items) => set({ items }),
}));

/** Selected rows of the Tasks view. */
export function useSelectedItems(): TodoItem[] {
  const items = useViewItems((s) => s.items);
  const sel = useTasksUi((s) => s.selection);
  const set = new Set(sel);
  return items.filter((i) => set.has(i.key));
}
