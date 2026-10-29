// Folder-tree control (mail folders, calendars, contact folders, task lists).
import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { Icon } from "./icons";
import type { MenuItem } from "./menu";
import { contextMenu } from "./menu";

export interface TreeNode {
  id: string;
  label: string;
  icon?: string;
  /** unread count shown as "(12)" in blue, or total shown as "[3]" for Drafts/Outbox */
  count?: number;
  countStyle?: "unread" | "total";
  bold?: boolean;
  children?: TreeNode[];
  /** extra element to the left of the label (e.g. a checkbox for calendars) */
  prefix?: ReactNode;
  /** colour swatch (calendars) */
  color?: string;
  title?: string;
  data?: unknown;
}

export function TreeView({
  nodes,
  selectedId,
  onSelect,
  expanded,
  onToggle,
  menuFor,
  onDropItems,
  canDrop,
  rootClassName,
  onActivate,
  autoFocus,
  onReorder,
  canReorder,
  dragItem,
  onDragItem,
}: {
  nodes: TreeNode[];
  selectedId?: string | null;
  onSelect?: (n: TreeNode) => void;
  /** controlled expansion; uncontrolled (all open) when omitted */
  expanded?: Set<string>;
  onToggle?: (id: string, open: boolean) => void;
  menuFor?: (n: TreeNode) => MenuItem[];
  /** drag & drop: messages / items dropped on a node (dataTransfer type "application/x-cm-items") */
  onDropItems?: (n: TreeNode, payload: string, copy: boolean) => void;
  canDrop?: (n: TreeNode) => boolean;
  rootClassName?: string;
  /** Enter on the selected node (keyboard) */
  onActivate?: (n: TreeNode) => void;
  /** focus the tree when it mounts (dialogs) */
  autoFocus?: boolean;
  /** press-and-hold a row to lift it and drag it up/down among its siblings; gets the new order */
  onReorder?: (parentId: string | null, orderedIds: string[]) => void;
  canReorder?: (n: TreeNode) => boolean;
  /** rows that can be dragged onto other rows (a quick drag; press-and-hold still reorders): the
   *  "application/x-cm-items" payload for a row, or null when it can't be dragged */
  dragItem?: (n: TreeNode) => string | null;
  /** the row being dragged (null when the drag ends), so canDrop can judge targets meanwhile */
  onDragItem?: (n: TreeNode | null) => void;
}) {
  // keyboard: Up/Down/Home/End move the selection over visible nodes, Left/Right collapse/expand,
  // Enter activates, letters jump to the next node starting with them (like the Windows tree view)
  const typed = useRef({ s: "", t: 0 });
  const visible = (): { n: TreeNode; parent: TreeNode | null }[] => {
    const out: { n: TreeNode; parent: TreeNode | null }[] = [];
    const walk = (list: TreeNode[], parent: TreeNode | null) => {
      for (const n of list) {
        out.push({ n, parent });
        if (n.children?.length && (expanded ? expanded.has(n.id) : true)) walk(n.children, n);
      }
    };
    walk(nodes, null);
    return out;
  };
  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    const vis = visible();
    if (!vis.length) return;
    const cur = vis.findIndex((v) => v.n.id === selectedId);
    const go = (i: number) => {
      const v = vis[Math.max(0, Math.min(vis.length - 1, i))];
      if (v && v.n.id !== selectedId) onSelect?.(v.n);
      requestAnimationFrame(() => (e.currentTarget as HTMLElement | null)?.querySelector?.(".cm-tree-row.selected")?.scrollIntoView({ block: "nearest" }));
    };
    const target = e.currentTarget;
    const page = Math.max(1, Math.floor(target.clientHeight / 20) - 1);
    const handled = () => {
      e.preventDefault();
      e.stopPropagation();
    };
    const cn = cur >= 0 ? vis[cur].n : null;
    const isOpen = (n: TreeNode) => (expanded ? expanded.has(n.id) : true);
    switch (e.key) {
      case "ArrowDown":
        handled();
        return go(cur + 1);
      case "ArrowUp":
        handled();
        return go(cur < 0 ? 0 : cur - 1);
      case "PageDown":
        handled();
        return go(cur + page);
      case "PageUp":
        handled();
        return go(cur - page);
      case "Home":
        handled();
        return go(0);
      case "End":
        handled();
        return go(vis.length - 1);
      case "ArrowRight":
        handled();
        if (cn?.children?.length) {
          if (!isOpen(cn)) onToggle?.(cn.id, true);
          else go(cur + 1);
        }
        return;
      case "ArrowLeft": {
        handled();
        if (!cn) return;
        if (cn.children?.length && isOpen(cn)) onToggle?.(cn.id, false);
        else {
          const parent = vis[cur].parent;
          if (parent) go(vis.findIndex((v) => v.n.id === parent.id));
        }
        return;
      }
      case "Enter":
        if (cn && onActivate) {
          handled();
          onActivate(cn);
        }
        return;
    }
    if (e.key.length === 1 && e.key !== " ") {
      const now = Date.now();
      const t = typed.current;
      t.s = now - t.t < 800 ? t.s + e.key.toLowerCase() : e.key.toLowerCase();
      t.t = now;
      // pressing the same letter again cycles through the nodes starting with it
      if ([...t.s].every((c) => c === t.s[0])) t.s = t.s[0];
      const start = t.s.length > 1 ? Math.max(cur, 0) : cur + 1;
      for (let k = 0; k < vis.length; k++) {
        const v = vis[(start + k) % vis.length];
        if (v.n.label.toLowerCase().startsWith(t.s)) {
          handled();
          return go((start + k) % vis.length);
        }
      }
    }
  };
  const rootRef = useRef<HTMLDivElement>(null);
  const reorder = useReorder(rootRef, onReorder, canReorder);
  return (
    <ReorderCtx.Provider value={reorder}>
      <div
        ref={rootRef}
        className={`cm-tree ${rootClassName ?? ""}${reorder.drag ? " reordering" : ""}`}
        role="tree"
        tabIndex={0}
        onKeyDown={onKeyDown}
        data-autofocus={autoFocus ? "" : undefined}
      >
        {nodes.map((n) => (
          <TreeRow
            key={n.id}
            n={n}
            depth={0}
            parentId={null}
            siblings={nodes}
            {...{ selectedId, onSelect, expanded, onToggle, menuFor, onDropItems, canDrop, dragItem, onDragItem }}
          />
        ))}
      </div>
    </ReorderCtx.Provider>
  );
}

// ---- press-and-hold reordering ---------------------------------------------------------------

type Drag = { id: string; parentId: string | null; siblingIds: string[]; startY: number; dy: number; target: number };

interface ReorderApi {
  drag: Drag | null;
  /** pointer down on a row: starts the hold timer */
  press: (e: React.PointerEvent, n: TreeNode, parentId: string | null, siblings: TreeNode[]) => void;
  /** a drag of the row itself started: no lifting */
  cancel: () => void;
  enabled: boolean;
}

const ReorderCtx = createContext<ReorderApi>({ drag: null, press: () => {}, cancel: () => {}, enabled: false });

/** how long to hold the mouse button before the row lifts */
const HOLD_MS = 450;

function useReorder(
  rootRef: React.RefObject<HTMLDivElement | null>,
  onReorder?: (parentId: string | null, ids: string[]) => void,
  canReorder?: (n: TreeNode) => boolean,
): ReorderApi {
  const [drag, setDrag] = useState<Drag | null>(null);
  const dragRef = useRef<Drag | null>(null);
  dragRef.current = drag;
  const hold = useRef<{ timer: number; x: number; y: number } | null>(null);

  const cancelHold = () => {
    if (hold.current) window.clearTimeout(hold.current.timer);
    hold.current = null;
  };

  // slot among the siblings (the dragged one left out) under the pointer
  const targetFor = (d: Drag, y: number): number => {
    const el0 = rootRef.current;
    if (!el0) return d.target;
    let idx = 0;
    for (const id of d.siblingIds) {
      if (id === d.id) continue;
      const el = el0.querySelector<HTMLElement>(`[data-tree-id="${CSS.escape(id)}"]`);
      if (!el) continue;
      const r = el.getBoundingClientRect();
      if (y > r.top + r.height / 2) idx++;
    }
    return idx;
  };

  useEffect(() => {
    if (!drag) return;
    const move = (e: PointerEvent) => {
      const d = dragRef.current;
      if (!d) return;
      setDrag({ ...d, dy: e.clientY - d.startY, target: targetFor(d, e.clientY) });
    };
    const up = () => {
      const d = dragRef.current;
      setDrag(null);
      if (!d || !onReorder) return;
      const rest = d.siblingIds.filter((id) => id !== d.id);
      rest.splice(d.target, 0, d.id);
      if (rest.join("|") !== d.siblingIds.join("|")) onReorder(d.parentId, rest);
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setDrag(null);
      }
    };
    // the click that follows the drop must not select or open anything
    const swallow = (e: MouseEvent) => {
      e.stopPropagation();
      e.preventDefault();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up, { once: true });
    window.addEventListener("keydown", key, true);
    window.addEventListener("click", swallow, { capture: true, once: true });
    document.body.style.cursor = "grabbing";
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("keydown", key, true);
      setTimeout(() => window.removeEventListener("click", swallow, { capture: true }), 0);
      document.body.style.cursor = "";
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [!!drag]);

  const press = (e: React.PointerEvent, n: TreeNode, parentId: string | null, siblings: TreeNode[]) => {
    if (!onReorder || e.button !== 0 || siblings.length < 2 || (canReorder && !canReorder(n))) return;
    cancelHold();
    const x = e.clientX;
    const y = e.clientY;
    const ids = siblings.map((sib) => sib.id);
    const timer = window.setTimeout(() => {
      hold.current = null;
      setDrag({ id: n.id, parentId, siblingIds: ids, startY: y, dy: 0, target: ids.indexOf(n.id) });
    }, HOLD_MS);
    hold.current = { timer, x, y };
    const moveEarly = (ev: PointerEvent) => {
      if (hold.current && Math.hypot(ev.clientX - hold.current.x, ev.clientY - hold.current.y) > 6) cancelHold();
    };
    const release = () => {
      cancelHold();
      window.removeEventListener("pointermove", moveEarly);
    };
    window.addEventListener("pointermove", moveEarly);
    window.addEventListener("pointerup", release, { once: true });
  };

  return { drag, press, cancel: cancelHold, enabled: !!onReorder };
}

function TreeRow({
  n,
  depth,
  parentId,
  siblings,
  selectedId,
  onSelect,
  expanded,
  onToggle,
  menuFor,
  onDropItems,
  canDrop,
  dragItem,
  onDragItem,
}: {
  n: TreeNode;
  depth: number;
  parentId: string | null;
  siblings: TreeNode[];
  selectedId?: string | null;
  onSelect?: (n: TreeNode) => void;
  expanded?: Set<string>;
  onToggle?: (id: string, open: boolean) => void;
  menuFor?: (n: TreeNode) => MenuItem[];
  onDropItems?: (n: TreeNode, payload: string, copy: boolean) => void;
  canDrop?: (n: TreeNode) => boolean;
  dragItem?: (n: TreeNode) => string | null;
  onDragItem?: (n: TreeNode | null) => void;
}) {
  const [dropHover, setDropHover] = useState(false);
  const hasKids = !!n.children?.length;
  const open = expanded ? expanded.has(n.id) : true;
  const selected = selectedId === n.id;
  const ro = useContext(ReorderCtx);
  const d = ro.drag;
  const lifted = d?.id === n.id;
  const payload = dragItem?.(n) ?? null;
  // insertion marker: before the sibling at the target slot (or after the last one)
  let mark = "";
  if (d && d.parentId === parentId && !lifted) {
    const rest = d.siblingIds.filter((id) => id !== d.id);
    const i = rest.indexOf(n.id);
    if (i === d.target) mark = " insert-before";
    else if (i === rest.length - 1 && d.target === rest.length) mark = " insert-after";
  }
  return (
    <>
      <div
        role="treeitem"
        aria-selected={selected}
        className={`cm-tree-row${selected ? " selected" : ""}${dropHover ? " drop" : ""}${lifted ? " lifted" : ""}${mark}`}
        data-tree-id={n.id}
        style={{ paddingLeft: 4 + depth * 14, ...(lifted ? { transform: `translateY(${d!.dy}px)` } : null) }}
        onPointerDown={ro.enabled ? (e) => ro.press(e, n, parentId, siblings) : undefined}
        draggable={payload !== null}
        onDragStart={
          payload !== null
            ? (e) => {
                // held long enough to lift: that is a reorder, not a drag onto another row
                if (d) return e.preventDefault();
                ro.cancel();
                e.dataTransfer.setData("application/x-cm-items", payload);
                e.dataTransfer.effectAllowed = "move";
                onDragItem?.(n);
              }
            : undefined
        }
        onDragEnd={payload !== null ? () => onDragItem?.(null) : undefined}
        title={n.title}
        onMouseDown={() => onSelect?.(n)}
        onDoubleClick={() => hasKids && onToggle?.(n.id, !open)}
        onContextMenu={menuFor ? contextMenu(() => menuFor(n)) : undefined}
        onDragOver={(e) => {
          if (!onDropItems || !e.dataTransfer.types.includes("application/x-cm-items")) return;
          if (canDrop && !canDrop(n)) return;
          e.preventDefault();
          e.dataTransfer.dropEffect = e.ctrlKey ? "copy" : "move";
          setDropHover(true);
        }}
        onDragLeave={() => setDropHover(false)}
        onDrop={(e) => {
          setDropHover(false);
          const payload = e.dataTransfer.getData("application/x-cm-items");
          if (payload && onDropItems) {
            e.preventDefault();
            onDropItems(n, payload, e.ctrlKey);
          }
        }}
      >
        <span
          className="cm-tree-toggle"
          onMouseDown={(e) => {
            e.stopPropagation();
            if (hasKids) onToggle?.(n.id, !open);
          }}
        >
          {hasKids && <Icon name={open ? "tree-expanded" : "tree-collapsed"} size={10} />}
        </span>
        {n.prefix}
        {n.color && <span className="cm-tree-swatch" style={{ background: n.color }} />}
        {n.icon && <Icon name={n.icon} />}
        <span className={`cm-tree-label${n.bold || (n.countStyle !== "total" && (n.count ?? 0) > 0) ? " bold" : ""}`}>{n.label}</span>
        {(n.count ?? 0) > 0 && <span className="cm-tree-count">{n.countStyle === "total" ? `[${n.count}]` : `(${n.count})`}</span>}
      </div>
      {hasKids &&
        open &&
        n.children!.map((c) => (
          <TreeRow
            key={c.id}
            n={c}
            depth={depth + 1}
            parentId={n.id}
            siblings={n.children!}
            {...{ selectedId, onSelect, expanded, onToggle, menuFor, onDropItems, canDrop, dragItem, onDragItem }}
          />
        ))}
    </>
  );
}
