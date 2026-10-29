// The compose body is one plain contentEditable page. Edits that code makes (replacing a block,
// shading a table) have to be undoable with Ctrl+Z like typing, so they go through the browser's
// own `insertHTML` command instead of poking the DOM.
//
// Spike result (real WebView2, one contentEditable holding a reply and an Outlook-style original):
//  - Replacing a <table> with its mutated outerHTML through insertHTML keeps the markup exactly
//    (no added style or wrapper), in any position, and Ctrl+Z / execCommand("undo") restores the
//    old table exactly in one step; redo works too.
//  - Replacing several blocks works as one undo step, but only if the range ends INSIDE the last
//    block: a range ending at an element boundary after a block (setEndAfter, selectNodeContents,
//    (container, childCount)) makes Chromium merge the following block into the inserted one and
//    drop its wrapper (the original's <div id="cm-original"> vanished). replaceUndoably therefore
//    moves such boundaries inside the nearest block (see `inward`), so callers pass natural ranges.
//    Tables are left alone: Chromium replaces a table selected as a node cleanly.
//  - Known limit: Chromium drops a style attribute that holds only `mso-*` properties (and unwraps
//    a span left with nothing) from the markup it re-inserts; everything else in an Outlook table
//    (`o:p`, colgroup, mso-* next to real properties, classes) came back identical.

import { signatureStart } from "./aiHelpers";

const TABLES = new Set(["TABLE", "THEAD", "TBODY", "TFOOT", "TR", "TD", "TH"]);
const OPAQUE = new Set(["BR", "IMG", "HR", "INPUT", "CANVAS", "VIDEO", "IFRAME", "SVG"]);
const HIDDEN = new Set(["STYLE", "SCRIPT", "TEMPLATE"]);

/** Nodes that show nothing, which a boundary can look past: comments (Outlook's `<!--[if gte mso 9]>`),
 *  whitespace between blocks, and elements that are not drawn. */
function invisible(n: Node): boolean {
  if (n.nodeType === Node.TEXT_NODE) return !/\S/.test(n.nodeValue ?? "");
  if (n.nodeType === Node.ELEMENT_NODE) return HIDDEN.has((n as Element).tagName.toUpperCase());
  return true; // comments, processing instructions
}

/** Whether a boundary must not move into this node: a table (Chromium replaces those cleanly as a
 *  node) or a node without content. */
const solid = (el: Element): boolean => TABLES.has(el.tagName.toUpperCase()) || OPAQUE.has(el.tagName.toUpperCase());

/** A copy of `range` whose boundaries sit at the edge of the text next to them instead of at an
 *  element boundary, so Chromium does not merge a neighbouring block into the inserted HTML.
 *  The end moves back into the last visible thing before it, the start forward into the first
 *  visible thing after it, looking past comments, whitespace-only text and <style>/<script>; a
 *  table (or anything inside one) is never entered, and the boundary stays put there. */
function inward(range: Range): Range {
  const r = range.cloneRange();
  if (r.collapsed) return r;
  // end: (container, offset) -> end of the last visible descendant before it
  let ec = r.endContainer;
  let eo = r.endOffset;
  while (ec.nodeType === Node.ELEMENT_NODE) {
    let i = eo - 1;
    while (i >= 0 && invisible(ec.childNodes[i])) i--;
    if (i < 0) break;
    const prev = ec.childNodes[i];
    if (prev.nodeType === Node.TEXT_NODE) {
      ec = prev;
      eo = (prev.nodeValue ?? "").length;
      break;
    }
    if (prev.nodeType !== Node.ELEMENT_NODE || solid(prev as Element)) break;
    ec = prev;
    eo = prev.childNodes.length;
  }
  // start: (container, offset) -> start of the first visible descendant after it
  let sc = r.startContainer;
  let so = r.startOffset;
  while (sc.nodeType === Node.ELEMENT_NODE) {
    let i = so;
    while (i < sc.childNodes.length && invisible(sc.childNodes[i])) i++;
    if (i >= sc.childNodes.length) break;
    const next = sc.childNodes[i];
    if (next.nodeType === Node.TEXT_NODE) {
      sc = next;
      so = 0;
      break;
    }
    if (next.nodeType !== Node.ELEMENT_NODE || solid(next as Element)) break;
    sc = next;
    so = 0;
  }
  const out = document.createRange();
  out.setStart(sc, so);
  out.setEnd(ec, eo);
  // never let the nudging turn a real range into nothing
  return out.collapsed ? r : out;
}

/** Replace what `range` covers with `html`, as one undo step. Selects the range (see `inward`)
 *  and runs `insertHTML`. If Chromium refuses, falls back to editing the DOM directly, which is
 *  NOT undoable (Ctrl+Z skips it) but still gets the content in. */
export function replaceUndoably(range: Range, html: string): void {
  const sel = window.getSelection();
  if (!sel) return;
  const r = inward(range);
  sel.removeAllRanges();
  sel.addRange(r);
  if (document.execCommand("insertHTML", false, html)) return;
  // not undoable: the browser declined the command, so this is a plain DOM edit
  r.deleteContents();
  const t = document.createElement("template");
  t.innerHTML = html;
  r.insertNode(t.content);
}

/** Child-index path from `root` down to `node` (the last entry is the offset when `node` is text). */
function pathOf(root: Node, node: Node): number[] {
  const path: number[] = [];
  for (let n: Node | null = node; n && n !== root; n = n.parentNode) {
    path.unshift(Array.prototype.indexOf.call(n.parentNode!.childNodes, n));
  }
  return path;
}

/** The node at `path` below `root`, or the deepest one that still exists. */
function nodeAt(root: Node, path: number[]): Node {
  let n = root;
  for (const i of path) {
    const next = n.childNodes[i];
    if (!next) break;
    n = next;
  }
  return n;
}

/** Make `live` (a node of the page) look exactly like `snap` again, keeping its own nodes wherever
 *  the two have the same shape, so the browser's undo history (typing in a cell, say) still points
 *  at nodes that are on the page. Only a node whose kind or tag differs is swapped for a copy. */
function restoreInPlace(live: Element, snap: Element): void {
  for (const a of Array.from(live.attributes)) if (!snap.hasAttribute(a.name)) live.removeAttribute(a.name);
  for (const a of Array.from(snap.attributes)) if (live.getAttribute(a.name) !== a.value) live.setAttribute(a.name, a.value);
  const want = Array.from(snap.childNodes);
  const have = Array.from(live.childNodes);
  want.forEach((w, i) => {
    const h = have[i];
    if (!h) live.appendChild(w.cloneNode(true));
    else if (w.nodeType === Node.ELEMENT_NODE && h.nodeType === Node.ELEMENT_NODE && (w as Element).tagName === (h as Element).tagName) restoreInPlace(h as Element, w as Element);
    else if (w.nodeType === h.nodeType && w.nodeType !== Node.ELEMENT_NODE) {
      if (h.nodeValue !== w.nodeValue) h.nodeValue = w.nodeValue;
    } else live.replaceChild(w.cloneNode(true), h);
  });
  for (const h of have.slice(want.length)) live.removeChild(h);
}

/** Run `mutate` on the live element `el` (editing its cells, styles...), then make that whole
 *  change one undo step: put the old markup back (in place, see `restoreInPlace`), and replace it
 *  with the new markup through `insertHTML`. The caret goes back to the same place inside the
 *  element. A change that removes `el` itself (Delete Table) deletes it undoably. Limits: a change
 *  that adds or removes rows or cells shifts which nodes are kept, so an earlier typing step inside
 *  the moved cells may not undo; and Chromium drops style attributes that hold only `mso-*`
 *  properties (and spans left with nothing) from the re-inserted element. */
export function changeUndoably(el: Element, mutate: () => void): void {
  const parent = el.parentNode;
  if (!parent) return;
  const index = Array.prototype.indexOf.call(parent.childNodes, el);

  // remember the caret, as a path inside el
  const sel = window.getSelection();
  const r = sel && sel.rangeCount ? sel.getRangeAt(0) : null;
  const caret = r && el.contains(r.startContainer) ? { path: pathOf(el, r.startContainer), offset: r.startOffset } : null;

  const snap = el.cloneNode(true) as Element;
  mutate();
  // a change that removed el (Delete Table) puts it back, to be deleted undoably
  const removed = el.parentNode !== parent;
  const after = removed ? "" : el.outerHTML;
  if (removed) parent.insertBefore(el, parent.childNodes[index] ?? null);
  restoreInPlace(el, snap);

  const range = document.createRange();
  range.selectNode(el);
  replaceUndoably(range, after);

  const now = parent.childNodes[index];
  if (caret && now && !removed) {
    const node = nodeAt(now, caret.path);
    const max = node.nodeType === Node.TEXT_NODE ? (node.nodeValue ?? "").length : node.childNodes.length;
    const c = document.createRange();
    c.setStart(node, Math.min(caret.offset, max));
    c.collapse(true);
    const s = window.getSelection();
    s?.removeAllRanges();
    s?.addRange(c);
  }
}

/** A selection boundary in a text node as a place in `root`'s text: the characters before it, and
 *  whether it is at the start of its node (it then stays at the start of the next text, on its own
 *  line, rather than at the end of the line before). Null when it isn't in text. */
function textPoint(root: Node, node: Node, offset: number): { at: number; forward: boolean } | null {
  if (node.nodeType !== Node.TEXT_NODE || !root.contains(node)) return null;
  let at = offset;
  const tw = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let t = tw.nextNode(); t && t !== node; t = tw.nextNode()) at += t.nodeValue?.length ?? 0;
  return { at, forward: offset === 0 };
}

/** The text node and offset at a `textPoint`. */
function fromTextPoint(root: Node, p: { at: number; forward: boolean }): { node: Node; offset: number } | null {
  let at = 0;
  const tw = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let t = tw.nextNode(); t; t = tw.nextNode()) {
    const len = t.nodeValue?.length ?? 0;
    if (p.forward ? p.at < at + len : p.at <= at + len) return { node: t, offset: p.at - at };
    at += len;
  }
  return null;
}

/** Run an editing command that rebuilds the text it works on, then put the selection back at the
 *  same place in the text. Chromium's Bullets / Numbering on a line move its text into new nodes
 *  inside the list and leave the caret at the start of the line. Nothing is done when the command
 *  changed the text, or when the selection wasn't in text (an empty line). */
export function keepSelection(root: Element, run: () => void): void {
  const sel = window.getSelection();
  const r = sel?.rangeCount ? sel.getRangeAt(0) : null;
  const text = root.textContent;
  const a = r && textPoint(root, r.startContainer, r.startOffset);
  const b = r && textPoint(root, r.endContainer, r.endOffset);
  run();
  if (!sel || !a || !b || root.textContent !== text) return;
  const start = fromTextPoint(root, a);
  const end = fromTextPoint(root, b);
  if (!start || !end) return;
  const now = document.createRange();
  now.setStart(start.node, start.offset);
  now.setEnd(end.node, end.offset);
  sel.removeAllRanges();
  sel.addRange(now);
}

// ---- The page's content: your reply, then the original -------------------------------------------
//
// The body is the reply's top-level blocks followed by `<div id="cm-original">` (from the Rust side,
// prepare.rs ORIGINAL_MARKER). The original is never rewritten; every helper below that produces
// HTML leaves its markup exactly as the browser serialises it.

/** The `#cm-original` element among `root`'s top-level children, if any. */
export function originalOf(root: Element): Element | null {
  for (const c of Array.from(root.children)) if (c.id === "cm-original") return c;
  return null;
}

/** Your reply: the top-level children before the original (all of them when there is none). */
export function replyBlocks(root: Element): Element[] {
  const out: Element[] = [];
  for (const c of Array.from(root.children)) {
    if (c.id === "cm-original") break;
    out.push(c);
  }
  return out;
}

/** A block with no text and nothing else to see (an empty line). */
const blank = (b: Element): boolean => !(b.textContent ?? "").trim() && !b.querySelector("img, table, hr");

/** Your own text as a range from the page's start: ending before the signature (and the blank lines
 *  above it), or with `withSignature` after everything above the original. Natural range, for
 *  `replaceUndoably`. Collapsed at the start when there is no text. */
export function replyRange(root: Element, signatureText: string, withSignature = false): Range {
  const blocks = replyBlocks(root);
  let last =
    (withSignature
      ? null
      : signatureStart(
          blocks.map((b) => b.textContent ?? ""),
          signatureText,
        )) ?? blocks.length;
  while (last > 0 && blank(blocks[last - 1])) last--;
  const r = document.createRange();
  r.setStart(root, 0);
  if (last > 0) r.setEndAfter(blocks[last - 1]);
  else r.collapse(true);
  return r;
}

const FONT_WRAPPER = /^\s*font-family:[^;]+;\s*font-size:[\d.]+pt;?\s*$/i;

/** Replace the reply wrapper `div.cm-reply` (and the older class-less one whose style is exactly the
 *  font style) among `root`'s top-level children by its content. */
function unwrapReply(root: Element): void {
  for (const c of Array.from(root.children)) {
    if (c.tagName !== "DIV" || c.id) continue;
    if (c.classList.contains("cm-reply") || (!c.hasAttribute("class") && FONT_WRAPPER.test(c.getAttribute("style") ?? ""))) {
      unwrapReply(c);
      c.replaceWith(...Array.from(c.childNodes));
    }
  }
}

const DIR_BLOCKS = new Set(["P", "DIV", "H1", "H2", "H3", "H4", "H5", "H6", "UL", "OL", "BLOCKQUOTE", "PRE"]);

const isList = (e: Element | null) => e?.tagName === "UL" || e?.tagName === "OL";
/** Chromium's Bullets / Numbering on a line make `<p><ul>…</ul></p>` (and the line after the list
 *  goes inside that `<p>` too), which parses back as an empty `<p>` on each side of the list, or a
 *  bare `<p></p>` from the stray `</p>`. A real empty line holds a `<br>` or `&nbsp;`; on the page,
 *  one with nothing at all still has attributes (`dir`, a pasted Word line's class), so `bare` (no
 *  attributes either) is a leftover there. Not for loading: prepare's empty lines are bare `<p></p>`. */
const listSplit = (p: Element, bare = false) => !p.firstChild && ((bare && !p.attributes.length) || isList(p.previousElementSibling) || isList(p.nextElementSibling));

/** The markup that puts back a reply block ending in a list through `insertHTML` (the From
 *  account's signature going in after it). Bullets on a line make `<p><ul>…</ul></p>`, which HTML
 *  parsing can't rebuild (a list closes an open <p>), so such a <p> gives its content, with the
 *  <p>'s attributes (a Hebrew list's dir="rtl") moved onto its lists where they have none of their
 *  own. Any other block is used as it is. */
export function listBlockHtml(block: Element): string {
  if (block.tagName !== "P") return block.outerHTML;
  const copy = block.cloneNode(true) as Element;
  for (const list of Array.from(copy.children).filter(isList)) for (const a of Array.from(block.attributes)) if (!list.hasAttribute(a.name)) list.setAttribute(a.name, a.value);
  return copy.innerHTML;
}

/** Page content from saved HTML: unwraps `div.cm-reply`, gives the reply's blocks `dir="auto"` (the
 *  original's never get one), drops the empty `<p>`s a list leaves around it (`listSplit`), and makes
 *  an empty `<p></p>` or a nbsp-only one (a saved empty line) `<p><br></p>` so the caret can sit in
 *  it and typing there gets no leading space. */
export function normalizeLoaded(html: string): string {
  const doc = new DOMParser().parseFromString(`<div id="root">${html}</div>`, "text/html");
  const root = doc.getElementById("root")!;
  unwrapReply(root);
  for (const b of replyBlocks(root)) {
    const ps = b.tagName === "P" ? [b, ...Array.from(b.querySelectorAll("p"))] : Array.from(b.querySelectorAll("p"));
    for (const p of ps) {
      if (listSplit(p)) p.remove();
      else if (/^\u00a0*$/.test(p.textContent ?? "") && !p.querySelector("img")) p.innerHTML = "<br>";
    }
    if (b.isConnected && DIR_BLOCKS.has(b.tagName) && !b.hasAttribute("dir")) b.setAttribute("dir", "auto");
  }
  return root.innerHTML;
}

function serialize(n: Node): string {
  if (n.nodeType === Node.ELEMENT_NODE) return (n as Element).outerHTML;
  const t = document.createElement("template");
  t.content.append(n.cloneNode(true));
  return t.innerHTML;
}

/** A table cell's outgoing style: its own formatting as it is, with every border written out side
 *  by side (Outlook's Word engine reads those, not the shorthands). A side without a border of its
 *  own gets what the page shows there: the grid line of a table with a `border` attribute (an
 *  older draft's table), else none. */
export function cellCss(c: HTMLElement): string {
  const line = Number(c.closest("table")?.getAttribute("border") ?? 0) > 0 ? "1px solid #808080" : "none";
  const st = document.createElement("td").style;
  st.cssText = `border:${line};padding:2pt 4pt;${c.getAttribute("style") ?? ""}`;
  const sides = ["top", "right", "bottom", "left"].map((s) => {
    const style = st.getPropertyValue(`border-${s}-style`);
    return `border-${s}:${!style || style === "none" ? "none" : `${st.getPropertyValue(`border-${s}-width`) || "1px"} ${style} ${st.getPropertyValue(`border-${s}-color`) || "#808080"}`}`;
  });
  // a column width an older draft kept as TipTap's colwidth is written on the cell for Outlook
  const cw = (c.getAttribute("colwidth") ?? c.getAttribute("data-colwidth") ?? "")
    .split(",")
    .map(Number)
    .filter((n) => n > 0);
  if (cw.length) st.setProperty("width", `${cw.reduce((a, b) => a + b, 0)}px`);
  c.removeAttribute("colwidth");
  // everything else the cell has (alignment, font, shading...) goes as it is
  st.removeProperty("border");
  return [...sides, st.cssText.replace(/;\s*$/, "")].filter(Boolean).join(";");
}

/** Outgoing HTML: your reply in a `div.cm-reply` carrying the font (Word-mail defaults made explicit
 *  so every client renders the same, without zero-width spaces or the empty `<p>`s a list leaves),
 *  then the original exactly as it is on the page. */
export function finalizeBody(html: string, font: { family: string; size: number }): string {
  const doc = new DOMParser().parseFromString(`<div id="root">${html}</div>`, "text/html");
  const root = doc.getElementById("root")!;
  unwrapReply(root);
  const nodes = Array.from(root.childNodes);
  const at = nodes.findIndex((n) => n.nodeType === Node.ELEMENT_NODE && (n as Element).id === "cm-original");
  const reply = at < 0 ? nodes : nodes.slice(0, at);
  const rest = at < 0 ? [] : nodes.slice(at);

  const box = doc.createElement("div");
  box.append(...reply);
  const tw = doc.createTreeWalker(box, NodeFilter.SHOW_TEXT);
  for (let t = tw.nextNode(); t; t = tw.nextNode()) if (t.nodeValue?.includes("\u200b")) t.nodeValue = t.nodeValue.replace(/\u200b/g, "");
  box.querySelectorAll("p").forEach((p) => {
    if (listSplit(p, true)) return p.remove();
    const st = p.getAttribute("style") ?? "";
    if (!/^\s*margin:\s*0\s*;/.test(st)) p.setAttribute("style", `margin:0;${st}`);
    if (!p.textContent && !p.querySelector("img")) p.innerHTML = "&nbsp;";
  });
  box.querySelectorAll("table").forEach((t) => {
    const st = t.getAttribute("style") ?? "";
    if (!/^\s*border-collapse:\s*collapse\s*;/.test(st)) t.setAttribute("style", `border-collapse:collapse;${st}`);
  });
  box.querySelectorAll<HTMLElement>("td, th").forEach((c) => c.setAttribute("style", cellCss(c)));
  const style = `font-family:'${font.family}',sans-serif;font-size:${font.size}pt`;
  return `<div class="cm-reply" style="${style}">${box.innerHTML}</div>${rest.map(serialize).join("")}`;
}

/** Pasted Word / Outlook HTML without its scaffolding: `<o:p>`, `<meta>`, `<style>`, `<link>`,
 *  `<xml>` and comments (Word's `<!--[if …]>`, the clipboard's `<!--StartFragment-->`) go, and so
 *  does the id of a copied `#cm-original` (only the page's own original may have it). A web page's
 *  floating parts (positioned fixed or sticky: an accessibility button, a cookie bar, a sticky
 *  header) go too. Every other element, attribute and style stays, so the paste looks like what
 *  was copied. */
export function cleanPastedHtml(html: string): string {
  const doc = new DOMParser().parseFromString(html, "text/html");
  Array.from(doc.querySelectorAll("*"))
    .filter((e) => /^(o:p|meta|style|link|xml)$/i.test(e.tagName))
    .forEach((e) => e.remove());
  const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_COMMENT);
  const comments: Node[] = [];
  for (let n = walker.nextNode(); n; n = walker.nextNode()) comments.push(n);
  comments.forEach((c) => c.parentNode?.removeChild(c));
  doc.querySelectorAll("#cm-original").forEach((e) => e.removeAttribute("id"));
  doc.querySelectorAll<HTMLElement>("[style]").forEach((e) => {
    if (/position\s*:\s*(fixed|sticky)/i.test(e.getAttribute("style") ?? "")) e.remove();
  });
  return doc.body.innerHTML;
}

/** The cell after (`dir` 1) or before (-1) `cell` in its table, row by row (Tab / Shift+Tab). */
export function nextCell(cell: HTMLTableCellElement, dir: 1 | -1): HTMLTableCellElement | null {
  const table = cell.closest("table");
  const cells = table ? Array.from(table.rows).flatMap((r) => Array.from(r.cells)) : [];
  return cells[cells.indexOf(cell) + dir] ?? null;
}
