// The Outlook 2007 task table: column headers (click to sort), "Click here to add a new Task",
// optional group headers, overdue rows in red, completed struck through, multi-select, drag to
// reorder (manual order) or onto a folder, right-click menus, double-click to open.
import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import { tasksApi, type TaskList, type TaskView } from "../../api/tasks";
import { Icon, contextMenu, errorBox, showMenu, useHotkeys } from "../../ui";
import { DatePicker } from "./DatePicker";
import { DUE_GROUPS, dueGroup, flagColor, fmtDate, isOverdue, todayKey } from "./dates";
import { BellGlyph, CategoryChips, CheckGlyph, FlagGlyph, MailGlyph, TaskGlyph } from "./icons";
import { compareItems, useTasksUi, type ColId, type TodoItem } from "./store";
import { deleteItems, followUpMenu, itemMenu, openItem, setComplete, toggleComplete } from "./actions";

interface Col {
  id: ColId;
  label: string;
  width: number | "flex";
  title?: string;
  glyph?: "icon" | "priority" | "complete" | "flag";
}

const C: Record<ColId, Col> = {
  icon: { id: "icon", label: "", width: 22, title: "Icon", glyph: "icon" },
  priority: { id: "priority", label: "", width: 18, title: "Priority", glyph: "priority" },
  complete: { id: "complete", label: "", width: 22, title: "Complete", glyph: "complete" },
  subject: { id: "subject", label: "Subject", width: "flex" },
  status: { id: "status", label: "Status", width: 120 },
  start: { id: "start", label: "Start Date", width: 108 },
  due: { id: "due", label: "Due Date", width: 108 },
  percent: { id: "percent", label: "% Complete", width: 76 },
  categories: { id: "categories", label: "Categories", width: 110 },
  folder: { id: "folder", label: "In Folder", width: 112 },
  flag: { id: "flag", label: "", width: 24, title: "Flag Status", glyph: "flag" },
};

export function columnsFor(view: TaskView, multiFolder: boolean): Col[] {
  const ids: ColId[] =
    view === "simple"
      ? ["icon", "complete", "subject", ...(multiFolder ? (["folder"] as ColId[]) : []), "due", "flag"]
      : view === "todo"
        ? ["icon", "priority", "subject", "start", "due", "folder", "categories", "flag"]
        : ["icon", "priority", "complete", "subject", "status", "due", "percent", "categories", ...(multiFolder ? (["folder"] as ColId[]) : []), "flag"];
  return ids.map((i) => C[i]);
}

interface Group {
  id: string;
  label: string;
  items: TodoItem[];
}

export function TaskTable({
  items,
  view,
  multiFolder,
  manualOrder,
  addTarget,
  lists,
  emptyText,
}: {
  items: TodoItem[];
  view: TaskView;
  multiFolder: boolean;
  /** rows can be dragged to reorder */
  manualOrder: boolean;
  /** list for "Click here to add a new Task" (undefined = default list) */
  addTarget: { listId?: number } | null;
  lists: TaskList[];
  emptyText: string;
}) {
  const ui = useTasksUi();
  const cols = columnsFor(view, multiFolder);
  const template = cols.map((c) => (c.width === "flex" ? "minmax(120px, 1fr)" : `${c.width}px`)).join(" ");
  const bodyRef = useRef<HTMLDivElement>(null);

  // sort + group
  const groups: Group[] = useMemo(() => {
    // subtasks are indented only in the plain (folder order) list
    const nested = !ui.sort && view !== "todo" && view !== "category";
    let rows = nested ? items : items.map((i) => (i.depth ? { ...i, depth: 0 } : i));
    const sort = ui.sort ?? (view === "todo" ? { col: "due" as ColId, dir: 1 as const } : null);
    if (sort) rows = [...rows].sort((a, b) => compareItems(a, b, sort));
    if (view === "todo") {
      return DUE_GROUPS.map((g) => ({ id: g.id, label: g.label, items: rows.filter((r) => dueGroup(r.due) === g.id) })).filter((g) => g.items.length);
    }
    if (view === "category") {
      const names = [...new Set(rows.flatMap((r) => r.categories))].sort();
      const gs = names.map((n) => ({ id: `cat:${n}`, label: `Categories: ${n}`, items: rows.filter((r) => r.categories.includes(n)) }));
      const none = rows.filter((r) => !r.categories.length);
      if (none.length) gs.unshift({ id: "cat:", label: "Categories: (none)", items: none });
      return gs;
    }
    return [{ id: "", label: "", items: rows }];
  }, [items, ui.sort, view]);

  const flat = useMemo(() => groups.flatMap((g) => (g.id && ui.collapsed.includes(g.id) ? [] : g.items)), [groups, ui.collapsed]);
  const byKey = useMemo(() => new Map(items.map((i) => [i.key, i])), [items]);
  const selected = ui.selection.map((k) => byKey.get(k)).filter((x): x is TodoItem => !!x);
  const selSet = new Set(ui.selection);

  // keep the selection valid when items change
  useEffect(() => {
    const valid = ui.selection.filter((k) => byKey.has(k));
    if (valid.length !== ui.selection.length) ui.select(valid);
  }, [byKey]); // eslint-disable-line react-hooks/exhaustive-deps

  const clickRow = (e: React.MouseEvent, it: TodoItem) => {
    if (e.shiftKey && ui.anchor) {
      const a = flat.findIndex((x) => x.key === ui.anchor);
      const b = flat.findIndex((x) => x.key === it.key);
      if (a >= 0 && b >= 0) {
        const [lo, hi] = a < b ? [a, b] : [b, a];
        ui.select(
          flat.slice(lo, hi + 1).map((x) => x.key),
          ui.anchor,
        );
        return;
      }
    }
    if (e.ctrlKey) {
      ui.select(selSet.has(it.key) ? ui.selection.filter((k) => k !== it.key) : [...ui.selection, it.key], it.key);
      return;
    }
    if (e.button === 2 && selSet.has(it.key)) return; // right-click keeps a multi-selection
    ui.select([it.key], it.key);
  };

  const moveFocus = (delta: number, extend: boolean) => {
    if (!flat.length) return;
    const cur = flat.findIndex((x) => x.key === (ui.selection[ui.selection.length - 1] ?? ""));
    const next = Math.max(0, Math.min(flat.length - 1, cur < 0 ? 0 : cur + delta));
    const k = flat[next].key;
    if (extend && ui.anchor) {
      const a = flat.findIndex((x) => x.key === ui.anchor);
      const [lo, hi] = a < next ? [a, next] : [next, a];
      // keep the moving end last so the next arrow continues from it
      const sel = flat
        .slice(lo, hi + 1)
        .map((x) => x.key)
        .filter((x) => x !== k);
      ui.select([...sel, k], ui.anchor);
    } else ui.select([k], k);
    bodyRef.current?.querySelector(`[data-key="${k}"]`)?.scrollIntoView({ block: "nearest" });
  };

  // the table has focus, or nothing does (after a toolbar click, say): not the To-Do Bar or a box
  const focused = () => document.activeElement === bodyRef.current || document.activeElement === document.body;
  useHotkeys({
    delete: () => focused() && deleteItems(selected),
    "ctrl+d": () => focused() && deleteItems(selected),
    insert: () => focused() && toggleComplete(selected),
    enter: () => focused() && selected.slice(0, 5).forEach(openItem),
    arrowdown: () => focused() && moveFocus(1, false),
    arrowup: () => focused() && moveFocus(-1, false),
    "shift+arrowdown": () => focused() && moveFocus(1, true),
    "shift+arrowup": () => focused() && moveFocus(-1, true),
    home: () => focused() && moveFocus(-1e6, false),
    end: () => focused() && moveFocus(1e6, false),
    "ctrl+a": () =>
      focused() &&
      ui.select(
        flat.map((x) => x.key),
        flat[0]?.key ?? null,
      ),
  });

  // drag & drop
  const [dropAt, setDropAt] = useState<{ key: string; after: boolean } | null>(null);
  const onDrop = async (target: TodoItem, after: boolean, payload: string) => {
    setDropAt(null);
    let p: { kind?: string; ids?: number[] };
    try {
      p = JSON.parse(payload);
    } catch {
      return;
    }
    if (p.kind !== "tasks" || !p.ids?.length || target.kind !== "task" || !target.task) return;
    const moving = new Set(p.ids);
    if (moving.has(target.id)) return;
    const t = target.task;
    // siblings in display order (same parent), without the moved rows
    const sibs = flat.filter((x) => x.task && x.task.parentId === t.parentId && x.task.listId === t.listId && !moving.has(x.id));
    const ti = sibs.findIndex((x) => x.id === target.id);
    const previousId = after ? target.id : ti > 0 ? sibs[ti - 1].id : null;
    // keep the visual order of the dragged rows
    const order = flat.filter((x) => moving.has(x.id)).map((x) => x.id);
    await tasksApi
      .move({ ids: order, listId: t.listId, parentId: t.parentId, previousId }, order.length === 1 ? "Move 1 task" : `Move ${order.length} tasks`)
      .catch((e) => errorBox(e));
  };

  const sortBy = (c: Col) => {
    const s = ui.sort;
    ui.setSort(s && s.col === c.id ? { col: c.id, dir: s.dir === 1 ? -1 : 1 } : { col: c.id, dir: 1 });
  };

  const cell = (c: Col, it: TodoItem) => {
    const overdue = isOverdue(it.due, it.complete);
    switch (c.id) {
      case "icon":
        return it.kind === "mail" ? <MailGlyph /> : <TaskGlyph done={it.complete} />;
      case "priority":
        return it.importance === "high" ? <Icon name="importance-high" size={12} /> : it.importance === "low" ? <Icon name="importance-low" size={12} /> : null;
      case "complete":
        return (
          <span
            className="cm-tasks-check"
            title={it.complete ? "Mark incomplete" : "Mark complete"}
            onMouseDown={(e) => e.stopPropagation()}
            onClick={(e) => {
              e.stopPropagation();
              setComplete([it], !it.complete);
            }}
          >
            <CheckGlyph checked={it.complete} />
          </span>
        );
      case "subject":
        return (
          <span className="cm-ellipsis" style={{ paddingLeft: it.depth * 16 }} dir="auto">
            {it.task?.reminderAt && !it.complete ? (
              <span className="cm-tasks-bell" title="Reminder">
                <BellGlyph />
              </span>
            ) : null}
            {it.title || <span className="cm-muted">(no subject)</span>}
          </span>
        );
      case "status":
        return <span className="cm-ellipsis">{it.status}</span>;
      case "start":
        return <span className="cm-ellipsis">{fmtDate(it.start)}</span>;
      case "due":
        return <span className={`cm-ellipsis${overdue ? " cm-tasks-red" : ""}`}>{fmtDate(it.due)}</span>;
      case "percent":
        return <span>{it.percent}%</span>;
      case "categories":
        return <CategoryChips names={it.categories} />;
      case "folder":
        return <span className="cm-ellipsis">{it.folder}</span>;
      case "flag":
        return (
          <span
            className="cm-tasks-flag"
            title={it.complete ? "Completed" : "Click to mark complete; right-click to change the flag"}
            onMouseDown={(e) => e.stopPropagation()}
            onClick={(e) => {
              e.stopPropagation();
              setComplete([it], !it.complete);
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              showMenu(e.clientX, e.clientY, followUpMenu([it]));
            }}
          >
            <FlagGlyph complete={it.complete} color={flagColor(it.due)} />
          </span>
        );
    }
  };

  const total = flat.length;
  return (
    <div className="cm-tasks-table">
      <div className="cm-tasks-head" style={{ gridTemplateColumns: template }}>
        {cols.map((c) => (
          <div
            key={c.id}
            className={`cm-tasks-th${c.glyph ? " glyph" : ""}${ui.sort?.col === c.id ? " sorted" : ""}`}
            title={c.title ?? c.label}
            onClick={() => sortBy(c)}
            onContextMenu={contextMenu(() => [
              { label: "Sort Ascending", icon: "arrow-up", onClick: () => ui.setSort({ col: c.id, dir: 1 }) },
              { label: "Sort Descending", icon: "arrow-down", onClick: () => ui.setSort({ col: c.id, dir: -1 }) },
              { type: "separator" },
              { label: view === "todo" ? "Arrange By Due Date" : "Original Order", disabled: !ui.sort, onClick: () => ui.setSort(null) },
            ])}
          >
            {c.glyph === "icon" && <TaskGlyph size={13} />}
            {c.glyph === "priority" && <span className="cm-tasks-th-bang">!</span>}
            {c.glyph === "complete" && <CheckGlyph checked />}
            {c.glyph === "flag" && <FlagGlyph size={13} color="#9a9a9a" />}
            {!c.glyph && <span className="cm-ellipsis">{c.label}</span>}
            {ui.sort?.col === c.id && !c.glyph && <span className="cm-tasks-sortarrow">{ui.sort.dir === 1 ? "▲" : "▼"}</span>}
          </div>
        ))}
      </div>
      <div
        ref={bodyRef}
        className="cm-tasks-body"
        tabIndex={0}
        onMouseDown={(e) => {
          if (e.target === e.currentTarget) ui.select([]);
        }}
      >
        {addTarget && <AddRow template={template} cols={cols} target={addTarget} view={view} />}
        {total === 0 && <div className="cm-tasks-empty">{emptyText}</div>}
        {groups.map((g) => {
          const collapsed = !!g.id && ui.collapsed.includes(g.id);
          return (
            <Fragment key={g.id || "all"}>
              {g.id && (
                <div className="cm-tasks-group" onClick={() => ui.toggleGroup(g.id)}>
                  <span className="cm-tasks-group-toggle">{collapsed ? "+" : "−"}</span>
                  <span className="cm-tasks-group-label">{g.label}</span>
                  <span className="cm-tasks-group-count">
                    ({g.items.length} item{g.items.length === 1 ? "" : "s"})
                  </span>
                </div>
              )}
              {!collapsed &&
                g.items.map((it) => {
                  const overdue = isOverdue(it.due, it.complete);
                  const drop = dropAt?.key === it.key ? (dropAt.after ? " drop-after" : " drop-before") : "";
                  return (
                    <div
                      key={`${g.id}/${it.key}`}
                      data-key={it.key}
                      className={`cm-tasks-row${selSet.has(it.key) ? " selected" : ""}${it.complete ? " done" : ""}${overdue ? " overdue" : ""}${drop}`}
                      style={{ gridTemplateColumns: template }}
                      draggable={it.kind === "task"}
                      onMouseDown={(e) => clickRow(e, it)}
                      onDoubleClick={() => openItem(it)}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        const sel = selSet.has(it.key) ? selected : [it];
                        if (!selSet.has(it.key)) ui.select([it.key], it.key);
                        showMenu(e.clientX, e.clientY, itemMenu(sel, lists));
                      }}
                      onDragStart={(e) => {
                        const sel = (selSet.has(it.key) ? selected : [it]).filter((x) => x.kind === "task");
                        e.dataTransfer.setData("application/x-cm-items", JSON.stringify({ kind: "tasks", ids: sel.map((x) => x.id) }));
                        e.dataTransfer.setData("text/plain", sel.map((x) => x.title).join("\n"));
                        e.dataTransfer.effectAllowed = "move";
                      }}
                      onDragOver={(e) => {
                        if (!manualOrder || it.kind !== "task" || !e.dataTransfer.types.includes("application/x-cm-items")) return;
                        e.preventDefault();
                        const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
                        const after = e.clientY > r.top + r.height / 2;
                        if (dropAt?.key !== it.key || dropAt.after !== after) setDropAt({ key: it.key, after });
                      }}
                      onDragLeave={() => setDropAt((d) => (d?.key === it.key ? null : d))}
                      onDrop={(e) => {
                        const payload = e.dataTransfer.getData("application/x-cm-items");
                        if (!payload || !manualOrder) return;
                        e.preventDefault();
                        const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
                        onDrop(it, e.clientY > r.top + r.height / 2, payload);
                      }}
                      onDragEnd={() => setDropAt(null)}
                    >
                      {cols.map((c) => (
                        <div key={c.id} className={`cm-tasks-td${c.glyph ? " glyph" : ""} col-${c.id}`}>
                          {cell(c, it)}
                        </div>
                      ))}
                    </div>
                  );
                })}
            </Fragment>
          );
        })}
      </div>
    </div>
  );
}

/** "Click here to add a new Task" → inline subject + due date editor; Enter saves. */
function AddRow({ template, cols, target, view }: { template: string; cols: Col[]; target: { listId?: number }; view: TaskView }) {
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState("");
  const [due, setDue] = useState<string | null>(view === "todo" || view === "next7" ? todayKey() : null);
  const [busy, setBusy] = useState(false);
  const save = async () => {
    if (!title.trim() || busy) {
      setEditing(false);
      return;
    }
    setBusy(true);
    try {
      await tasksApi.create({ listId: target.listId, title: title.trim(), due }, `New Task "${title.trim()}"`);
      setTitle("");
    } catch (e) {
      // the typed subject stays, so nothing is lost
      errorBox(e);
    } finally {
      setBusy(false);
    }
  };
  const keys = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      save();
    } else if (e.key === "Escape") {
      setTitle("");
      setEditing(false);
    }
  };
  if (!editing)
    return (
      <div className="cm-tasks-addrow" style={{ gridTemplateColumns: template }} onMouseDown={(e) => (e.preventDefault(), setEditing(true))}>
        {cols.map((c) => (
          <div key={c.id} className={`cm-tasks-td${c.glyph ? " glyph" : ""}`}>
            {c.id === "subject" ? <span className="cm-tasks-addhint">Click here to add a new Task</span> : c.id === "icon" ? <TaskGlyph /> : null}
          </div>
        ))}
      </div>
    );
  return (
    <div
      className="cm-tasks-addrow editing"
      style={{ gridTemplateColumns: template }}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node) && !document.querySelector(".cm-tasks-dropcal")) {
          if (title.trim()) save().then(() => setEditing(false));
          else setEditing(false);
        }
      }}
    >
      {cols.map((c) => (
        <div key={c.id} className={`cm-tasks-td${c.glyph ? " glyph" : ""}`}>
          {c.id === "icon" && <TaskGlyph />}
          {c.id === "subject" && (
            <input
              className="cm-input cm-tasks-cellinput"
              autoFocus
              value={title}
              placeholder="Click here to add a new Task"
              onChange={(e) => setTitle(e.target.value)}
              onKeyDown={keys}
              dir="auto"
            />
          )}
          {c.id === "due" && <DatePicker value={due} onChange={setDue} width={c.width as number} onKeyDown={keys} className="cm-tasks-celldate" />}
        </div>
      ))}
    </div>
  );
}
