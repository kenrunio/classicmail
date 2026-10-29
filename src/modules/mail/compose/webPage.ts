// Include › Web Page: what turns a web page, as it shows on screen, into HTML an e-mail keeps.
// Mail programs drop a page's <style> sheets, scripts, web layout (grid, flex, columns) and
// floating parts, so the result carries each element's look on the element itself, grids become
// tables, and links and pictures get full addresses. It is what copying a page in a browser does,
// minus what can't work in mail.
//
// `inlinePage` runs inside the page's own hidden window (src-tauri/src/mail/compose/webpage.rs),
// sent there as text: it must stay self-contained (nothing from outside the function).

/** The page shown in `doc`, as e-mail HTML (or `{ error }`). */
export function inlinePage(doc: Document): string | { error: string } {
  try {
    const win = doc.defaultView;
    if (!win || !doc.body) return { error: "no page" };
    const out = doc.implementation.createHTMLDocument("");
    // never in a message: code, forms, players, embedded pages
    const DROP = new Set([
      "SCRIPT", "STYLE", "LINK", "META", "NOSCRIPT", "TEMPLATE", "HEAD", "TITLE", "BASE",
      "INPUT", "SELECT", "TEXTAREA", "BUTTON", "FORM", "LABEL", "DATALIST", "OPTION",
      "IFRAME", "FRAME", "OBJECT", "EMBED", "VIDEO", "AUDIO", "CANVAS", "SVG", "DIALOG",
    ]); // prettier-ignore
    // passed down from the parent: written only where an element differs from it
    const INHERITED = [
      "color",
      "font-family",
      "font-size",
      "font-weight",
      "font-style",
      "line-height",
      "text-align",
      "letter-spacing",
      "text-transform",
      "white-space",
      "direction",
    ];
    // the element's own box: written when it isn't the default
    const OWN: [string, (v: string) => boolean][] = [
      ["background-color", (v) => v !== "rgba(0, 0, 0, 0)" && v !== "transparent"],
      ...["top", "right", "bottom", "left"].flatMap((s): [string, (v: string) => boolean][] => [
        [`padding-${s}`, (v) => v !== "0px"],
        [`margin-${s}`, (v) => v !== "0px" && v !== "auto"],
        [`border-${s}-width`, (v) => v !== "0px"],
        [`border-${s}-style`, (v) => v !== "none"],
        [`border-${s}-color`, () => true],
      ]),
      ["border-top-left-radius", (v) => v !== "0px"],
      ["border-top-right-radius", (v) => v !== "0px"],
      ["border-bottom-left-radius", (v) => v !== "0px"],
      ["border-bottom-right-radius", (v) => v !== "0px"],
      ["text-decoration-line", (v) => v !== "none"],
      ["vertical-align", (v) => v !== "baseline"],
      ["list-style-type", (v) => v !== "disc" && v !== "decimal"],
    ];
    // tags mail keeps as they are; any other becomes a div (block) or span (inline)
    const KEEP = new Set([
      "P", "DIV", "SPAN", "A", "B", "STRONG", "I", "EM", "U", "S", "SMALL", "SUB", "SUP", "MARK", "CODE", "PRE", "BLOCKQUOTE",
      "H1", "H2", "H3", "H4", "H5", "H6", "UL", "OL", "LI", "BR", "HR", "IMG", "TABLE", "THEAD", "TBODY", "TFOOT", "TR", "TD", "TH", "CAPTION",
    ]); // prettier-ignore
    const visible = (cs: CSSStyleDeclaration) => cs.display !== "none" && cs.visibility !== "hidden" && cs.position !== "fixed" && cs.position !== "sticky";

    // spacing a page uses to place things on a screen (a tall top padding that centres the
    // content, say) is out of place in a message: at most this much
    const MAX_SPACE = 48;

    /** Style text for `cs` against its parent's `pcs` (border colour only with a border; nothing
     *  of the names in `skip`). */
    const styleOf = (cs: CSSStyleDeclaration, pcs: CSSStyleDeclaration | null, extra: string[] = [], skip: string[] = []): string => {
      const parts: string[] = [];
      for (const p of INHERITED) {
        const v = cs.getPropertyValue(p);
        if (v && (!pcs || pcs.getPropertyValue(p) !== v)) parts.push(`${p}:${v}`);
      }
      for (const [p, keep] of OWN) {
        let v = cs.getPropertyValue(p);
        if (!v || !keep(v) || skip.includes(p)) continue;
        if (/^(margin|padding)-/.test(p) && parseFloat(v) > MAX_SPACE) v = `${MAX_SPACE}px`;
        if (p.startsWith("border-") && p.endsWith("-color")) {
          const side = p.slice(7, -6);
          if (cs.getPropertyValue(`border-${side}-style`) === "none" || cs.getPropertyValue(`border-${side}-width`) === "0px") continue;
        }
        parts.push(`${p}:${v}`);
      }
      return [...parts, ...extra].join(";");
    };

    /** A copy of `el` for the message, or null when it doesn't belong there. */
    const copy = (el: Element, pcs: CSSStyleDeclaration | null): Node | null => {
      const tag = el.tagName.toUpperCase();
      if (DROP.has(tag)) return null;
      const cs = win.getComputedStyle(el);
      if (!visible(cs)) return null;
      const display = cs.display;
      const inline = display.startsWith("inline") && display !== "inline-block";
      const name = KEEP.has(tag) ? tag.toLowerCase() : inline ? "span" : "div";
      const node = out.createElement(name);
      const extra: string[] = [];
      if (tag === "IMG") {
        const img = el as HTMLImageElement;
        node.setAttribute("src", img.currentSrc || img.src);
        if (img.alt) node.setAttribute("alt", img.alt);
        const r = img.getBoundingClientRect();
        if (r.width) node.setAttribute("width", String(Math.round(r.width)));
        extra.push("max-width:100%", "height:auto");
      }
      if (tag === "A" && (el as HTMLAnchorElement).href) node.setAttribute("href", (el as HTMLAnchorElement).href);
      for (const a of ["colspan", "rowspan", "dir"]) if (el.hasAttribute(a)) node.setAttribute(a, el.getAttribute(a)!);
      // a tag mail shows inline that the page made a block (a bold title on its own line), or the
      // other way round, keeps how it showed
      const INLINE_TAGS = /^(A|B|STRONG|I|EM|U|S|SMALL|SUB|SUP|MARK|CODE|SPAN|IMG|BR)$/;
      if (display === "inline-block") extra.push("display:inline-block");
      else if (!inline && INLINE_TAGS.test(tag) && !/^(flex|grid)/.test(display)) extra.push("display:block");
      else if (inline && !INLINE_TAGS.test(tag) && KEEP.has(tag) && !/^(TD|TH|TR|TABLE|LI)$/.test(tag)) extra.push("display:inline");
      const rect = el.getBoundingClientRect();
      // a block the page centres with equal side margins stays centred, at most as wide as it was
      const skip: string[] = [];
      const ml = parseFloat(cs.marginLeft) || 0;
      const mr = parseFloat(cs.marginRight) || 0;
      const centred = !inline && ml > 0 && Math.abs(ml - mr) < 1;
      const centre = centred ? ["margin-left:auto", "margin-right:auto", `max-width:${Math.round(rect.width)}px`] : [];
      if (centred) skip.push("margin-left", "margin-right");
      node.setAttribute("style", styleOf(cs, pcs, [...extra, ...centre], skip));

      const kids = Array.from(el.childNodes);
      const copies = (): Node[] =>
        kids.flatMap((k): Node[] => {
          if (k.nodeType === 3) return (k.nodeValue ?? "").length ? [out.createTextNode(k.nodeValue ?? "")] : [];
          if (k.nodeType !== 1) return [];
          const c = copy(k as Element, cs);
          return c ? [c] : [];
        });
      /** A layout table for a grid or a flex row: never wider than the page showed it (a full
       *  width table with side margins would run past the window), centred if the page centred it. */
      const layoutTable = (): HTMLTableElement => {
        const table = out.createElement("table");
        table.setAttribute("role", "presentation");
        table.setAttribute("cellpadding", "0");
        table.setAttribute("cellspacing", "0");
        const box = [`max-width:${Math.round(rect.width)}px`, ...(centred ? ["margin-left:auto", "margin-right:auto"] : [])];
        // separate (not collapsed) borders: a collapsed table drops its padding and rounded corners
        table.setAttribute("style", `border-collapse:separate;border-spacing:0;width:100%;${styleOf(cs, pcs, box, ["margin-left", "margin-right"])}`);
        return table;
      };
      /** A cell holding `c`: dt / dd / li outside their list don't hold up in mail, so their content
       *  goes in as a div. */
      const cellOf = (c: Element, style: string): HTMLTableCellElement => {
        const td = out.createElement("td");
        td.setAttribute("style", style);
        if (/^(dt|dd|li)$/i.test(c.tagName)) {
          const d = out.createElement("div");
          d.setAttribute("style", c.getAttribute("style") ?? "");
          d.append(...Array.from(c.childNodes));
          td.append(d);
        } else td.append(c);
        return td;
      };
      const px = (v: string) => Math.round(parseFloat(v) || 0);
      // a grid: its cells as table rows (mail keeps tables, not grids), the gaps as cell padding
      if (display === "grid" || display === "inline-grid") {
        const n = Math.max(
          1,
          cs
            .getPropertyValue("grid-template-columns")
            .trim()
            .split(/\s+(?![^(]*\))/)
            .filter(Boolean).length,
        );
        const cells = copies().filter((c) => c.nodeType === 1) as Element[];
        const colGap = px(cs.columnGap);
        const rowGap = px(cs.rowGap);
        const table = layoutTable();
        const rows = Math.ceil(cells.length / n);
        for (let r = 0; r < rows; r++) {
          const tr = out.createElement("tr");
          const row = cells.slice(r * n, r * n + n);
          row.forEach((c, i) => {
            const pad = [i < row.length - 1 && colGap ? `padding-right:${colGap}px` : "", r < rows - 1 && rowGap ? `padding-bottom:${rowGap}px` : ""];
            tr.append(cellOf(c, ["vertical-align:top", ...pad].filter(Boolean).join(";")));
          });
          table.append(tr);
        }
        return table;
      }
      const children = copies();
      if ((display === "flex" || display === "inline-flex") && !cs.flexDirection.startsWith("column")) {
        // the row's items: its elements, and any loose text (a title written straight in the row)
        const items = children.flatMap((c): Element[] => {
          if (c.nodeType === 1) return [c as Element];
          if (!(c.nodeValue ?? "").trim()) return [];
          const span = out.createElement("span");
          span.append(c);
          return [span];
        });
        const gap = px(cs.columnGap);
        if (cs.flexWrap === "nowrap" && items.length > 1) {
          // a flex row: one table row, its items side by side as the page aligned them
          const valign = { "flex-end": "bottom", end: "bottom", center: "middle", baseline: "baseline" }[cs.alignItems] ?? "top";
          const spread = /space-between|flex-end|end/.test(cs.justifyContent);
          const table = layoutTable();
          const tr = out.createElement("tr");
          items.forEach((c, i) => {
            const last = i === items.length - 1;
            tr.append(
              cellOf(c, [`vertical-align:${valign}`, !last && gap ? `padding-right:${gap}px` : "", last && spread && i > 0 ? "text-align:right" : ""].filter(Boolean).join(";")),
            );
          });
          table.append(tr);
          return table;
        }
        // a flex row that wraps (chips, tags): its items side by side as long as they fit
        for (const e of items) {
          const st = e.getAttribute("style") ?? "";
          e.setAttribute("style", `${st ? `${st};` : ""}display:inline-block;vertical-align:top${gap ? `;margin-right:${gap}px` : ""}`);
        }
      }
      node.append(...children);
      return node;
    };

    const body = doc.body;
    const bcs = win.getComputedStyle(body);
    const box = out.createElement("div");
    // the page's own frame: its colours and type, not its margins and padding (screen placement)
    const frame = ["top", "right", "bottom", "left"].flatMap((s) => [`margin-${s}`, `padding-${s}`]);
    // the page's colour: the body's, or the whole page's when the body has none
    const clear = (c: string) => !c || c === "rgba(0, 0, 0, 0)" || c === "transparent";
    const bodyBg = bcs.backgroundColor;
    const pageBg = win.getComputedStyle(doc.documentElement).backgroundColor;
    const bg = clear(bodyBg) ? pageBg : bodyBg;
    const own = clear(bg) ? [] : clear(bodyBg) ? [`background-color:${bg}`, "padding:16px"] : ["padding:16px"];
    box.setAttribute("style", styleOf(bcs, null, own, frame));
    for (const k of Array.from(body.childNodes)) {
      if (k.nodeType === 3) box.append(out.createTextNode(k.nodeValue ?? ""));
      else if (k.nodeType === 1) {
        const c = copy(k as Element, bcs);
        if (c) box.append(c);
      }
    }
    return box.outerHTML;
  } catch (e) {
    return { error: String(e) };
  }
}

/** The script the page window runs: `inlinePage` on its own document. */
export const inlinePageScript = () => `(${inlinePage.toString()})(document)`;
