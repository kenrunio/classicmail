// Format Text › Table on any table on the compose page (your reply's or the original's), done on
// the DOM table: shading, borders, rows & columns, merge / split, header row, alignment and sizes.
// The compose window runs each change through `changeUndoably` (body.ts) so Ctrl+Z undoes it.

export type BorderKind = "all" | "none" | "outside" | "inside" | "top" | "bottom" | "left" | "right";
export interface Line {
  width: number;
  color: string;
}
export const lineCss = (l: Line) => `${l.width}px solid ${l.color}`;

const SIDES = ["Top", "Right", "Bottom", "Left"] as const;
type Side = (typeof SIDES)[number];

export interface Rect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}
type Cell = HTMLTableCellElement;

/** Which sides of a cell at `at` get the border, for a `kind` applied over `rect`. */
export function sidesFor(kind: BorderKind, at: Rect, rect: Rect): Side[] {
  const edge = { Top: at.top === rect.top, Bottom: at.bottom === rect.bottom, Left: at.left === rect.left, Right: at.right === rect.right };
  switch (kind) {
    case "all":
    case "none":
      return [...SIDES];
    case "outside":
      return SIDES.filter((s) => edge[s]);
    case "inside":
      return SIDES.filter((s) => !edge[s]);
    default:
      return SIDES.filter((s) => s.toLowerCase() === kind && edge[s]);
  }
}

/** A DOM table as a grid: grid[row][col] is the cell covering that slot (spans repeat it). */
export interface Grid {
  table: HTMLTableElement;
  grid: Cell[][];
  width: number;
}

export function gridOf(table: HTMLTableElement): Grid {
  const grid: Cell[][] = [];
  Array.from(table.rows).forEach((tr, r) => {
    grid[r] ??= [];
    let c = 0;
    for (const cell of Array.from(tr.cells)) {
      while (grid[r][c]) c++;
      for (let dr = 0; dr < Math.max(1, cell.rowSpan); dr++)
        for (let dc = 0; dc < Math.max(1, cell.colSpan); dc++) {
          grid[r + dr] ??= [];
          grid[r + dr][c + dc] = cell;
        }
      c += Math.max(1, cell.colSpan);
    }
  });
  const rows = grid.slice(0, table.rows.length);
  return { table, grid: rows, width: Math.max(0, ...rows.map((r) => r.length)) };
}

/** Where a cell sits in the grid. */
export function rectOf(g: Grid, cell: Cell): Rect {
  let left = Infinity,
    top = Infinity,
    right = -1,
    bottom = -1;
  g.grid.forEach((row, r) =>
    row.forEach((x, c) => {
      if (x !== cell) return;
      left = Math.min(left, c);
      top = Math.min(top, r);
      right = Math.max(right, c + 1);
      bottom = Math.max(bottom, r + 1);
    }),
  );
  return { left, top, right, bottom };
}

/** The cells covering `rect`, each once, in grid order. */
export function cellsIn(g: Grid, rect: Rect): Cell[] {
  const out: Cell[] = [];
  for (let r = rect.top; r < rect.bottom; r++) for (let c = rect.left; c < rect.right; c++) if (g.grid[r]?.[c] && !out.includes(g.grid[r][c])) out.push(g.grid[r][c]);
  return out;
}

export interface TableSel {
  g: Grid;
  rect: Rect;
  /** more than one cell selected (else: the cell at the cursor) */
  multi: boolean;
}

const cellOf = (n: Node | null, root: Element): Cell | null => {
  const el = n && (n.nodeType === Node.ELEMENT_NODE ? (n as Element) : n.parentElement);
  const c = el?.closest("td, th") as Cell | null;
  return c && root.contains(c) ? c : null;
};

/** The table cells a selection on the page covers (a rectangle, like Word), or null when it
 *  isn't in a table. */
export function tableSelection(range: Range, root: Element): TableSel | null {
  const a = cellOf(range.startContainer, root);
  const b = cellOf(range.endContainer, root) ?? a;
  if (!a || !b) return null;
  const table = a.closest("table") as HTMLTableElement;
  if (b.closest("table") !== table) return null;
  const g = gridOf(table);
  const ra = rectOf(g, a);
  const rb = rectOf(g, b);
  const rect = { left: Math.min(ra.left, rb.left), top: Math.min(ra.top, rb.top), right: Math.max(ra.right, rb.right), bottom: Math.max(ra.bottom, rb.bottom) };
  return { g, rect, multi: a !== b };
}

const whole = (s: TableSel): Rect => (s.multi ? s.rect : { left: 0, top: 0, right: s.g.width, bottom: s.g.grid.length });

export function shadeCells(s: TableSel, color: string | null) {
  for (const c of cellsIn(s.g, s.rect)) c.style.backgroundColor = color ?? "";
}

export function alignCells(s: TableSel, v: "top" | "middle" | "bottom") {
  for (const c of cellsIn(s.g, s.rect)) c.style.verticalAlign = v;
}

const SIDE_PROP = { Top: "borderTop", Right: "borderRight", Bottom: "borderBottom", Left: "borderLeft" } as const;
const OPPOSITE = { Top: "Bottom", Bottom: "Top", Left: "Right", Right: "Left" } as const;

/** Borders on the selected cells, or the whole table with none selected; the cell
 *  across a shared edge gets the same line. */
export function setBorders(s: TableSel, kind: BorderKind, line: Line) {
  const rect = whole(s);
  const css = kind === "none" ? "none" : lineCss(line);
  // single lines between cells, as in Word (a table from elsewhere may draw them doubled)
  s.g.table.style.borderCollapse = "collapse";
  for (const c of cellsIn(s.g, rect)) {
    const at = rectOf(s.g, c);
    for (const side of sidesFor(kind, at, rect)) {
      c.style[SIDE_PROP[side]] = css;
      const r = side === "Top" ? at.top - 1 : side === "Bottom" ? at.bottom : at.top;
      const col = side === "Left" ? at.left - 1 : side === "Right" ? at.right : at.left;
      const n = s.g.grid[r]?.[col];
      if (n && n !== c) n.style[SIDE_PROP[OPPOSITE[side]]] = css;
    }
  }
}

/** An empty copy of a cell (its formatting, no text). */
function blank(c: Cell, rowSpan = 1, colSpan = 1): Cell {
  const n = c.cloneNode(false) as Cell;
  n.rowSpan = rowSpan;
  n.colSpan = colSpan;
  n.innerHTML = "<br>";
  return n;
}

export function insertRow(s: TableSel, where: "above" | "below") {
  const { g } = s;
  const base = where === "above" ? s.rect.top : s.rect.bottom - 1;
  const tr = g.table.rows[base];
  const row = tr.cloneNode(false) as HTMLTableRowElement;
  const seen = new Set<Cell>();
  for (let c = 0; c < g.width; c++) {
    const cell = g.grid[base]?.[c];
    if (!cell || seen.has(cell)) continue;
    seen.add(cell);
    const at = rectOf(g, cell);
    // a cell spanning across the new row's place grows instead
    if ((where === "below" && at.bottom > base + 1) || (where === "above" && at.top < base)) cell.rowSpan += 1;
    else row.appendChild(blank(cell, 1, at.right - at.left));
  }
  tr.parentElement!.insertBefore(row, where === "above" ? tr : tr.nextSibling);
}

export function insertCol(s: TableSel, where: "left" | "right") {
  const { g } = s;
  const col = where === "left" ? s.rect.left : s.rect.right - 1;
  const seen = new Set<Cell>();
  g.grid.forEach((row) => {
    const cell = row[col];
    if (!cell || seen.has(cell)) return;
    seen.add(cell);
    const at = rectOf(g, cell);
    if ((where === "right" && at.right > col + 1) || (where === "left" && at.left < col)) cell.colSpan += 1;
    else cell.parentElement!.insertBefore(blank(cell, at.bottom - at.top), where === "left" ? cell : cell.nextSibling);
  });
}

export function deleteRows(s: TableSel) {
  const { g, rect } = s;
  if (rect.top === 0 && rect.bottom >= g.grid.length) return deleteTable(s);
  for (const cell of cellsIn(g, { left: 0, right: g.width, top: rect.top, bottom: rect.bottom })) {
    const at = rectOf(g, cell);
    if (at.top >= rect.top && at.bottom <= rect.bottom) continue; // goes with its row
    cell.rowSpan -= Math.min(at.bottom, rect.bottom) - Math.max(at.top, rect.top);
    // a spanning cell that starts in a deleted row moves down to the first kept row
    if (at.top >= rect.top) {
      const next = g.table.rows[rect.bottom];
      next.insertBefore(cell, Array.from(next.cells).find((x) => rectOf(g, x).left > at.left) ?? null);
    }
  }
  for (let r = rect.bottom - 1; r >= rect.top; r--) g.table.rows[r].remove();
}

export function deleteCols(s: TableSel) {
  const { g, rect } = s;
  if (rect.left === 0 && rect.right >= g.width) return deleteTable(s);
  for (const cell of cellsIn(g, { left: rect.left, right: rect.right, top: 0, bottom: g.grid.length })) {
    const at = rectOf(g, cell);
    const inside = Math.min(at.right, rect.right) - Math.max(at.left, rect.left);
    if (inside >= at.right - at.left) cell.remove();
    else cell.colSpan -= inside;
  }
}

export function deleteTable(s: TableSel) {
  s.g.table.remove();
}

/** Merge the selected rectangle into its first cell (content kept, one paragraph per cell). */
export function mergeCells(s: TableSel): boolean {
  const { g, rect } = s;
  const all = cellsIn(g, rect);
  if (all.length < 2) return false;
  // only when no cell sticks out of the rectangle
  if (
    all.some((c) => {
      const at = rectOf(g, c);
      return at.left < rect.left || at.top < rect.top || at.right > rect.right || at.bottom > rect.bottom;
    })
  )
    return false;
  const [first, ...rest] = all;
  for (const c of rest) {
    if (c.textContent?.trim()) {
      first.appendChild(document.createElement("br"));
      while (c.firstChild) first.appendChild(c.firstChild);
    }
    c.remove();
  }
  first.colSpan = rect.right - rect.left;
  first.rowSpan = rect.bottom - rect.top;
  return true;
}

/** What the ribbon needs to know about a selection, as a string that changes when that does. */
export const selKey = (s: TableSel | null) => (s ? `${s.multi ? "cells" : "cell"}${canSplit(s) ? " merged" : ""}` : "");

export const canSplit = (s: TableSel) => !s.multi && cellsIn(s.g, s.rect).some((c) => c.colSpan > 1 || c.rowSpan > 1);

/** Split a merged cell back into single cells. */
export function splitCell(s: TableSel): boolean {
  const { g } = s;
  const cell = cellsIn(g, s.rect)[0];
  if (!cell || (cell.colSpan <= 1 && cell.rowSpan <= 1)) return false;
  const at = rectOf(g, cell);
  const cols = at.right - at.left;
  for (let r = at.top; r < at.bottom; r++) {
    const tr = g.table.rows[r];
    if (r === at.top) {
      for (let i = 1; i < cols; i++) tr.insertBefore(blank(cell), cell.nextSibling);
    } else {
      const after = Array.from(tr.cells).find((x) => rectOf(g, x).left > at.left) ?? null;
      for (let i = 0; i < cols; i++) tr.insertBefore(blank(cell), after);
    }
  }
  cell.colSpan = 1;
  cell.rowSpan = 1;
  return true;
}

/** First row as header cells (th) or back to ordinary ones. */
export function toggleHeaderRow(s: TableSel) {
  const first = s.g.table.rows[0];
  if (!first) return;
  const toHeader = Array.from(first.cells).some((c) => c.tagName === "TD");
  for (const c of Array.from(first.cells)) {
    const n = document.createElement(toHeader ? "th" : "td");
    for (const a of Array.from(c.attributes)) n.setAttribute(a.name, a.value);
    while (c.firstChild) n.appendChild(c.firstChild);
    c.replaceWith(n);
  }
}

/** Column widths / row heights (grid index → px) for a table. */
export function sizeTable(table: HTMLTableElement, sizes: { cols?: Map<number, number>; rows?: Map<number, number> }) {
  const g = gridOf(table);
  const seen = new Set<Cell>();
  g.grid.forEach((row) =>
    row.forEach((c) => {
      if (!c || seen.has(c)) return;
      seen.add(c);
      const at = rectOf(g, c);
      const w = c.colSpan === 1 ? sizes.cols?.get(at.left) : undefined;
      if (w !== undefined) {
        c.style.width = `${w}px`;
        c.removeAttribute("width");
      }
      const h = c.rowSpan === 1 ? sizes.rows?.get(at.top) : undefined;
      if (h !== undefined) {
        c.style.height = `${h}px`;
        c.removeAttribute("height");
      }
    }),
  );
  // a <colgroup> (Outlook / Word tables) would win over the cells
  table.querySelectorAll("col").forEach((col, i) => {
    const w = sizes.cols?.get(i);
    if (w !== undefined) {
      col.style.width = `${w}px`;
      col.removeAttribute("width");
    }
  });
  if (sizes.cols?.size) table.style.width = "";
}

/** A new table (Insert › Table). */
export function tableHtml(rows: number, cols: number): string {
  const td = '<td style="border:1px solid #808080;padding:2pt 4pt;min-width:3em;vertical-align:top"><br></td>';
  return `<table style="border-collapse:collapse"><tbody>${`<tr>${td.repeat(cols)}</tr>`.repeat(rows)}</tbody></table><p><br></p>`;
}
