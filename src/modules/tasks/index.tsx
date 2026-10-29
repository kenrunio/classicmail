// Tasks module: Navigation Pane (My Tasks folders + Current View), the task table view,
// the module toolbar and the Edit / View / Actions menu contributions.
import { useEffect, useMemo, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import type { ModuleDef, ModuleMenus } from "../../shell/types";
import { useStatus } from "../../shell/store";
import { newItemMenu } from "../../shell/newItems";
import { useRpc } from "../../api/query";
import { rpc } from "../../api/rpc";
import { queryClient } from "../../api/query";
import { openWindow } from "../../api/windows";
import { listFlaggedMail, tasksApi, type Task, type TaskList, type TaskView } from "../../api/tasks";
import { errorBox, Icon, TbButton, TbDropdown, TbSep, TbSplit, Toolbar, TreeView, useDebounced, useHotkeys, type MenuItem, type TreeNode } from "../../ui";
import { mailItem, taskItem, useSelectedItems, useTasksUi, useViewItems, VIEWS, type Folder } from "./store";
import { TaskTable } from "./TaskTable";
import { categorizeMenu, deleteFolder, deleteItems, followUpMenu, moveMenu, newFolder, newTask, printItems, renameFolder, setComplete } from "./actions";
import { todayKey } from "./dates";
import { ColorCategoriesHost } from "../calendar/categories";
import "./tasks.css";

export function useTaskLists() {
  return useRpc<TaskList[]>(["tasks", "lists"], "tasks.lists");
}

function folderKey(f: Folder): string {
  return f.kind === "todo" ? "todo" : f.kind === "list" ? `l${f.listId}` : `a${f.accountId}`;
}

// ---- navigation pane ---------------------------------------------------------------------------

function NavPane() {
  const ui = useTasksUi();
  const data = useTaskLists().data;
  const lists = useMemo(() => data ?? [], [data]);
  const accounts = useMemo(() => {
    const m = new Map<number, { id: number; email: string; lists: TaskList[] }>();
    for (const l of lists) {
      if (!m.has(l.accountId)) m.set(l.accountId, { id: l.accountId, email: l.accountEmail, lists: [] });
      m.get(l.accountId)!.lists.push(l);
    }
    return [...m.values()];
  }, [lists]);

  const listNode = (l: TaskList): TreeNode => ({
    id: `l${l.id}`,
    label: l.title,
    icon: "tasks",
    count: l.activeCount,
    countStyle: "total",
    data: l,
  });
  const nodes: TreeNode[] = [
    { id: "todo", label: "To-Do List", icon: "todo-bar" },
    ...(accounts.length === 1
      ? accounts[0].lists.map(listNode)
      : accounts.map((a) => ({ id: `a${a.id}`, label: `Tasks - ${a.email}`, icon: "mailbox", children: a.lists.map(listNode), data: a }))),
  ];

  const menuFor = (n: TreeNode): MenuItem[] => {
    if (n.id === "todo") return [{ label: "Open", icon: "folder-open", bold: true, onClick: () => ui.setFolder({ kind: "todo" }) }];
    if (n.id.startsWith("a")) {
      const a = n.data as { id: number };
      return [
        { label: "Open", icon: "folder-open", bold: true, onClick: () => ui.setFolder({ kind: "account", accountId: a.id }) },
        { type: "separator" },
        { label: "New Folder…", icon: "new-folder", onClick: () => newFolder(lists, a.id) },
      ];
    }
    const l = n.data as TaskList;
    return [
      { label: "Open", icon: "folder-open", bold: true, onClick: () => ui.setFolder({ kind: "list", listId: l.id }) },
      { type: "separator" },
      { label: "New Folder…", icon: "new-folder", onClick: () => newFolder(lists, l.accountId) },
      { label: `Rename "${l.title}"…`, icon: "rename", disabled: l.isDefault, onClick: () => renameFolder(l) },
      { label: `Delete "${l.title}"`, icon: "delete", disabled: l.isDefault, onClick: () => deleteFolder(l) },
    ];
  };

  return (
    <div className="cm-tasks-nav">
      <div className="cm-nav-section">My Tasks</div>
      <TreeView
        nodes={nodes}
        selectedId={folderKey(ui.folder)}
        onSelect={(n) => {
          if (n.id === "todo") ui.setFolder({ kind: "todo" });
          else if (n.id.startsWith("l")) ui.setFolder({ kind: "list", listId: (n.data as TaskList).id });
          else ui.setFolder({ kind: "account", accountId: (n.data as { id: number }).id });
        }}
        menuFor={menuFor}
        canDrop={(n) => n.id.startsWith("l")}
        onDropItems={(n, payload) => {
          try {
            const p = JSON.parse(payload) as { kind?: string; ids?: number[] };
            if (p.kind === "tasks" && p.ids) {
              const l = n.data as TaskList;
              tasksApi.move({ ids: p.ids, listId: l.id }, `Move ${p.ids.length === 1 ? "1 task" : `${p.ids.length} tasks`} to ${l.title}`).catch(errorBox);
            }
          } catch {
            /* not ours */
          }
        }}
      />
      <div className="cm-nav-section">Current View</div>
      <div className="cm-tasks-views" role="radiogroup">
        {VIEWS.map((v) => (
          <label key={v.id} className="cm-tasks-viewradio">
            <input type="radio" name="cm-tasks-view" checked={ui.view === v.id} onChange={() => setView(v.id)} />
            {v.label}
          </label>
        ))}
      </div>
      <div className="cm-fill" />
      <div className="cm-tasks-navlinks">
        <a className="cm-nav-link" onClick={() => newFolder(lists)}>
          Add New Folder…
        </a>
        <a className="cm-nav-link" onClick={() => rpc("sync.now", {})}>
          Refresh from Google Tasks
        </a>
      </div>
    </div>
  );
}

/** Choosing a view: To-Do List shows every folder; other views keep the current folder. */
function setView(v: TaskView) {
  const ui = useTasksUi.getState();
  if (v === "todo") {
    ui.setFolder({ kind: "todo" });
  } else if (ui.folder.kind === "todo") {
    // stay on "all folders" — represented by the To-Do List folder with a list view
    useTasksUi.setState({ view: v, sort: null, selection: [] });
  } else ui.setView(v);
}

// ---- view --------------------------------------------------------------------------------------

function View() {
  const ui = useTasksUi();
  const lists = useTaskLists().data ?? [];
  const search = useDebounced(ui.search, 200);
  const params = {
    view: ui.view,
    listId: ui.folder.kind === "list" ? ui.folder.listId : undefined,
    accountId: ui.folder.kind === "account" ? ui.folder.accountId : undefined,
    search: search || undefined,
    today: todayKey(),
  };
  const q = useRpc<Task[]>(["tasks", "list", params], "tasks.list", params, { placeholderData: (prev) => prev });
  const flagged = useQuery({ queryKey: ["todo", "flagged"], queryFn: listFlaggedMail, enabled: ui.view === "todo", staleTime: 30_000 });
  const items = useMemo(() => {
    const t = (q.data ?? []).map(taskItem);
    if (ui.view !== "todo") return t;
    const s = search.toLowerCase();
    const m = (flagged.data ?? []).filter((f) => f.flagStatus !== "complete" && (!s || f.subject.toLowerCase().includes(s))).map(mailItem);
    return [...t, ...m];
  }, [q.data, flagged.data, ui.view, search]);

  const setItems = useViewItems((s) => s.set);
  useEffect(() => setItems(items), [items, setItems]);
  const setLeft = useStatus((s) => s.setLeft);
  useEffect(() => {
    setLeft(`Items: ${items.length.toLocaleString()}`);
  }, [items.length, setLeft]);
  useEffect(() => () => setLeft(""), [setLeft]);

  const list = ui.folder.kind === "list" ? lists.find((l) => l.id === (ui.folder as { listId: number }).listId) : undefined;
  const title =
    ui.folder.kind === "todo"
      ? ui.view === "todo"
        ? "To-Do List"
        : "Tasks"
      : ui.folder.kind === "list"
        ? (list?.title ?? "Tasks")
        : `Tasks - ${lists.find((l) => l.accountId === (ui.folder as { accountId: number }).accountId)?.accountEmail ?? ""}`;
  const multiFolder = ui.folder.kind !== "list";
  const manualOrder = ui.folder.kind === "list" && !ui.sort && !search && (ui.view === "simple" || ui.view === "detailed" || ui.view === "active");

  const searchRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const f = () => searchRef.current?.focus();
    const p = () =>
      printItems(
        useViewItems.getState().items.filter((i) => i.kind === "task"),
        title,
      );
    window.addEventListener("cm-focus-search", f);
    window.addEventListener("cm-print", p);
    return () => {
      window.removeEventListener("cm-focus-search", f);
      window.removeEventListener("cm-print", p);
    };
  }, [title]);
  useHotkeys({ "ctrl+e": () => searchRef.current?.focus(), "ctrl+p": () => window.dispatchEvent(new CustomEvent("cm-print")) });

  const viewLabel = VIEWS.find((v) => v.id === ui.view)?.label ?? "";
  return (
    <>
      <div className="cm-pane-title">
        <Icon name={ui.view === "todo" ? "todo-bar" : "tasks"} size={20} />
        <span className="cm-fill cm-ellipsis">
          {title}
          {ui.view !== "todo" && ui.view !== "simple" && <span className="cm-tasks-titleview"> ({viewLabel})</span>}
        </span>
        <div className={`cm-tasks-search${ui.search ? " active" : ""}`}>
          <input
            ref={searchRef}
            placeholder={`Search ${title}`}
            value={ui.search}
            onChange={(e) => ui.setSearch(e.target.value)}
            onKeyDown={(e) => e.key === "Escape" && ui.setSearch("")}
            dir="auto"
          />
          {ui.search ? (
            <button title="Clear search" onClick={() => ui.setSearch("")}>
              <Icon name="close" size={10} />
            </button>
          ) : (
            <Icon name="find" size={14} />
          )}
        </div>
      </div>
      {search && <div className="cm-tasks-searchbar">{items.length ? `Search results for "${search}"` : `No matches for "${search}".`}</div>}
      <TaskTable
        items={items}
        view={ui.view}
        multiFolder={multiFolder}
        manualOrder={manualOrder}
        addTarget={
          ui.view === "completed"
            ? null
            : {
                listId:
                  list?.id ?? (ui.folder.kind === "account" ? lists.find((l) => l.accountId === (ui.folder as { accountId: number }).accountId && l.isDefault)?.id : undefined),
              }
        }
        lists={lists}
        emptyText={q.isLoading ? "Loading…" : search ? "We didn't find anything to show here." : "There are no items to show in this view."}
      />
      {/* Categorize ▸ All Categories… */}
      <ColorCategoriesHost />
    </>
  );
}

// ---- toolbar & menus ---------------------------------------------------------------------------

function newTaskHere() {
  const f = useTasksUi.getState().folder;
  newTask(f.kind === "list" ? { listId: f.listId } : {});
}

function ModToolbar() {
  const sel = useSelectedItems();
  const lists = useTaskLists().data ?? [];
  const tasksSel = sel.filter((i) => i.kind === "task");
  return (
    <Toolbar>
      <TbSplit icon="new" label="New" title="New Task (Ctrl+N)" menu={newItemMenu} onClick={newTaskHere} />
      <TbButton icon="print" title="Print (Ctrl+P)" onClick={() => window.dispatchEvent(new CustomEvent("cm-print"))} />
      <TbDropdown icon="move" title="Move to Folder" disabled={!tasksSel.length} menu={() => moveMenu(tasksSel, lists)} />
      <TbButton icon="delete" title="Delete (Ctrl+D)" disabled={!tasksSel.length} onClick={() => deleteItems(tasksSel)} />
      <TbSep />
      <TbButton icon="complete-task" label="Mark Complete" disabled={!sel.length} onClick={() => setComplete(sel, true)} />
      <TbDropdown icon="flag" label="Follow Up" disabled={!sel.length} menu={() => followUpMenu(sel)} />
      <TbDropdown icon="categorize" title="Categorize" disabled={!tasksSel.length} menu={() => categorizeMenu(tasksSel)} />
      <TbSep />
      <TbButton icon="send-receive" label="Send/Receive" title="Send/Receive All Folders (F9)" onClick={() => rpc("sync.now", {})} />
      <TbButton icon="find" label="Search" title="Instant Search (Ctrl+E)" onClick={() => window.dispatchEvent(new CustomEvent("cm-focus-search"))} />
      <TbButton icon="address-book" title="Address Book (Ctrl+Shift+B)" onClick={() => openWindow("address-book", {}, { label: "address-book", title: "Address Book" })} />
    </Toolbar>
  );
}

/** Current selection / lists, read when a menu opens (no React hooks: MainWindow calls
 * `useMenus` conditionally per module, so it must not change the hook order). */
function current() {
  const ui = useTasksUi.getState();
  const set = new Set(ui.selection);
  const sel = useViewItems.getState().items.filter((i) => set.has(i.key));
  const lists = queryClient.getQueryData<TaskList[]>(["tasks", "lists"]) ?? [];
  return { view: ui.view, sel, tasksSel: sel.filter((i) => i.kind === "task"), lists };
}

function useMenus(): ModuleMenus {
  return {
    get edit(): MenuItem[] {
      const { sel, tasksSel, lists } = current();
      return [
        { label: "Select All", shortcut: "Ctrl+A", onClick: () => useTasksUi.getState().select(useViewItems.getState().items.map((i) => i.key)) },
        { type: "separator" },
        { label: "Delete", icon: "delete", shortcut: "Ctrl+D", disabled: !tasksSel.length, onClick: () => deleteItems(tasksSel) },
        { label: "Move to Folder", icon: "move", disabled: !tasksSel.length, submenu: moveMenu(tasksSel, lists) },
        { type: "separator" },
        { label: "Categorize", icon: "categorize", disabled: !tasksSel.length, submenu: categorizeMenu(tasksSel) },
        { label: "Mark Complete", icon: "complete-task", shortcut: "Ins", disabled: !sel.length, onClick: () => setComplete(sel, true) },
      ];
    },
    get view(): MenuItem[] {
      const { view } = current();
      return [
        {
          label: "Current View",
          submenu: VIEWS.map((v) => ({ label: v.label, radio: true, checked: v.id === view, onClick: () => setView(v.id) })),
        },
      ];
    },
    get actions(): MenuItem[] {
      const { sel, tasksSel, lists } = current();
      return [
        { label: "New Task", icon: "task", shortcut: "Ctrl+N", onClick: newTaskHere },
        { label: "New Task Request", icon: "task", shortcut: "Ctrl+Shift+U", disabled: true },
        { type: "separator" },
        { label: "Mark Complete", icon: "complete-task", disabled: !sel.length, onClick: () => setComplete(sel, true) },
        { label: "Follow Up", icon: "flag", disabled: !sel.length, submenu: followUpMenu(sel) },
        { label: "Categorize", icon: "categorize", disabled: !tasksSel.length, submenu: categorizeMenu(tasksSel) },
        { type: "separator" },
        { label: "New Folder…", icon: "new-folder", onClick: () => newFolder(lists) },
      ];
    },
  };
}

export const tasksModule: ModuleDef = {
  id: "tasks",
  label: "Tasks",
  icon: "tasks",
  NavPane,
  View,
  Toolbar: ModToolbar,
  useMenus,
  newItem: newTaskHere,
};
