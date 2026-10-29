// Show as Conversations — pure list logic: the visible lines (group headers, rows, and the child
// rows of expanded conversations) and how selected row keys resolve to message ids.
import type { ListGroup, MessageRow } from "../../api/mail";

/** Rows per page of the message list (a page is one `mail.listMessages` call). */
export const PAGE = 200;

/** A line of the virtual list. `index` is the row's position in the server's (paged) list. */
export type Line = { kind: "group"; g: ListGroup; collapsed: boolean } | { kind: "row"; index: number } | { kind: "child"; index: number; ci: number };

/**
 * Lines for `total` rows, optionally in `groups` (some collapsed), with `children.get(i)` child
 * lines after row `i` (expanded conversations).
 */
export function buildLines(total: number, groups: ListGroup[] | null, collapsedGroups: readonly string[], children: ReadonlyMap<number, number>): Line[] {
  const out: Line[] = [];
  const row = (i: number) => {
    out.push({ kind: "row", index: i });
    const n = children.get(i) ?? 0;
    for (let ci = 0; ci < n; ci++) out.push({ kind: "child", index: i, ci });
  };
  if (!groups) {
    for (let i = 0; i < total; i++) row(i);
    return out;
  }
  let start = 0;
  for (const g of groups) {
    const c = collapsedGroups.includes(g.key);
    out.push({ kind: "group", g, collapsed: c });
    if (!c) for (let i = start; i < start + g.count; i++) row(i);
    start += g.count;
  }
  return out;
}

/** The groups once the rows at these positions (in the whole list) are gone: each one's count
 *  drops by its removed rows, and an emptied group goes. */
export function groupsWithout(groups: ListGroup[], removed: readonly number[]): ListGroup[] {
  let start = 0;
  return groups
    .map((g) => {
      const n = removed.filter((i) => i >= start && i < start + g.count).length;
      start += g.count;
      return n ? { ...g, count: g.count - n } : g;
    })
    .filter((g) => g.count > 0);
}

/** Selection key of a list row: a conversation header or a plain message row. */
export function rowKey(r: MessageRow): string {
  return r.conv ? `c:${r.conv.key}` : `m:${r.id}`;
}

/** Selection key of a message inside an expanded conversation. */
export function childKey(r: MessageRow): string {
  return `t:${r.id}`;
}

/** Does the row have messages to expand (a conversation of more than one message)? */
export function expandable(r: MessageRow | undefined): boolean {
  return !!r?.conv && r.conv.total > 1;
}

/** Message ids a key stands for, and the message to show for it. */
export function resolveKey(key: string, byKey: ReadonlyMap<string, MessageRow>): { ids: number[]; focus: number | null } {
  const r = byKey.get(key);
  if (r?.conv) return { ids: r.conv.ids, focus: r.id };
  if (r) return { ids: [r.id], focus: r.id };
  // not loaded (e.g. scrolled away): plain keys still name their message
  const m = /^[mt]:(-?\d+)$/.exec(key);
  if (m) return { ids: [Number(m[1])], focus: Number(m[1]) };
  return { ids: [], focus: null };
}

/** Selected keys → the message ids commands act on (each once, in key order) + the focused message. */
export function resolveSelection(keys: readonly string[], focusKey: string | null, byKey: ReadonlyMap<string, MessageRow>): { selected: number[]; focused: number | null } {
  const seen = new Set<number>();
  const selected: number[] = [];
  for (const k of keys)
    for (const id of resolveKey(k, byKey).ids)
      if (!seen.has(id)) {
        seen.add(id);
        selected.push(id);
      }
  return { selected, focused: focusKey ? resolveKey(focusKey, byKey).focus : null };
}

/** Flag state of rows (conversation headers count as flagged when any of their messages is). */
export function rowFlagged(r: Pick<MessageRow, "isFlagged" | "flagStatus" | "conv">): boolean {
  return r.conv ? r.conv.flag === 2 : r.isFlagged || r.flagStatus === "flagged";
}

/** Ids rows stand for (conversation headers → their messages in the view). */
export function idsOfRows(rows: readonly Pick<MessageRow, "id" | "conv">[]): number[] {
  return [...new Set(rows.flatMap((r) => (r.conv ? r.conv.ids : [r.id])))];
}
