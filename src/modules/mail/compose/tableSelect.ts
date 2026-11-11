// Selecting table rows and columns in the compose body, like Word: a selection across two or more
// rows takes the whole rows; a click just left of a row picks the row, just above a column the
// column (drag on for more). Picked cells are shaded. Backspace deletes the picked rows /
// columns; Delete empties the picked cells. The caller runs each change (one undo step).
import { cellsIn, deleteCols, deleteRows, gridOf, rectOf, tableSelection, type Grid, type Rect } from "./pageTables";

/** How far outside a table's left / top edge a click picks a row / column (px). */
const ZONE = 14;

export interface Pick {
  table: HTMLTableElement;
  rect: Rect;
  /** whole rows / columns (Backspace deletes them); null: just these cells */
  whole: "rows" | "cols" | null;
}

/** What a selection over `rect` picks: across two or more rows, the whole rows; else the cells. */
export function pickFor(g: Grid, rect: Rect): Omit<Pick, "table"> {
  if (rect.bottom - rect.top >= 2) return { rect: { left: 0, right: g.width, top: rect.top, bottom: rect.bottom }, whole: "rows" };
  if (rect.left === 0 && rect.right >= g.width) return { rect, whole: "rows" };
  return { rect, whole: null };
}

/** Row or column under a pointer just outside a table's left / top edge. */
function edgeAt(x: number, y: number, root: Element): { table: HTMLTableElement; kind: "rows" | "cols"; index: number } | null {
  for (const table of Array.from(root.querySelectorAll("table"))) {
    const t = table.getBoundingClientRect();
    if (y >= t.top && y < t.bottom && x >= t.left - ZONE && x < t.left + 2) {
      const i = Array.from(table.rows).findIndex((r) => {
        const b = r.getBoundingClientRect();
        return y >= b.top && y < b.bottom;
      });
      if (i >= 0) return { table, kind: "rows", index: i };
    }
    if (x >= t.left && x < t.right && y >= t.top - ZONE && y < t.top + 2) {
      const g = gridOf(table);
      const col = (g.grid[0] ?? []).findIndex((c) => {
        const b = c.getBoundingClientRect();
        return x >= b.left && x < b.right;
      });
      if (col >= 0) return { table, kind: "cols", index: rectOf(g, g.grid[0][col]).left };
    }
  }
  return null;
}

/** The page's selection set to cover the picked cells (so Copy and the Table menu see them). */
function selectCells(p: Pick) {
  const cells = cellsIn(gridOf(p.table), p.rect);
  if (!cells.length) return;
  const r = document.createRange();
  r.setStart(cells[0], 0);
  r.setEnd(cells[cells.length - 1], cells[cells.length - 1].childNodes.length);
  const s = window.getSelection();
  s?.removeAllRanges();
  s?.addRange(r);
}

export function attachTableSelect(root: HTMLElement, change: (table: HTMLTableElement, f: () => void) => void): () => void {
  let pick: Pick | null = null;
  // a pick made by a click on a row / column edge, kept while the selection is the one it set
  let held: Pick | null = null;
  let drag: { table: HTMLTableElement; kind: "rows" | "cols"; from: number } | null = null;
  const shades: HTMLDivElement[] = [];
  let frame = 0;

  const paint = () => {
    frame = 0;
    const cells = pick && pick.table.isConnected ? cellsIn(gridOf(pick.table), pick.rect) : [];
    while (shades.length > cells.length) shades.pop()!.remove();
    while (shades.length < cells.length) {
      const d = document.createElement("div");
      d.className = "cm-table-pick";
      document.body.appendChild(d);
      shades.push(d);
    }
    const view = root.closest(".cm-compose-body")?.getBoundingClientRect();
    cells.forEach((c, i) => {
      const b = c.getBoundingClientRect();
      // clipped to the scrolling text area (not over the header)
      const top = view ? Math.max(b.top, view.top) : b.top;
      const bottom = view ? Math.min(b.bottom, view.bottom) : b.bottom;
      Object.assign(shades[i].style, { left: `${b.left}px`, top: `${top}px`, width: `${b.width}px`, height: `${Math.max(0, bottom - top)}px` });
    });
    if (pick) frame = requestAnimationFrame(paint);
  };
  const set = (p: Pick | null) => {
    pick = p;
    if (p && !frame) frame = requestAnimationFrame(paint);
    if (!p) {
      cancelAnimationFrame(frame);
      frame = 0;
      paint();
    }
  };
  const sameRect = (a: Rect, b: Rect) => a.left === b.left && a.top === b.top && a.right === b.right && a.bottom === b.bottom;

  const onSelection = () => {
    if (drag) return;
    const s = window.getSelection();
    if (!s?.rangeCount || s.isCollapsed || !root.contains(s.anchorNode)) {
      held = null;
      return set(null);
    }
    const ts = tableSelection(s.getRangeAt(0), root);
    if (!ts || !ts.multi) {
      held = null;
      return set(null);
    }
    if (held && held.table === ts.g.table && sameRect(cellRect(held), ts.rect)) return set(held);
    held = null;
    set({ table: ts.g.table, ...pickFor(ts.g, ts.rect) });
  };
  // the rectangle a pick's selection reports (its first to last cell)
  const cellRect = (p: Pick): Rect => {
    const g = gridOf(p.table);
    const cells = cellsIn(g, p.rect);
    const a = rectOf(g, cells[0]);
    const b = rectOf(g, cells[cells.length - 1]);
    return { left: Math.min(a.left, b.left), top: Math.min(a.top, b.top), right: Math.max(a.right, b.right), bottom: Math.max(a.bottom, b.bottom) };
  };

  const edgePick = (table: HTMLTableElement, kind: "rows" | "cols", a: number, b: number): Pick => {
    const g = gridOf(table);
    const lo = Math.min(a, b);
    const hi = Math.max(a, b) + 1;
    return { table, whole: kind, rect: kind === "rows" ? { left: 0, right: g.width, top: lo, bottom: hi } : { left: lo, right: hi, top: 0, bottom: g.grid.length } };
  };
  const hover = (e: MouseEvent) => {
    if (drag) {
      const edge = edgeAt(e.clientX, e.clientY, root);
      const at = edge && edge.table === drag.table && edge.kind === drag.kind ? edge.index : overAt(e, drag);
      if (at === null) return;
      held = edgePick(drag.table, drag.kind, drag.from, at);
      set(held);
      selectCells(held);
      return;
    }
    const at = e.buttons ? null : edgeAt(e.clientX, e.clientY, root);
    document.body.classList.toggle("cm-pick-row", at?.kind === "rows");
    document.body.classList.toggle("cm-pick-col", at?.kind === "cols");
  };
  // dragging on from an edge into the table: the row / column under the pointer
  const overAt = (e: MouseEvent, d: NonNullable<typeof drag>): number | null => {
    const cell = (e.target as Element | null)?.closest?.("td, th") as HTMLTableCellElement | null;
    if (!cell || cell.closest("table") !== d.table) return null;
    const r = rectOf(gridOf(d.table), cell);
    return d.kind === "rows" ? r.top : r.left;
  };
  const down = (e: MouseEvent) => {
    if (e.button !== 0) return;
    const at = edgeAt(e.clientX, e.clientY, root);
    if (!at) return;
    e.preventDefault();
    e.stopPropagation();
    root.focus({ preventScroll: true });
    drag = { table: at.table, kind: at.kind, from: at.index };
    held = edgePick(at.table, at.kind, at.index, at.index);
    set(held);
    selectCells(held);
  };
  const up = () => {
    drag = null;
  };
  const key = (e: KeyboardEvent) => {
    if (!pick || (e.key !== "Backspace" && e.key !== "Delete") || e.ctrlKey || e.altKey) return;
    const p = pick;
    e.preventDefault();
    e.stopPropagation();
    const g = gridOf(p.table);
    const sel = { g, rect: p.rect, multi: true };
    // where the cursor goes after: the cell now at the first picked one's place (or after the table)
    const after = () => {
      const s = window.getSelection();
      const r = document.createRange();
      const g2 = p.table.isConnected ? gridOf(p.table) : null;
      const cell = g2?.grid[Math.min(p.rect.top, g2.grid.length - 1)]?.[Math.min(p.rect.left, g2.width - 1)];
      if (cell) r.setStart(cell, 0);
      else return;
      r.collapse(true);
      s?.removeAllRanges();
      s?.addRange(r);
    };
    held = null;
    set(null);
    if (e.key === "Backspace" && p.whole) {
      change(p.table, () => (p.whole === "rows" ? deleteRows(sel) : deleteCols(sel)));
    } else {
      change(p.table, () => {
        for (const c of cellsIn(g, p.rect)) c.innerHTML = "<br>";
      });
    }
    after();
  };

  document.addEventListener("selectionchange", onSelection);
  root.addEventListener("mousedown", down, true);
  root.addEventListener("keydown", key, true);
  window.addEventListener("mousemove", hover);
  window.addEventListener("mouseup", up);
  return () => {
    document.removeEventListener("selectionchange", onSelection);
    root.removeEventListener("mousedown", down, true);
    root.removeEventListener("keydown", key, true);
    window.removeEventListener("mousemove", hover);
    window.removeEventListener("mouseup", up);
    document.body.classList.remove("cm-pick-row", "cm-pick-col");
    set(null);
  };
}
