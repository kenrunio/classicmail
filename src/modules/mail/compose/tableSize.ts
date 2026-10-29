// Table sizes in the compose body, like Word: drag a column's or row's line with the mouse, and
// "Even Out" (all columns after the first as wide as the widest cell, all rows after the header
// as tall as the tallest). Works on the DOM table; the caller applies a size (pageTables.ts).
import { gridOf, rectOf } from "./pageTables";

const EDGE = 4; // px from a line that counts as "on" it

export interface ResizeTarget {
  table: HTMLTableElement;
  kind: "col" | "row";
  /** the column whose right line / the row whose bottom line is dragged */
  index: number;
}

/** The line under the pointer, if any: a cell's right / bottom edge (or left / top edge, which is
 *  the previous column's / row's). */
export function lineAt(e: MouseEvent, root: Element): ResizeTarget | null {
  const cell = (e.target as Element | null)?.closest?.("td, th") as HTMLTableCellElement | null;
  if (!cell || !root.contains(cell)) return null;
  const r = cell.getBoundingClientRect();
  const near = (a: number, b: number) => Math.abs(a - b) <= EDGE;
  // the table's grid only when the pointer is on a line: a big table costs a walk of every cell
  if (!near(e.clientX, r.right) && !near(e.clientX, r.left) && !near(e.clientY, r.bottom) && !near(e.clientY, r.top)) return null;
  const table = cell.closest("table") as HTMLTableElement;
  const at = rectOf(gridOf(table), cell);
  if (near(e.clientX, r.right)) return { table, kind: "col", index: at.right - 1 };
  if (near(e.clientX, r.left) && at.left > 0) return { table, kind: "col", index: at.left - 1 };
  if (near(e.clientY, r.bottom)) return { table, kind: "row", index: at.bottom - 1 };
  if (near(e.clientY, r.top) && at.top > 0) return { table, kind: "row", index: at.top - 1 };
  return null;
}

/** Where a column starts (left x) / a row starts (top y) on screen. */
function startOf(t: ResizeTarget): number {
  if (t.kind === "row") return t.table.rows[t.index].getBoundingClientRect().top;
  const g = gridOf(t.table);
  const one = g.grid.map((row) => row[t.index]).find((c) => c && c.colSpan === 1) ?? g.grid[0][t.index];
  return one.getBoundingClientRect().left;
}

/** The width / height to set on a cell for an outer size: the same when cells are sized with
 *  their padding and borders (box-sizing: border-box, as in the compose window), else without. */
function inner(cell: HTMLElement, outer: number, kind: "col" | "row"): number {
  const cs = getComputedStyle(cell);
  const f = (p: string) => parseFloat(cs.getPropertyValue(p)) || 0;
  const extra =
    cs.boxSizing === "border-box"
      ? 0
      : kind === "col"
        ? f("padding-left") + f("padding-right") + f("border-left-width") + f("border-right-width")
        : f("padding-top") + f("padding-bottom") + f("border-top-width") + f("border-bottom-width");
  // a width rounded down would wrap the widest text: round up, with a pixel to spare
  return Math.max(kind === "col" ? 16 : 8, kind === "col" ? Math.ceil(outer - extra) + 1 : Math.round(outer - extra));
}

/**
 * Mouse resizing on every table in `root`: the cursor changes on a line, a drag shows a guide and
 * `apply` gets the new content size (px).
 */
export function attachResizer(root: HTMLElement, apply: (t: ResizeTarget, px: number) => void): () => void {
  let hover: ResizeTarget | null = null;
  let drag: { t: ResizeTarget; start: number; guide: HTMLDivElement } | null = null;
  const cursor = (k: "col" | "row" | null) => {
    document.body.classList.toggle("cm-resize-col", k === "col");
    document.body.classList.toggle("cm-resize-row", k === "row");
  };
  const move = (e: MouseEvent) => {
    if (drag) {
      const tr = drag.t.table.getBoundingClientRect();
      Object.assign(
        drag.guide.style,
        drag.t.kind === "col"
          ? { left: `${e.clientX}px`, top: `${tr.top}px`, height: `${tr.height}px`, width: "0" }
          : { top: `${e.clientY}px`, left: `${tr.left}px`, width: `${tr.width}px`, height: "0" },
      );
      return;
    }
    hover = lineAt(e, root);
    cursor(hover?.kind ?? null);
  };
  const down = (e: MouseEvent) => {
    if (!hover || e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const guide = document.createElement("div");
    guide.className = `cm-resize-guide ${hover.kind}`;
    document.body.appendChild(guide);
    drag = { t: hover, start: startOf(hover), guide };
    move(e);
  };
  const up = (e: MouseEvent) => {
    if (!drag) return;
    const { t, start, guide } = drag;
    drag = null;
    guide.remove();
    cursor(null);
    const g = gridOf(t.table);
    const cell = t.kind === "col" ? g.grid.map((row) => row[t.index]).find(Boolean) : g.grid[t.index]?.find(Boolean);
    if (!cell) return;
    const outer = (t.kind === "col" ? e.clientX : e.clientY) - start;
    apply(t, inner(cell, outer, t.kind));
  };
  // mousemove on the window only (moves over the page reach it too)
  root.addEventListener("mousedown", down, true);
  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", up);
  return () => {
    root.removeEventListener("mousedown", down, true);
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", up);
    cursor(null);
  };
}

export interface Even {
  /** content width for every column after the first (null: only one column) */
  width: number | null;
  /** content height for every row after the header (null: nothing to even out) */
  height: number | null;
  /** the first row is a header row (th), left as it is */
  header: boolean;
}

/**
 * Sizes that make a table tidy: measured on an unsized copy with the same fonts and cell CSS
 * (so the live table on the page isn't touched), the widest cell of the
 * columns after the first sets their width, then the tallest row after the header sets the rows.
 */
export function measureEven(table: HTMLTableElement): Even {
  const host = document.createElement("div");
  const cs = getComputedStyle(table);
  host.className = "cm-compose-page";
  Object.assign(host.style, {
    position: "absolute",
    left: "-20000px",
    top: "0",
    visibility: "hidden",
    fontFamily: cs.fontFamily,
    fontSize: cs.fontSize,
    lineHeight: cs.lineHeight,
  });
  const clone = table.cloneNode(true) as HTMLTableElement;
  clone.style.width = "";
  clone.removeAttribute("width");
  clone.querySelectorAll("col").forEach((c) => {
    c.removeAttribute("style");
    c.removeAttribute("width");
  });
  clone.querySelectorAll<HTMLElement>("td, th").forEach((c) => {
    c.style.width = "";
    c.style.height = "";
    c.removeAttribute("width");
    c.removeAttribute("height");
  });
  host.appendChild(clone);
  document.body.appendChild(host);
  try {
    const g = gridOf(clone);
    const header = !!clone.rows[0] && Array.from(clone.rows[0].cells).some((c) => c.tagName === "TH");
    const rest = (col: number) => g.grid.map((row) => row[col]).filter((c, i, a) => c && c.colSpan === 1 && a.indexOf(c) === i);
    let width: number | null = null;
    if (g.width > 1) {
      const cells = Array.from({ length: g.width - 1 }, (_, i) => rest(i + 1)).flat();
      const widest = cells.reduce((m, c) => Math.max(m, c.getBoundingClientRect().width), 0);
      if (cells.length && widest) {
        width = inner(cells[0], widest, "col");
        for (const c of cells) c.style.width = `${width}px`;
      }
    }
    const start = header ? 1 : 0;
    let height: number | null = null;
    if (clone.rows.length > start + 1 || (clone.rows.length > start && !header)) {
      const rows = Array.from(clone.rows).slice(start);
      const tallest = rows.reduce((m, r) => Math.max(m, r.getBoundingClientRect().height), 0);
      const cell = rows[0].cells[0];
      if (tallest && cell) height = inner(cell, tallest, "row");
    }
    return { width, height, header };
  } finally {
    host.remove();
  }
}
