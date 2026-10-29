// AI Review in the quoted original. People also write in the original below the reply (inline
// answers, or an old e-mail reused as a template), so {instructions} there count too — but only
// ones the user added: braces that weren't in the original when the window opened, or, for a
// reopened draft (no clean original to compare with), ones that don't look machine-made. Each
// block holding one goes to the AI on its own, with the original's own braces in it masked.
import { instructions } from "./aiHelpers";

const BLOCK = "p, li, td, th, h1, h2, h3, h4, h5, h6, pre, blockquote, div, dd, dt";
const SKIP = "style, script, title, head";
const MASK: Record<string, string> = { "{": "⦃", "}": "⦄" };
const UNMASK: Record<string, string> = { "⦃": "{", "⦄": "}" };

export interface Unit {
  el: Element;
  /** the {instructions} in it, in order */
  found: string[];
}

function visibleText(el: Element): string {
  let s = "";
  const w = el.ownerDocument.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  for (let n = w.nextNode(); n; n = w.nextNode()) if (!n.parentElement?.closest(SKIP)) s += n.nodeValue ?? "";
  // line breaks in the HTML source show as spaces
  return s.replace(/\s+/g, " ");
}

/** The blocks of a quoted section that hold {…}, outermost only, in order (`fields`: mail-merge
 *  columns, not instructions). */
export function braceUnits(root: Element, fields: string[] = []): Unit[] {
  const els: Element[] = [];
  const w = root.ownerDocument.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let n = w.nextNode(); n; n = w.nextNode()) {
    const parent = n.parentElement;
    if (!parent || parent.closest(SKIP) || !(n.nodeValue ?? "").includes("{")) continue;
    const b = parent.closest(BLOCK);
    const el = b && root.contains(b) ? b : root;
    if (!els.includes(el)) els.push(el);
  }
  return els
    .filter((e) => !els.some((o) => o !== e && o.contains(e)))
    .map((el) => ({ el, found: instructions(visibleText(el), fields) }))
    .filter((u) => u.found.length > 0);
}

/** Braces a program wrote rather than a person: codes, ids, links ({0}, {ID}, {https://…}). */
export function machineMade(s: string): boolean {
  const t = s.trim();
  return /^(mailto:|https?:|cid:|www\.)/i.test(t) || /^\S+@\S+$/.test(t) || !/[a-z]/.test(t);
}

/**
 * The units with instructions the user added, each with the braces in it to leave alone.
 * `baseline` = the instructions of the original as the window opened it (null for a reopened
 * draft, where only `machineMade` ones are left alone).
 */
export function unitsToReview(units: Unit[], baseline: string[] | null): { el: Element; keep: string[] }[] {
  const left = new Map<string, number>();
  for (const b of baseline ?? []) left.set(b, (left.get(b) ?? 0) + 1);
  const out: { el: Element; keep: string[] }[] = [];
  for (const u of units) {
    const keep: string[] = [];
    let fresh = false;
    for (const f of u.found) {
      const n = left.get(f) ?? 0;
      if (baseline ? n > 0 : machineMade(f)) {
        keep.push(f);
        if (n > 0) left.set(f, n - 1);
      } else fresh = true;
    }
    if (fresh) out.push({ el: u.el, keep });
  }
  return out;
}

/** The block's HTML with the braces of `keep` swapped for look-alikes the AI won't act on. */
export function maskedHtml(el: Element, keep: string[]): string {
  const clone = el.cloneNode(true) as Element;
  const left = new Map<string, number>();
  for (const k of keep) left.set(k, (left.get(k) ?? 0) + 1);
  if (left.size) {
    const w = clone.ownerDocument.createTreeWalker(clone, NodeFilter.SHOW_TEXT);
    for (let n = w.nextNode(); n; n = w.nextNode()) {
      n.nodeValue = (n.nodeValue ?? "").replace(/\{([^{}\n]*)\}/g, (m, inner: string) => {
        const c = left.get(inner.trim()) ?? 0;
        if (!c) return m;
        left.set(inner.trim(), c - 1);
        return `${MASK["{"]}${inner}${MASK["}"]}`;
      });
    }
  }
  return clone.innerHTML;
}

export function unmask(html: string): string {
  return html.replace(/[⦃⦄]/g, (c) => UNMASK[c]);
}
