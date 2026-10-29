// Format Painter's formatting: read from the page's computed styles (your reply or the original
// alike), so it can copy from one and paint on the other.

export interface Paint {
  fontFamily: string | null;
  fontSize: string | null;
  color: string | null;
  highlight: string | null;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  strike: boolean;
}

const transparent = (c: string) => !c || c === "transparent" || /^rgba\(.*,\s*0\)$/.test(c);

/** The formatting at a point of the page (`root`): computed font, colour, highlight,
 *  and the underline / strike-through of any element around it (they aren't inherited). */
export function paintFromDom(node: Node, root: Element): Paint {
  const el = (node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement) as Element;
  const cs = getComputedStyle(el);
  let highlight: string | null = null;
  let underline = false;
  let strike = false;
  for (let e: Element | null = el; e && e !== root.parentElement; e = e.parentElement) {
    const s = getComputedStyle(e);
    if (!highlight && !transparent(s.backgroundColor) && e !== root) highlight = s.backgroundColor;
    underline ||= s.textDecorationLine.includes("underline");
    strike ||= s.textDecorationLine.includes("line-through");
    if (e === root) break;
  }
  const px = parseFloat(cs.fontSize);
  return {
    fontFamily: cs.fontFamily || null,
    fontSize: px ? `${Math.round(px * 0.75 * 2) / 2}pt` : null,
    color: cs.color || null,
    highlight,
    bold: parseInt(cs.fontWeight, 10) >= 600,
    italic: cs.fontStyle === "italic",
    underline,
    strike,
  };
}

/** Inline CSS for painting `p` on text on the page. */
export function paintCss(p: Paint): string {
  const deco = [p.underline && "underline", p.strike && "line-through"].filter(Boolean).join(" ") || "none";
  return [
    p.fontFamily && `font-family:${p.fontFamily}`,
    p.fontSize && `font-size:${p.fontSize}`,
    p.color && `color:${p.color}`,
    p.highlight && `background-color:${p.highlight}`,
    `font-weight:${p.bold ? "bold" : "normal"}`,
    `font-style:${p.italic ? "italic" : "normal"}`,
    `text-decoration:${deco}`,
  ]
    .filter(Boolean)
    .join(";");
}

// whitespace between these is layout, not text to paint
const LAYOUT = new Set(["TABLE", "TBODY", "THEAD", "TFOOT", "TR", "UL", "OL"]);

/** Put each piece of text under `root` that `within` covers (all of it when null) in a span with
 *  `css`; the first and last text are split where the range starts and ends. */
function wrapText(root: Node, within: Range | null, css: string): void {
  const texts: Text[] = [];
  const w = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let n = w.nextNode() as Text | null; n; n = w.nextNode() as Text | null) {
    if (!n.nodeValue || (within && !within.intersectsNode(n))) continue;
    if (!/\S/.test(n.nodeValue) && (n.parentNode === root || LAYOUT.has(n.parentElement?.tagName ?? ""))) continue;
    texts.push(n);
  }
  for (let t of texts) {
    if (within && t === within.endContainer && within.endOffset < t.length) t.splitText(within.endOffset);
    if (within && t === within.startContainer && within.startOffset > 0) t = t.splitText(within.startOffset);
    const span = document.createElement("span");
    span.setAttribute("style", css);
    t.replaceWith(span);
    span.append(t);
  }
}

/** Paint `p` over the text `range` covers, in place: each piece of text gets a span of its own, so
 *  the links, paragraphs, lists and table cells in the selection stay as they are. */
export function paintRange(range: Range, p: Paint): void {
  const at = range.commonAncestorContainer;
  wrapText(at.nodeType === Node.TEXT_NODE ? at.parentNode! : at, range, paintCss(p));
}

/** What `range` covers as HTML, painted with `p` (for a selection over several blocks, which
 *  replaces itself through insertHTML). */
export function paintedHtml(range: Range, p: Paint): string {
  const box = document.createElement("div");
  box.append(range.cloneContents());
  wrapText(box, null, paintCss(p));
  return box.innerHTML;
}
