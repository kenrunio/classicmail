// Pure layout: overlapping appointments side by side (day/week grid) and all-day lanes.

/** Items drawn in the all-day band (and as bars in month view): all-day or 24 hours and longer. */
export function isBandItem(e: { allDay: boolean; start: number; end: number }) {
  return e.allDay || e.end - e.start >= 86_400_000;
}

export interface Timed {
  id: number;
  /** minutes from the column's midnight (may be clipped) */
  startMin: number;
  endMin: number;
}

export interface Placed<T> {
  item: T;
  startMin: number;
  endMin: number;
  /** column index inside its overlap cluster */
  col: number;
  /** number of columns in the cluster */
  cols: number;
  /** how many columns it may stretch over (free columns to its right) */
  span: number;
}

/**
 * Outlook-style layout: items that overlap (transitively) form a cluster; each item takes the
 * left-most free column; every item in a cluster gets width 1/cols, stretched right over free
 * columns. `minLen` is the minimum visual length (one slot) used for overlap tests.
 */
export function layoutDay<T extends Timed>(items: T[], minLen = 30): Placed<T>[] {
  const sorted = [...items].sort((a, b) => a.startMin - b.startMin || b.endMin - a.endMin || a.id - b.id);
  const out: Placed<T>[] = [];
  let cluster: Placed<T>[] = [];
  let colEnds: number[] = [];
  let clusterEnd = -Infinity;
  const vEnd = (p: { startMin: number; endMin: number }) => Math.max(p.endMin, p.startMin + minLen);

  const flush = () => {
    const cols = colEnds.length;
    for (const p of cluster) {
      p.cols = cols;
      // stretch over following columns that are free during this item
      let span = 1;
      for (let c = p.col + 1; c < cols; c++) {
        const blocked = cluster.some((o) => o.col === c && o.startMin < vEnd(p) && vEnd(o) > p.startMin);
        if (blocked) break;
        span++;
      }
      p.span = span;
    }
    out.push(...cluster);
    cluster = [];
    colEnds = [];
  };

  for (const it of sorted) {
    const s = it.startMin;
    const e = vEnd(it);
    if (cluster.length && s >= clusterEnd) {
      flush();
      clusterEnd = -Infinity;
    }
    let col = colEnds.findIndex((end) => end <= s);
    if (col < 0) {
      col = colEnds.length;
      colEnds.push(e);
    } else colEnds[col] = e;
    cluster.push({ item: it, startMin: it.startMin, endMin: it.endMin, col, cols: 1, span: 1 });
    clusterEnd = Math.max(clusterEnd, e);
  }
  if (cluster.length) flush();
  return out;
}

export interface Spanning {
  id: number;
  /** first and last day index covered (inclusive), already clipped to the visible days */
  first: number;
  last: number;
}

/** Assign multi-day bars to lanes (rows) so they don't overlap. Returns lane per item + lane count. */
export function layoutLanes<T extends Spanning>(items: T[]): { item: T; lane: number }[] {
  const sorted = [...items].sort((a, b) => a.first - b.first || b.last - b.first - (a.last - a.first) || a.id - b.id);
  const laneEnds: number[] = [];
  return sorted.map((it) => {
    let lane = laneEnds.findIndex((end) => end < it.first);
    if (lane < 0) {
      lane = laneEnds.length;
      laneEnds.push(it.last);
    } else laneEnds[lane] = it.last;
    return { item: it, lane };
  });
}

// ---- side-by-side calendars ------------------------------------------------------------------

export type Arrangement = "sideBySide" | "overlay";

/**
 * Column groups for the Day / Week views: one group per checked calendar side by side, except
 * calendars "viewed in overlay mode", which join the first group (Outlook 2007). `overlay` (or a
 * single calendar) gives one group with everything.
 */
export function buildGroups<C extends { id: number }>(selected: C[], arrangement: Arrangement, overlaid: number[]): C[][] {
  if (selected.length === 0) return [[]];
  if (arrangement === "overlay" || selected.length === 1) return [selected];
  const [first, ...rest] = selected;
  const merged = [first, ...rest.filter((c) => overlaid.includes(c.id))];
  return [merged, ...rest.filter((c) => !overlaid.includes(c.id)).map((c) => [c])];
}

export interface GridColumn {
  /** index into the visible days */
  day: number;
  /** index of the calendar group */
  group: number;
}

/** Group-major columns: every group shows all the days. */
export function gridColumns(days: number, groups: number): GridColumn[] {
  const out: GridColumn[] = [];
  for (let g = 0; g < Math.max(1, groups); g++) for (let d = 0; d < days; d++) out.push({ day: d, group: g });
  return out;
}

/** The calendar a new item in a group is saved to: its first writable calendar. */
export function groupTarget<C extends { id: number; canWrite: boolean }>(group: C[]): C | undefined {
  return group.find((c) => c.canWrite);
}

/** First / last column index an item spanning days [first, last] covers inside group `g`. */
export function spanInGroup(first: number, last: number, g: number, days: number): { first: number; last: number } {
  return { first: g * days + first, last: g * days + last };
}
