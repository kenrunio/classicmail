// Commands shared by the task table, toolbar, menus, the To-Do Bar and the task window.
import { useEffect, useRef, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { tasksApi, type TaskList } from "../../api/tasks";
import { openWindow } from "../../api/windows";
import { queryClient } from "../../api/query";
import { Dialog, errorBox, confirmBox, type MenuItem } from "../../ui";
import { DatePicker } from "./DatePicker";
import { flagDates, fmtDate, type FlagKind } from "./dates";
import { categorizeMenu as itemCategorizeMenu, currentCategories, openColorCategories } from "../calendar/categories";
import { printPage } from "../calendar/print";
import { escapeHtml } from "../mail/format";
import type { TodoItem } from "./store";
import { items as nItems, undoableMany } from "../../api/undo";

type Call = { method: string; params: object };
/** Task + flagged-mail changes as one Undo step (Ctrl+Z). */
const recorded = (label: string, calls: (Call | null | false)[]) =>
  run(
    undoableMany(
      label,
      calls.filter((c): c is Call => !!c),
    ).then(() => queryClient.invalidateQueries({ queryKey: ["todo"] })),
  );

const ids = (items: TodoItem[], kind: "task" | "mail") => items.filter((i) => i.kind === kind).map((i) => i.id);

async function run(p: Promise<unknown>) {
  try {
    await p;
  } catch (e) {
    errorBox(e);
  }
}

export function openTask(id: number, title?: string) {
  // one window per task: opening it again focuses the open window
  return openWindow("task", { id }, { title: `${title || "Untitled"} - Task`, label: `task-${id}` });
}

export function newTask(params: { listId?: number; due?: string } = {}) {
  return openWindow("task", params, { title: "Untitled - Task" });
}

export function openItem(it: TodoItem) {
  if (it.kind === "task") openTask(it.id, it.title);
  else openWindow("message", { id: it.id }, { title: `${it.title || "(no subject)"} - Message` });
}

export function setComplete(items: TodoItem[], complete: boolean) {
  const t = ids(items, "task");
  const m = ids(items, "mail");
  return recorded(`Mark ${nItems(t.length + m.length)} ${complete ? "Complete" : "Not Complete"}`, [
    t.length > 0 && { method: "tasks.setComplete", params: { ids: t, complete } },
    m.length > 0 && { method: "mail.setFlag", params: { ids: m, flag: complete ? "complete" : "today" } },
  ]);
}

/** Toggle: completes unless every item is already complete. */
export function toggleComplete(items: TodoItem[]) {
  return setComplete(items, !items.every((i) => i.complete));
}

export function followUp(items: TodoItem[], kind: FlagKind) {
  const { start, due } = flagDates(kind);
  const m = ids(items, "mail");
  return recorded(`Follow Up (${nItems(items.length)})`, [
    ...items.filter((i) => i.kind === "task").map((i) => ({ method: "tasks.update", params: { id: i.id, start, due, ...(i.complete ? { status: "notStarted" as const } : {}) } })),
    m.length > 0 && { method: "mail.setFlag", params: { ids: m, flag: kind } },
  ]);
}

export async function followUpCustom(items: TodoItem[]) {
  if (!items.length) return;
  const first = items[0];
  const r = await customFlagDialog(first.start, first.due);
  if (!r) return;
  const m = ids(items, "mail");
  await recorded(`Follow Up (${nItems(items.length)})`, [
    ...items.filter((i) => i.kind === "task").map((i) => ({ method: "tasks.update", params: { id: i.id, start: r.start, due: r.due } })),
    m.length > 0 && { method: "mail.setFlag", params: { ids: m, flag: "custom", due: r.due ?? undefined } },
  ]);
}

/** Category names compare without case, as in the Color Categories list. */
const hasCategory = (names: string[], name: string) => names.some((c) => c.toLowerCase() === name.toLowerCase());

/** Toggle one category on the tasks (on for all unless all have it), or clear them all (null). */
export function setCategory(items: TodoItem[], name: string | null) {
  const tasks = items.filter((i) => i.kind === "task");
  const allHave = name !== null && tasks.every((i) => hasCategory(i.categories, name));
  return recorded(
    name === null ? "Clear Categories" : `Categorize (${nItems(tasks.length)})`,
    tasks.map((i) => {
      const cats =
        name === null
          ? []
          : allHave
            ? i.categories.filter((c) => c.toLowerCase() !== name.toLowerCase())
            : hasCategory(i.categories, name)
              ? i.categories
              : [...i.categories, name];
      return { method: "tasks.update", params: { id: i.id, categories: cats } };
    }),
  );
}

/** Give one task exactly these categories (the Color Categories dialog's answer). */
function setTaskCategories(item: TodoItem, names: string[]) {
  return recorded(names.length ? "Categorize (1 item)" : "Clear Categories", [{ method: "tasks.update", params: { id: item.id, categories: names } }]);
}

export function deleteItems(items: TodoItem[]) {
  const t = ids(items, "task");
  if (!t.length) return Promise.resolve();
  return run(tasksApi.remove(t, t.length === 1 ? `Delete "${items.find((i) => i.kind === "task")?.title || "Untitled"}"` : `Delete ${t.length} tasks`));
}

export function moveItems(items: TodoItem[], listId: number) {
  const t = items.filter((i) => i.kind === "task" && i.task?.listId !== listId).map((i) => i.id);
  if (!t.length) return Promise.resolve();
  return run(tasksApi.move({ ids: t, listId }, t.length === 1 ? "Move 1 task" : `Move ${t.length} tasks`));
}

// ---- menus ----

export function followUpMenu(items: TodoItem[], opts: { complete?: boolean } = {}): MenuItem[] {
  const dis = !items.length;
  const f = (label: string, kind: FlagKind): MenuItem => ({ label, icon: "flag", disabled: dis, onClick: () => followUp(items, kind) });
  return [
    f("Today", "today"),
    f("Tomorrow", "tomorrow"),
    f("This Week", "thisWeek"),
    f("Next Week", "nextWeek"),
    f("No Date", "noDate"),
    { label: "Custom…", icon: "flag", disabled: dis, onClick: () => followUpCustom(items) },
    ...(opts.complete === false
      ? []
      : ([{ type: "separator" }, { label: "Mark Complete", icon: "flag-complete", disabled: dis, onClick: () => setComplete(items, true) }] as MenuItem[])),
  ];
}

/**
 * The calendar's "Categorize ▸" items for one item's categories (the user's Color Categories).
 * Their Ctrl+F2..F12 keys work in the Calendar only, so they aren't shown here.
 */
export function categoryItems(current: string[], apply: (names: string[]) => void): MenuItem[] {
  return itemCategorizeMenu(current, currentCategories(), apply).map((x) => (x.type === "separator" || x.type === "header" ? x : { ...x, shortcut: undefined }));
}

/** "Categorize ▸" for the selected tasks. */
export function categorizeMenu(items: TodoItem[]): MenuItem[] {
  const tasks = items.filter((i) => i.kind === "task");
  // one task: the calendar's own menu, whose All Categories… assigns several at once
  if (tasks.length === 1) return categoryItems(tasks[0].categories, (names) => setTaskCategories(tasks[0], names));
  const dis = !tasks.length;
  return [
    { label: "Clear All Categories", disabled: dis || tasks.every((i) => !i.categories.length), onClick: () => setCategory(tasks, null) },
    { type: "separator" },
    ...currentCategories().map((c): MenuItem => ({
      label: c.name,
      icon: `cm-cat-${c.color}`,
      checked: !dis && tasks.every((i) => hasCategory(i.categories, c.name)),
      disabled: dis,
      onClick: () => setCategory(tasks, c.name),
    })),
    { type: "separator" },
    { label: "All Categories…", onClick: () => void openColorCategories(null) },
  ];
}

export function moveMenu(items: TodoItem[], lists: TaskList[]): MenuItem[] {
  const tasks = items.filter((i) => i.kind === "task");
  const multi = new Set(lists.map((l) => l.accountId)).size > 1;
  return lists.map((l) => ({
    label: multi ? `${l.title} (${l.accountEmail})` : l.title,
    icon: "tasks",
    disabled: !tasks.length,
    onClick: () => moveItems(tasks, l.id),
  }));
}

export function itemMenu(items: TodoItem[], lists: TaskList[]): MenuItem[] {
  const single = items.length === 1 ? items[0] : null;
  const anyTask = items.some((i) => i.kind === "task");
  const allDone = items.length > 0 && items.every((i) => i.complete);
  return [
    { label: "Open", icon: "folder-open", bold: true, disabled: !single, onClick: () => single && openItem(single) },
    {
      label: "Print",
      icon: "print",
      disabled: !anyTask,
      onClick: () =>
        printItems(
          items.filter((i) => i.kind === "task"),
          "Tasks",
        ),
    },
    { type: "separator" },
    allDone
      ? { label: "Mark Incomplete", icon: "task", onClick: () => setComplete(items, false) }
      : { label: "Mark Complete", icon: "complete-task", shortcut: "Ins", onClick: () => setComplete(items, true) },
    { label: "Follow Up", icon: "flag", submenu: followUpMenu(items, { complete: false }) },
    { label: "Categorize", icon: "categorize", disabled: !anyTask, submenu: categorizeMenu(items) },
    { type: "separator" },
    { label: "Move to Folder", icon: "move", disabled: !anyTask, submenu: moveMenu(items, lists) },
    { label: "Delete", icon: "delete", shortcut: "Ctrl+D", disabled: !anyTask, onClick: () => deleteItems(items) },
  ];
}

// ---- dialogs (imperative, mounted on demand) ----

function showDialog<T>(render: (done: (v: T | null) => void) => ReactNode): Promise<T | null> {
  return new Promise((resolve) => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    const done = (v: T | null) => {
      root.unmount();
      host.remove();
      resolve(v);
    };
    root.render(render(done));
  });
}

function CustomFlag({ start0, due0, done }: { start0: string | null; due0: string | null; done: (v: { start: string | null; due: string | null } | null) => void }) {
  const [start, setStart] = useState(start0);
  const [due, setDue] = useState(due0);
  return (
    <Dialog
      title="Custom"
      onClose={() => done(null)}
      width={360}
      footer={
        <>
          <button className="cm-button default" onClick={() => done({ start, due: due && start && due < start ? start : due })}>
            OK
          </button>
          <button className="cm-button" onClick={() => done(null)}>
            Cancel
          </button>
        </>
      }
    >
      <div style={{ marginBottom: 10, color: "var(--text-muted)" }}>Choose a start date and a due date for the follow-up.</div>
      <div className="cm-form">
        <span className="cm-label">Start date:</span>
        <DatePicker value={start} onChange={(v) => setStart(v)} width={170} />
        <span className="cm-label">Due date:</span>
        <DatePicker value={due} onChange={(v) => setDue(v)} width={170} />
      </div>
    </Dialog>
  );
}

export function customFlagDialog(start: string | null, due: string | null) {
  return showDialog<{ start: string | null; due: string | null }>((done) => <CustomFlag start0={start} due0={due} done={done} />);
}

function FolderName({
  title,
  initial,
  accounts,
  done,
}: {
  title: string;
  initial: string;
  accounts?: { id: number; email: string }[];
  done: (v: { name: string; accountId?: number } | null) => void;
}) {
  const [name, setName] = useState(initial);
  const [acct, setAcct] = useState(accounts?.[0]?.id);
  const input = useRef<HTMLInputElement>(null);
  // Dialog focuses its first focusable (the title-bar X); move focus to the name box
  useEffect(() => {
    const t = setTimeout(() => input.current?.select(), 0);
    return () => clearTimeout(t);
  }, []);
  const ok = () => name.trim() && done({ name: name.trim(), accountId: acct });
  return (
    <Dialog
      title={title}
      onClose={() => done(null)}
      width={340}
      footer={
        <>
          <button className="cm-button default" disabled={!name.trim()} onClick={ok}>
            OK
          </button>
          <button className="cm-button" onClick={() => done(null)}>
            Cancel
          </button>
        </>
      }
    >
      <div style={{ marginBottom: 4 }}>Name:</div>
      <input
        ref={input}
        className="cm-input"
        style={{ width: "100%" }}
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            ok();
          }
        }}
      />
      {accounts && accounts.length > 1 && (
        <>
          <div style={{ margin: "10px 0 4px" }}>Select where to place the folder:</div>
          <select className="cm-input" style={{ width: "100%" }} value={acct} onChange={(e) => setAcct(Number(e.target.value))}>
            {accounts.map((a) => (
              <option key={a.id} value={a.id}>
                Tasks in {a.email}
              </option>
            ))}
          </select>
        </>
      )}
      <div style={{ marginTop: 8, color: "var(--text-muted)" }}>Folder contains: Task Items</div>
    </Dialog>
  );
}

export async function newFolder(lists: TaskList[], accountId?: number) {
  const accounts = [...new Map(lists.map((l) => [l.accountId, { id: l.accountId, email: l.accountEmail }])).values()];
  if (accountId) accounts.sort((a, b) => (a.id === accountId ? -1 : b.id === accountId ? 1 : 0));
  const r = await showDialog<{ name: string; accountId?: number }>((done) => <FolderName title="Create New Folder" initial="" accounts={accounts} done={done} />);
  if (r) await run(tasksApi.createList(r.name, r.accountId));
}

export async function renameFolder(l: TaskList) {
  const r = await showDialog<{ name: string }>((done) => <FolderName title="Rename" initial={l.title} done={done} />);
  if (r && r.name !== l.title) await run(tasksApi.renameList(l.id, r.name));
}

export async function deleteFolder(l: TaskList) {
  if (await confirmBox(`Are you sure you want to delete the folder "${l.title}" and all of its tasks?`, "ClassicMail", "warning", "tasks.deleteFolder"))
    await run(tasksApi.deleteList(l.id));
}

// ---- print ----

export function printItems(items: TodoItem[], title: string) {
  const rows = items
    .map(
      (i) =>
        `<tr${i.complete ? ' class="done"' : ""}><td>${i.complete ? "✓" : ""}</td><td>${escapeHtml(i.title)}</td><td>${escapeHtml(i.status)}</td><td>${escapeHtml(fmtDate(i.due, ""))}</td><td>${i.percent}%</td><td>${escapeHtml(i.categories.join(", "))}</td><td>${escapeHtml(i.folder)}</td></tr>` +
        (i.task?.notes ? `<tr><td></td><td colspan="6" class="notes">${escapeHtml(i.task.notes)}</td></tr>` : ""),
    )
    .join("");
  printPage(
    title,
    `body{font:12px "Segoe UI",Tahoma,sans-serif;margin:16px} h1{font-size:16px;margin:0 0 8px}
    table{border-collapse:collapse;width:100%} th,td{border-bottom:1px solid #ccc;padding:3px 6px;text-align:left;vertical-align:top}
    th{background:#eee} tr.done td{text-decoration:line-through;color:#777} td.notes{color:#444;white-space:pre-wrap;font-size:11px}`,
    `<h1>${escapeHtml(title)}</h1><table><tr><th></th><th>Subject</th><th>Status</th><th>Due Date</th><th>% Complete</th><th>Categories</th><th>In Folder</th></tr>${rows}</table>`,
  );
}
