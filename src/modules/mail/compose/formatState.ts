// What the Font group shows, like Word: the font, size and bold / italic / underline / strike of
// the text at the cursor or across the selection (your reply, the original or both after Ctrl+A).
// When the selection mixes values, that box is blank (null) and the button isn't pressed.
import { paintFromDom } from "./painter";

export interface Fmt {
  family: string;
  size: number;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  strike: boolean;
}
export interface FmtState {
  /** null: mixed */
  family: string | null;
  size: number | null;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  strike: boolean;
}

/** "'Times New Roman', serif" → "Times New Roman" */
export const firstFamily = (f: string) =>
  f
    .split(",")[0]
    .trim()
    .replace(/^['"]|['"]$/g, "");

/** "11pt" → 11, "14.6667px" → 11 (to the half point). */
export function ptOf(s: string | null | undefined): number | null {
  const m = /([\d.]+)\s*(pt|px)?/.exec(s ?? "");
  if (!m) return null;
  const v = parseFloat(m[1]);
  return Math.round((m[2] === "px" ? v * 0.75 : v) * 2) / 2;
}

/** One value per box, or null where the items differ; a button only when all of them have it. */
export function combine(items: Fmt[]): FmtState | null {
  if (!items.length) return null;
  const same = <K extends "family" | "size">(k: K) => (items.every((i) => i[k] === items[0][k]) ? items[0][k] : null);
  return {
    family: same("family"),
    size: same("size"),
    bold: items.every((i) => i.bold),
    italic: items.every((i) => i.italic),
    underline: items.every((i) => i.underline),
    strike: items.every((i) => i.strike),
  };
}

const MAX = 400; // text runs looked at (a whole long message is plenty)

/** The page's formatting at / over a DOM range (computed styles). */
export function domFormats(range: Range, root: Element): Fmt[] {
  const fmt = (n: Node): Fmt => {
    const p = paintFromDom(n, root);
    return { family: firstFamily(p.fontFamily ?? ""), size: ptOf(p.fontSize) ?? 0, bold: p.bold, italic: p.italic, underline: p.underline, strike: p.strike };
  };
  if (range.collapsed) return root.contains(range.startContainer) ? [fmt(range.startContainer)] : [];
  const out: Fmt[] = [];
  const scope = range.commonAncestorContainer;
  const w = document.createTreeWalker(scope.nodeType === Node.TEXT_NODE ? scope.parentNode! : scope, NodeFilter.SHOW_TEXT);
  for (let n = w.nextNode(); n && out.length < MAX; n = w.nextNode()) {
    if (!range.intersectsNode(n) || !n.nodeValue?.trim() || n.parentElement?.closest("style, script")) continue;
    out.push(fmt(n));
  }
  return out;
}
