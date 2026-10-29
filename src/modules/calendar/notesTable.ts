// Phone-friendly tables in appointment notes. Event notes are plain text (Apple Calendar shows
// nothing else, and a 9-column table can't fit a phone anyway), so a pasted table — tab-separated
// lines from Excel / a web page / an e-mail, or a Markdown table — becomes one block per row:
//
//   2. Bluewater Fund (Boston) — James Whitaker
//      Last met Orion: Not in CRM
//      $ NGEV at Q2-end: $84.2M
//
// Text around the table is left as it is.

export type Grid = string[][];

const DASH = /^[\s—–‒-]*$/;
const blank = (s: string | undefined) => !s || DASH.test(s);

/** A row-number column: titled "#" / "No." etc., or untitled and counting 1, 2, 3… (a column of
 *  years or amounts is data, not numbering). */
function isIndexColumn(header: string, values: string[]): boolean {
  const h = header.trim().toLowerCase();
  if (["#", "no", "no.", "nr", "num", "№"].includes(h)) return true;
  if (h) return false;
  const nums = values.filter((v) => v.trim()).map((v) => Number(v.trim().replace(/\.$/, "")));
  return nums.length > 0 && nums.every((n, i) => Number.isInteger(n) && n === nums[0] + i) && (nums[0] === 0 || nums[0] === 1);
}

/** Starts a list made by `tableToBlocks`: the column titles, so the list can be turned back into a
 *  table (a grid) later. */
export const LIST_HEADER = "Table header: ";
const SEP = " | ";
const TITLE_SEP = " — ";

/** Which columns make the title line: the first (or, with 4+ columns, the first two) non-index columns. */
function titleColumns(cols: number[]): number[] {
  return cols.length >= 4 ? cols.slice(0, 2) : cols.slice(0, 1);
}

/**
 * One numbered block per data row (the first row holds the column titles), under a
 * "Table header: # | Firm | …" line; blocks separated by a blank line. Rows are always numbered
 * (the index column's value, else 1, 2, 3…) so the list's end can be found again, and an empty
 * title is "—" (never borrowed from another column) so `findLists` gets the same cells back.
 */
export function tableToBlocks(grid: Grid): string {
  const [headers, ...rows] = grid;
  if (!headers || !rows.length) return "";
  const width = Math.max(headers.length, ...rows.map((r) => r.length));
  const cell = (r: string[], i: number) => (r[i] ?? "").trim();
  const index = isIndexColumn(
    cell(headers, 0),
    rows.map((r) => cell(r, 0)),
  )
    ? 0
    : -1;
  const head = Array.from({ length: width }, (_, i) => cell(headers, i) || (i === index ? "#" : `Column ${i + 1}`));
  const cols = head.map((_, i) => i).filter((i) => i !== index);
  const titled = titleColumns(cols);
  const rest = cols.filter((i) => !titled.includes(i));
  const blocks: string[] = [];
  let n = 0;
  for (const r of rows) {
    if (r.every((c) => blank(c))) continue;
    n++;
    const title =
      titled
        .map((i) => cell(r, i))
        .filter((v) => !blank(v))
        .join(TITLE_SEP) || "—";
    // the row's own # (a blank one stays blank: "—."), else 1, 2, 3… when the table has no # column
    const num = index >= 0 ? cell(r, index).replace(/\.$/, "") || "—" : String(n);
    const lines = [`${num}. ${title}`];
    for (const i of rest) {
      const v = cell(r, i);
      if (!blank(v)) lines.push(`   ${head[i]}: ${v}`);
    }
    blocks.push(lines.join("\n"));
  }
  return `${LIST_HEADER}${head.join(SEP)}\n\n${blocks.join("\n\n")}`;
}

export interface FoundList {
  from: number;
  to: number;
  grid: Grid;
}

/** Lists made by `tableToBlocks` (they start with a "Table header:" line), read back into cells. */
export function findLists(lines: string[]): FoundList[] {
  const out: FoundList[] = [];
  for (let h = 0; h < lines.length; h++) {
    if (!lines[h].startsWith(LIST_HEADER)) continue;
    const head = lines[h]
      .slice(LIST_HEADER.length)
      .split(SEP)
      .map((s) => s.trim());
    if (head.length < 2) continue;
    const index = isIndexColumn(head[0], []) ? 0 : -1;
    const cols = head.map((_, i) => i).filter((i) => i !== index);
    const titled = titleColumns(cols);
    const rest = cols.filter((i) => !titled.includes(i)).sort((a, b) => head[b].length - head[a].length);
    const rows: string[][] = [];
    let i = h + 1;
    let end = h + 1;
    while (i < lines.length) {
      while (i < lines.length && !lines[i].trim()) i++;
      const m = i < lines.length ? /^(\S+?)\. (.*)$/.exec(lines[i]) : null;
      if (!m) break;
      const row = head.map(() => "");
      if (index >= 0) row[index] = m[1] === "—" ? "" : m[1];
      const title = m[2].trim();
      if (title !== "—") {
        const parts = titled.length === 2 ? splitOnce(title, TITLE_SEP) : [title];
        titled.forEach((c, k) => (row[c] = parts[k] ?? ""));
      }
      i++;
      while (i < lines.length && lines[i].startsWith("   ") && lines[i].trim()) {
        const line = lines[i].trimStart();
        const c = rest.find((r) => line.startsWith(`${head[r]}: `));
        if (c !== undefined) row[c] = line.slice(head[c].length + 2).trim();
        i++;
      }
      rows.push(row);
      end = i;
    }
    if (rows.length) {
      out.push({ from: h, to: end, grid: [head, ...rows] });
      h = end - 1;
    }
  }
  return out;
}

function splitOnce(s: string, sep: string): [string, string] {
  const k = s.indexOf(sep);
  return k < 0 ? [s, ""] : [s.slice(0, k), s.slice(k + sep.length)];
}

const isTsvLine = (l: string) => l.includes("\t") && l.split("\t").some((c) => c.trim());
const isMdLine = (l: string) => /^\s*\|.*\|\s*$/.test(l);
const isMdSeparator = (l: string) => /^\s*\|?(\s*:?-{3,}:?\s*\|)+\s*(:?-{3,}:?\s*)?\|?\s*$/.test(l);

function mdCells(l: string): string[] {
  return l
    .trim()
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|")
    .map((c) => c.trim());
}

interface Found {
  /** line range [from, to) */
  from: number;
  to: number;
  grid: Grid;
}

/** Tables (tab-separated runs of 2+ lines, or Markdown tables) in a text, by line. */
export function findTables(lines: string[]): Found[] {
  const out: Found[] = [];
  let i = 0;
  while (i < lines.length) {
    if (isMdLine(lines[i]) && i + 1 < lines.length && isMdSeparator(lines[i + 1])) {
      let j = i + 2;
      while (j < lines.length && isMdLine(lines[j])) j++;
      const grid = [mdCells(lines[i]), ...lines.slice(i + 2, j).map(mdCells)];
      if (grid.length >= 2) out.push({ from: i, to: j, grid });
      i = j;
      continue;
    }
    if (isTsvLine(lines[i])) {
      let j = i + 1;
      while (j < lines.length && isTsvLine(lines[j])) j++;
      if (j - i >= 2) out.push({ from: i, to: j, grid: lines.slice(i, j).map((l) => l.split("\t")) });
      i = j;
      continue;
    }
    i++;
  }
  return out;
}

export type TidyStyle = "list" | "grid";

/** Raw tables, grids and (reversible) lists in a text. */
export function tableKinds(text: string): { raw: number; grid: number; list: number } {
  const lines = text.split(/\r?\n/);
  return { raw: findTables(lines).length, grid: findGrids(lines).length, list: findLists(lines).length };
}

/** The text with every table — pasted ones, and grids / lists made earlier — turned into row
 *  blocks (`list`) or a fixed-width grid (`grid`); `count` tables were converted. */
export function tidyNotes(text: string, style: TidyStyle = "list"): { text: string; count: number } {
  const nl = text.includes("\r\n") ? "\r\n" : "\n";
  const lines = text.split(/\r?\n/);
  const other: { from: number; to: number; grid: Grid }[] = style === "list" ? findGrids(lines) : findLists(lines);
  const found = [...findTables(lines), ...other].sort((a, b) => a.from - b.from).filter((f, k, all) => k === 0 || f.from >= all[k - 1].to);
  if (!found.length) return { text, count: 0 };
  const out: string[] = [];
  let at = 0;
  for (const f of found) {
    out.push(...lines.slice(at, f.from));
    // a blank line between the table and the text around it
    if (out.length && out[out.length - 1].trim()) out.push("");
    const grid = pruneEmptyColumns(f.grid);
    out.push(...(style === "grid" ? tableToGrid(grid) : tableToBlocks(grid)).split("\n"));
    if (f.to < lines.length && lines[f.to].trim()) out.push("");
    at = f.to;
  }
  out.push(...lines.slice(at));
  return { text: out.join(nl), count: found.length };
}

/** Drop columns that are empty from top to bottom (a trailing tab on every pasted line). */
export function pruneEmptyColumns(grid: Grid): Grid {
  const width = Math.max(0, ...grid.map((r) => r.length));
  const keep = Array.from({ length: width }, (_, i) => i).filter((i) => grid.some((r) => (r[i] ?? "").trim()));
  return grid.map((r) => keep.map((i) => r[i] ?? ""));
}

// ---- fixed-width grid ----------------------------------------------------------------------------
//
//   # | Firm               | Investor      | CRM
//     |                    |               | mtgs
//   --+--------------------+---------------+-----
//   1 | Westpark           | Daniel Grant  |    0
//
// Long cells wrap inside their column; a continuation line has an empty first column (a row
// whose own first cell is empty gets "—" so it never looks like a continuation). The rule line's
// "+" marks give the column positions back to `findGrids`, which ClassicMail uses to show the
// grid as a real table.

/** Longest a column gets before its cells wrap. */
export const GRID_MAX_COL = 22;
const NUMERIC = /^[-−+]?[$€£₪]?[\d.,]+[%kKmMbB]?$/;

function wrap(text: string, width: number): string[] {
  const words = text.trim().split(/\s+/).filter(Boolean);
  const lines: string[] = [];
  let cur = "";
  for (let w of words) {
    while (w.length > width) {
      // a single word longer than the column: hard split
      if (cur) {
        lines.push(cur);
        cur = "";
      }
      lines.push(w.slice(0, width));
      w = w.slice(width);
    }
    if (!cur) cur = w;
    else if (cur.length + 1 + w.length <= width) cur += ` ${w}`;
    else {
      lines.push(cur);
      cur = w;
    }
  }
  if (cur) lines.push(cur);
  return lines.length ? lines : [""];
}

/** Columns whose filled cells are all numbers (right-aligned). */
export function numericColumns(rows: string[][], width: number): boolean[] {
  return Array.from({ length: width }, (_, i) => {
    const vals = rows.map((r) => (r[i] ?? "").trim()).filter((v) => !blank(v));
    return vals.length > 0 && vals.every((v) => NUMERIC.test(v));
  });
}

export function tableToGrid(grid: Grid, maxCol = GRID_MAX_COL): string {
  const [headers, ...rows] = grid;
  if (!headers || !rows.length) return "";
  const width = Math.max(headers.length, ...rows.map((r) => r.length));
  const cell = (r: string[], i: number) => (r[i] ?? "").trim();
  const numeric = numericColumns(rows, width);
  // column width: the longest cell, capped (long cells wrap) — except the first column, which never
  // wraps: a filled first column is what marks a new row when the grid is read back
  const widths = Array.from({ length: width }, (_, i) => {
    const cells = [...(i ? [cell(headers, i)] : []), ...rows.map((r) => cell(r, i))];
    const longest = Math.max(1, ...cells.map((c) => c.length));
    return i === 0 ? Math.max(longest, ...wrap(cell(headers, 0), maxCol).map((l) => l.length)) : Math.min(maxCol, longest);
  });
  const render = (cells: string[], align: boolean): string[] => {
    const wrapped = cells.map((c, i) => wrap(c, widths[i]));
    const h = Math.max(...wrapped.map((w) => w.length));
    const out: string[] = [];
    for (let k = 0; k < h; k++) {
      const parts = wrapped.map((w, i) => {
        const t = w[k] ?? "";
        return align && numeric[i] ? t.padStart(widths[i]) : t.padEnd(widths[i]);
      });
      out.push(parts.join(" | ").trimEnd());
    }
    return out;
  };
  const lines = [
    ...render(
      Array.from({ length: width }, (_, i) => cell(headers, i)),
      false,
    ),
    widths.map((w) => "-".repeat(w)).join("-+-"),
  ];
  for (const r of rows) {
    if (r.every((c) => blank(c))) continue;
    // "—" cells are left empty (the list leaves them out, so both routes give the same grid)
    const cells = Array.from({ length: width }, (_, i) => (blank(cell(r, i)) ? "" : cell(r, i)));
    if (!cells[0]) cells[0] = "—";
    lines.push(...render(cells, true));
  }
  return lines.join("\n");
}

const RULE = /^-+(-\+-+)+$/;

/** Column spans [start, end) from a grid rule line ("--+-----+---"). */
function spans(rule: string): [number, number][] {
  const out: [number, number][] = [];
  let start = 0;
  for (let i = 0; i <= rule.length; i++) {
    if (i === rule.length || rule[i] === "+") {
      // "-+-": a column ends one before the "+", the next starts one after it
      out.push([start, i === rule.length ? Number.MAX_SAFE_INTEGER : i - 1]);
      start = i + 2;
    }
  }
  return out;
}

/** Every column boundary has its "|" (the last column's padding may be trimmed away). */
function fitsGrid(line: string, sp: [number, number][]): boolean {
  return sp.slice(0, -1).every(([, end]) => line.length > end + 1 && line[end + 1] === "|");
}

function cellsAt(line: string, sp: [number, number][]): string[] {
  return sp.map(([a, b]) => line.slice(a, b).trim());
}

export interface FoundGrid {
  from: number;
  to: number;
  grid: Grid;
  numeric: boolean[];
}

/** Fixed-width grids made by `tableToGrid` in a text, by line, read back into cells. */
export function findGrids(lines: string[]): FoundGrid[] {
  const out: FoundGrid[] = [];
  for (let s = 0; s < lines.length; s++) {
    const rule = lines[s].trimEnd();
    if (!RULE.test(rule)) continue;
    const sp = spans(rule);
    let from = s;
    while (from > 0 && fitsGrid(lines[from - 1], sp)) from--;
    let to = s + 1;
    while (to < lines.length && fitsGrid(lines[to], sp)) to++;
    if (from === s || to === s + 1) continue;
    const join = (a: string, b: string) => (a && b ? `${a} ${b}` : a || b);
    // header lines merge column by column; in the body a non-empty first column starts a new row
    const header = lines
      .slice(from, s)
      .map((l) => cellsAt(l, sp))
      .reduce((acc, c) => acc.map((v, i) => join(v, c[i])));
    const rows: string[][] = [];
    for (const l of lines.slice(s + 1, to)) {
      const c = cellsAt(l, sp);
      if (c[0] || !rows.length) rows.push(c);
      else rows[rows.length - 1] = rows[rows.length - 1].map((v, i) => join(v, c[i]));
    }
    const body = rows.map((r) => (r[0] === "—" ? ["", ...r.slice(1)] : r));
    out.push({ from, to, grid: [header, ...body], numeric: numericColumns(body, sp.length) });
    s = to - 1;
  }
  return out;
}

export type NotesPart = { kind: "text"; text: string } | { kind: "table"; grid: Grid; numeric: boolean[] };

/** Notes split into text and grids (ClassicMail shows the grids as real tables). */
export function notesParts(text: string): NotesPart[] {
  const lines = text.split(/\r?\n/);
  const grids = findGrids(lines);
  const parts: NotesPart[] = [];
  let at = 0;
  for (const g of grids) {
    if (g.from > at) parts.push({ kind: "text", text: lines.slice(at, g.from).join("\n") });
    parts.push({ kind: "table", grid: g.grid, numeric: g.numeric });
    at = g.to;
  }
  if (at < lines.length) parts.push({ kind: "text", text: lines.slice(at).join("\n") });
  return parts;
}
