// What Translate sends to the AI: the text with just its structure (paragraphs, headings, lists,
// bold / italic / underline, links, pictures as their short references), never a web page's
// styles, classes, scripts, layout tables or widgets. A pasted page is mostly markup: the model
// would be paid to read and copy it back, and could garble it. "Back to <language>" puts the
// original back exactly, so nothing of it is lost.

/** Kept as they are (renamed where two tags mean the same). */
const KEEP: Record<string, string> = {
  P: "p",
  H1: "h1",
  H2: "h2",
  H3: "h3",
  H4: "h4",
  H5: "h4",
  H6: "h4",
  UL: "ul",
  OL: "ol",
  LI: "li",
  BLOCKQUOTE: "blockquote",
  B: "b",
  STRONG: "b",
  I: "i",
  EM: "i",
  U: "u",
  A: "a",
  BR: "br",
  IMG: "img",
};
/** Left out with everything in them. */
const DROP = new Set([
  "SCRIPT",
  "STYLE",
  "NOSCRIPT",
  "TEMPLATE",
  "HEAD",
  "TITLE",
  "META",
  "LINK",
  "SVG",
  "BUTTON",
  "INPUT",
  "SELECT",
  "TEXTAREA",
  "IFRAME",
  "OBJECT",
  "EMBED",
  "CANVAS",
  "VIDEO",
  "AUDIO",
]);
/** Containers whose content becomes paragraphs of its own (layout tables, divs, sections...). */
const CONTAINER = new Set([
  "DIV",
  "TABLE",
  "TBODY",
  "THEAD",
  "TFOOT",
  "TR",
  "TD",
  "TH",
  "SECTION",
  "ARTICLE",
  "HEADER",
  "FOOTER",
  "NAV",
  "MAIN",
  "ASIDE",
  "CENTER",
  "FIGURE",
  "FIGCAPTION",
  "DL",
  "DT",
  "DD",
  "FORM",
  "PRE",
  "ADDRESS",
]);
const BLOCKS = new Set(["p", "h1", "h2", "h3", "h4", "ul", "ol", "li", "blockquote"]);

const hiddenStyle = (el: Element) => /(^|;)\s*(display\s*:\s*none|visibility\s*:\s*hidden)/i.test(el.getAttribute("style") ?? "");

/** The AI's translation ready for the page: each paragraph, heading and list takes the direction
 *  of its own text (Hebrew right-to-left, English left-to-right). */
export function directed(html: string): string {
  const d = document.createElement("div");
  d.innerHTML = html;
  d.querySelectorAll("p, h1, h2, h3, h4, ul, ol, blockquote").forEach((b) => b.setAttribute("dir", "auto"));
  return d.innerHTML;
}

/** `html` as text with its structure only (see the module comment). */
export function textForAi(html: string): string {
  const src = new DOMParser().parseFromString(`<body>${html}</body>`, "text/html").body;
  const doc = document.implementation.createHTMLDocument("");
  const out = doc.createElement("div");

  /** The children of `from`, simplified, appended to `to`. */
  const walk = (from: Node, to: Element) => {
    for (const n of Array.from(from.childNodes)) {
      if (n.nodeType === Node.TEXT_NODE) {
        to.append(doc.createTextNode(n.nodeValue ?? ""));
        continue;
      }
      if (n.nodeType !== Node.ELEMENT_NODE) continue;
      const el = n as Element;
      const tag = el.tagName.toUpperCase();
      if (DROP.has(tag) || el.hasAttribute("hidden") || hiddenStyle(el)) continue;
      const keep = KEEP[tag];
      if (keep) {
        const copy = doc.createElement(keep);
        if (keep === "a" && el.getAttribute("href")) copy.setAttribute("href", el.getAttribute("href")!);
        if (keep === "img") copy.setAttribute("src", el.getAttribute("src") ?? "");
        to.append(copy);
        walk(el, copy);
      } else if (CONTAINER.has(tag)) {
        // its content as blocks: a paragraph for its loose text, its own blocks as they are
        const box = doc.createElement("div");
        walk(el, box);
        let para: Element | null = null;
        for (const c of Array.from(box.childNodes)) {
          if (c.nodeType === Node.ELEMENT_NODE && BLOCKS.has((c as Element).tagName.toLowerCase())) {
            para = null;
            to.append(c);
          } else {
            if (!para) {
              para = doc.createElement("p");
              to.append(para);
            }
            para.append(c);
          }
        }
      } else walk(el, to); // span, font, small...: their text
    }
  };
  walk(src, out);

  // whitespace tidied, empty paragraphs and list items gone
  const tw = doc.createTreeWalker(out, NodeFilter.SHOW_TEXT);
  for (let t = tw.nextNode(); t; t = tw.nextNode()) t.nodeValue = (t.nodeValue ?? "").replace(/[ \t\r\n\u00a0]+/g, " ");
  out.querySelectorAll("p, li, h1, h2, h3, h4, blockquote").forEach((b) => {
    if (!(b.textContent ?? "").trim() && !b.querySelector("img")) b.remove();
  });
  // the spaces between blocks (indentation of the page's source) aren't text
  for (const box of [out, ...Array.from(out.querySelectorAll("ul, ol, blockquote"))]) {
    for (const c of Array.from(box.childNodes)) if (c.nodeType === Node.TEXT_NODE && !(c.nodeValue ?? "").trim()) c.remove();
  }
  return out.innerHTML;
}
