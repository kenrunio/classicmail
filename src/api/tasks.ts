// Tasks DTOs and rpc wrappers (back end: src-tauri/src/tasks).
import { rpc, RpcError } from "./rpc";
import { maybeUndoable as via } from "./undo";

export type TaskStatus = "notStarted" | "inProgress" | "completed" | "waiting" | "deferred";
export type Importance = "low" | "normal" | "high";
export type TaskView = "simple" | "detailed" | "active" | "next7" | "overdue" | "completed" | "todo" | "category";

export interface TaskList {
  id: number;
  accountId: number;
  accountEmail: string;
  accountName: string;
  googleId: string | null;
  title: string;
  isDefault: boolean;
  position: number;
  activeCount: number;
  totalCount: number;
}

export interface Task {
  id: number;
  accountId: number;
  listId: number;
  listTitle: string;
  googleId: string | null;
  parentId: number | null;
  position: string;
  title: string;
  notes: string;
  /** YYYY-MM-DD */
  due: string | null;
  start: string | null;
  status: TaskStatus;
  complete: boolean;
  completedAt: number | null;
  updated: number;
  importance: Importance;
  percent: number;
  reminderAt: number | null;
  categories: string[];
  isPrivate: boolean;
  hidden: boolean;
}

export interface TaskInput {
  listId?: number;
  title: string;
  notes?: string;
  due?: string | null;
  start?: string | null;
  status?: TaskStatus;
  importance?: Importance;
  percent?: number;
  reminderAt?: number | null;
  categories?: string[];
  isPrivate?: boolean;
  parentId?: number;
  previousId?: number;
}

export type TaskPatch = Partial<Omit<TaskInput, "listId" | "parentId" | "previousId">> & { id: number };

export const STATUS_LABELS: Record<TaskStatus, string> = {
  notStarted: "Not Started",
  inProgress: "In Progress",
  completed: "Completed",
  waiting: "Waiting on someone else",
  deferred: "Deferred",
};

// ---- calls ----

export const tasksApi = {
  lists: () => rpc<TaskList[]>("tasks.lists"),
  createList: (title: string, accountId?: number) => rpc<TaskList>("tasks.createList", { title, accountId }),
  renameList: (id: number, title: string) => rpc<void>("tasks.renameList", { id, title }),
  deleteList: (id: number) => rpc<void>("tasks.deleteList", { id }),
  list: (p: { listId?: number; accountId?: number; view: TaskView; search?: string; today?: string }) => rpc<Task[]>("tasks.list", p),
  get: (id: number) => rpc<Task>("tasks.get", { id }),
  // with an undo `label` the change is recorded in the Undo history (Ctrl+Z)
  create: (t: TaskInput, label?: string) => via<Task>(label, "tasks.create", t),
  update: (p: TaskPatch, label?: string) => via<Task>(label, "tasks.update", p),
  remove: (ids: number[], label?: string) => via<void>(label, "tasks.delete", { ids }),
  setComplete: (ids: number[], complete: boolean, label?: string) => via<void>(label, "tasks.setComplete", { ids, complete }),
  move: (p: { ids: number[]; listId?: number; parentId?: number | null; previousId?: number | null }, label?: string) => via<number[]>(label, "tasks.move", p),
};

// ---- other domains (may not exist yet while domains are built in parallel) ----

/** Flagged mail for the To-Do Bar / To-Do List (mail domain, contracts.md). */
export interface FlaggedMail {
  id: number;
  accountId: number;
  subject: string;
  from: string;
  receivedAt: number;
  flagDue?: string;
  flagStatus: "flagged" | "complete";
}

/** Calendar event (calendar domain, contracts.md). */
export interface CalEvent {
  id: number;
  accountId: number;
  calendarId: number;
  calendarColor: string;
  summary: string;
  location: string;
  start: number;
  end: number;
  allDay: boolean;
  startDate?: string;
  endDate?: string;
  isRecurring: boolean;
  isMeeting: boolean;
  myResponse?: "accepted" | "tentative" | "declined" | "needsAction";
  busyStatus: "busy" | "free" | "tentative" | "oof";
  isPrivate: boolean;
  hasReminder: boolean;
}

/** Call another domain's method; a missing method (not built yet) yields `fallback`. */
export async function rpcOptional<T>(method: string, params: object, fallback: T): Promise<T> {
  try {
    return await rpc<T>(method, params);
  } catch (e) {
    if (e instanceof RpcError && e.code === "no_method") return fallback;
    throw e;
  }
}

export const listFlaggedMail = () => rpcOptional<FlaggedMail[]>("mail.listFlagged", {}, []);
export const listEvents = (start: number, end: number) => rpcOptional<CalEvent[]>("calendar.listEvents", { start, end }, []);
export const setMailFlag = (ids: number[], flag: string, due?: string) => rpcOptional<null>("mail.setFlag", { ids, flag, due }, null);
