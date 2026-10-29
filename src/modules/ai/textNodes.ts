// Helpers for translating a rendered message body in place: find the visible text nodes worth
// translating, split them into request batches, swap texts and text direction, and undo it all.

const SKIP_TAGS = new Set(["SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE", "HEAD", "TITLE", "CODE", "PRE", "KBD", "SAMP"]);

const URLISH = /^(?:[a-z][a-z0-9+.-]*:\/\/\S+|www\.\S+|mailto:\S+|[\w.+-]+@[\w-]+(?:\.[\w-]+)+)$/i;

/** Does this text have anything to translate (letters, not just a URL / address / number)? */
export function worthTranslating(s: string): boolean {
  const t = s.trim();
  if (!t) return false;
  if (!/\p{L}/u.test(t)) return false;
  if (URLISH.test(t)) return false;
  return true;
}

/** Visible text nodes under `root` that are worth translating, in document order. */
export function collectTextNodes(root: Node): Text[] {
  const doc = root.ownerDocument ?? (root as Document);
  const win = doc.defaultView;
  const hidden = new Map<Element, boolean>();
  const isHidden = (el: Element | null): boolean => {
    if (!el) return false;
    const known = hidden.get(el);
    if (known !== undefined) return known;
    let h = el.hasAttribute("hidden") || SKIP_TAGS.has(el.tagName);
    if (!h) {
      const st = win ? win.getComputedStyle(el) : (el as HTMLElement).style;
      h = st?.display === "none" || st?.visibility === "hidden";
    }
    if (!h) h = isHidden(el.parentElement);
    hidden.set(el, h);
    return h;
  };
  const out: Text[] = [];
  const walker = doc.createTreeWalker(root, 4 /* NodeFilter.SHOW_TEXT */);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const t = n as Text;
    if (!worthTranslating(t.nodeValue ?? "")) continue;
    if (isHidden(t.parentElement)) continue;
    out.push(t);
  }
  return out;
}

/** Consecutive batches of at most `maxChars` characters / `maxItems` items (an over-long item alone). */
export function batchTexts(texts: string[], maxChars = 3500, maxItems = 60): [number, number][] {
  const out: [number, number][] = [];
  let start = 0;
  let chars = 0;
  texts.forEach((t, i) => {
    if (i > start && (chars + t.length > maxChars || i - start >= maxItems)) {
      out.push([start, i]);
      start = i;
      chars = 0;
    }
    chars += t.length;
  });
  if (start < texts.length) out.push([start, texts.length]);
  return out;
}

/** Undo information for one in-place translation. */
export interface Applied {
  restore: () => void;
}

/**
 * Put `translations[i]` into `nodes[i]` and, when the direction changes (Hebrew → English),
 * flip right-to-left blocks to left-to-right. Returns a handle that restores everything.
 */
export function applyTranslation(doc: Document, nodes: Text[], translations: string[], toRtl: boolean): Applied {
  const originals = nodes.map((n) => n.nodeValue ?? "");
  nodes.forEach((n, i) => {
    const t = translations[i];
    if (typeof t === "string" && n.isConnected) n.nodeValue = t;
  });
  const attrs: { el: HTMLElement; dir: string | null; align: string | null; textAlign: string; direction: string }[] = [];
  const win = doc.defaultView;
  const want = toRtl ? "rtl" : "ltr";
  const body = doc.body;
  if (body) {
    const els = [body, ...Array.from(body.querySelectorAll<HTMLElement>("*"))];
    for (const el of els) {
      const explicit = el.getAttribute("dir");
      const cs = el === body && win ? win.getComputedStyle(el) : null;
      const flip = (explicit && explicit.toLowerCase() !== want) || (el.style.direction && el.style.direction !== want);
      const alignAttr = el.getAttribute("align");
      const oppositeAlign = toRtl ? "left" : "right";
      const alignFlip = alignAttr?.toLowerCase() === oppositeAlign || el.style.textAlign === oppositeAlign;
      if (!flip && !alignFlip && !(cs && cs.direction !== want)) continue;
      attrs.push({ el, dir: explicit, align: alignAttr, textAlign: el.style.textAlign, direction: el.style.direction });
      if (flip || el === body) {
        el.setAttribute("dir", want);
        if (el.style.direction) el.style.direction = want;
      }
      if (alignFlip) {
        if (alignAttr) el.setAttribute("align", toRtl ? "right" : "left");
        if (el.style.textAlign === oppositeAlign) el.style.textAlign = toRtl ? "right" : "left";
      }
    }
  }
  return {
    restore: () => {
      nodes.forEach((n, i) => {
        if (n.isConnected) n.nodeValue = originals[i];
      });
      for (const a of attrs) {
        if (a.dir === null) a.el.removeAttribute("dir");
        else a.el.setAttribute("dir", a.dir);
        if (a.align === null) a.el.removeAttribute("align");
        else a.el.setAttribute("align", a.align);
        a.el.style.textAlign = a.textAlign;
        a.el.style.direction = a.direction;
      }
    },
  };
}
