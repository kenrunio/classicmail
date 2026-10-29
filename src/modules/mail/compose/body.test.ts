import { describe, expect, it, vi } from "vitest";
import { cellCss, changeUndoably, cleanPastedHtml, finalizeBody, keepSelection, listBlockHtml, nextCell, normalizeLoaded, originalOf, replyBlocks, replyRange } from "./body";

const FONT = { family: "Calibri", size: 11 };

// an Outlook-style original: mso classes, <o:p>, a conditional comment, a nested table, inline fonts
const ORIGINAL =
  `<div id="cm-original"><hr style="display:inline-block;width:98%"><p class="MsoNormal" style="margin:0in"><b><span style="font-family:'Calibri',sans-serif">From:</span></b> Eli &lt;eli@x.com&gt;<o:p></o:p></p>` +
  `<!--[if gte mso 9]><xml><o:OfficeDocumentSettings><o:AllowPNG/></o:OfficeDocumentSettings></xml><![endif]-->` +
  `<p></p><div class="WordSection1"><p class="MsoNormal">Please book the call for  November 19.<o:p></o:p></p>` +
  `<table class="MsoNormalTable" border="0" cellspacing="0" cellpadding="0"><tbody><tr><td style="padding:0in 5.4pt"><p class="MsoNormal">A<o:p></o:p></p></td></tr></tbody></table></div>` +
  `<style>p{margin:0}</style></div>`;

const root = (html: string) => {
  const d = document.createElement("div");
  d.innerHTML = html;
  return d;
};

const parsedOriginal = (html: string) => originalOf(new DOMParser().parseFromString(html, "text/html").body)!.outerHTML;

describe("replyBlocks", () => {
  it("stops at #cm-original", () => {
    const r = root(`<p>One</p><p>Two</p>${ORIGINAL}`);
    expect(replyBlocks(r).map((b) => b.textContent)).toEqual(["One", "Two"]);
    expect(originalOf(r)?.id).toBe("cm-original");
  });
  it("is everything without it", () => {
    const r = root("<p>One</p><div>Two</div>");
    expect(replyBlocks(r).map((b) => b.textContent)).toEqual(["One", "Two"]);
    expect(originalOf(r)).toBeNull();
  });
});

describe("replyRange", () => {
  const page = root(`<p>Hi Eli</p><p><br></p><p>Kenny Green</p><p>Duvz Software</p>${ORIGINAL}`);
  const sig = "Kenny Green\nDuvz Software";

  it("ends before the signature and its blank lines", () => {
    expect(replyRange(page, sig).toString()).toBe("Hi Eli");
  });
  it("with withSignature covers everything above the original", () => {
    expect(replyRange(page, sig, true).toString()).toBe("Hi EliKenny GreenDuvz Software");
  });
  it("is empty when only the signature is there", () => {
    const r = replyRange(root("<p>Kenny Green</p><p>Duvz Software</p>"), sig);
    expect(r.collapsed).toBe(true);
    expect(r.startOffset).toBe(0);
  });
  it("treats the whole reply as text when it does not end with the signature", () => {
    const r = root("<p>Hi</p><p>Kenny Green</p><p>Duvz Software</p><p>PS</p>");
    expect(replyRange(r, sig).toString()).toBe("HiKenny GreenDuvz SoftwarePS");
  });
});

describe("finalizeBody", () => {
  it("keeps the original byte-identical", () => {
    const html = `<p>Hi Eli</p><p>Thanks</p>${ORIGINAL}`;
    const out = finalizeBody(html, FONT);
    expect(out.endsWith(parsedOriginal(html))).toBe(true);
    // not rewritten: the mso markup is all still there
    expect(out).toContain('<p class="MsoNormal" style="margin:0in">');
    expect(out).toContain("<o:p></o:p>");
    expect(out).toContain("<p></p>");
  });
  it("styles the reply", () => {
    const out = finalizeBody(`<p><br></p><p>Hi</p><table><tbody><tr><td>x</td></tr></tbody></table>${ORIGINAL}`, FONT);
    expect(out.startsWith(`<div class="cm-reply" style="font-family:'Calibri',sans-serif;font-size:11pt">`)).toBe(true);
    expect(out).toContain('<p style="margin:0;">&nbsp;</p>');
    expect(out).toContain('<p style="margin:0;">Hi</p>');
    expect(out).toMatch(/<table style="border-collapse:collapse;">/);
    // a table with no borders of its own goes without (as the page shows it)
    expect(out).toMatch(/<td style="border-top:none;/);
  });
  it("leaves the original's paragraphs, tables and cells unstyled", () => {
    const out = finalizeBody(`<p>Hi</p>${ORIGINAL}`, FONT);
    expect(out).toContain('<table class="MsoNormalTable" border="0" cellspacing="0" cellpadding="0"><tbody><tr><td style="padding:0in 5.4pt">');
  });
  it("works with no original", () => {
    expect(finalizeBody("<p>Hi</p>", FONT)).toBe(`<div class="cm-reply" style="font-family:'Calibri',sans-serif;font-size:11pt"><p style="margin:0;">Hi</p></div>`);
  });
});

describe("normalizeLoaded", () => {
  const cycle = (h: string) => normalizeLoaded(finalizeBody(h, FONT));

  it("unwraps our wrapper, and stays stable over repeated save and reopen", () => {
    const x = `<p>Hi Eli</p><p></p><p dir="rtl">שלום</p><table><tbody><tr><td>a</td></tr></tbody></table>${ORIGINAL}`;
    const first = cycle(normalizeLoaded(x));
    expect(first).not.toContain("cm-reply");
    let h = first;
    for (let i = 0; i < 5; i++) h = cycle(h);
    expect(h).toBe(first);
    const text = (t: string) => (root(t).textContent ?? "").split(String.fromCharCode(160)).join("");
    expect(text(h)).toBe(text(normalizeLoaded(x)));
    expect(h.endsWith(parsedOriginal(x))).toBe(true);
  });
  it("unwraps the older style-only wrapper too", () => {
    const old = `<div style="font-family:'Calibri',sans-serif;font-size:11pt"><p style="margin:0;">Hi</p></div>${ORIGINAL}`;
    const out = normalizeLoaded(old);
    expect(out.startsWith('<p style="margin:0;" dir="auto">Hi</p>')).toBe(true);
    expect(out.endsWith(parsedOriginal(old))).toBe(true);
  });
  it("makes the reply's blocks dir=auto, keeping an existing dir, and gives an empty p a caret spot", () => {
    const out = normalizeLoaded(`<p>Hi</p><p dir="rtl">x</p><p></p><div>d</div>`);
    expect(out).toBe(`<p dir="auto">Hi</p><p dir="rtl">x</p><p dir="auto"><br></p><div dir="auto">d</div>`);
  });
  it("turns the reply's nbsp-only line (a reopened draft's empty line) into a caret spot", () => {
    const out = normalizeLoaded(`<p style="margin:0;">&nbsp;</p><p>&nbsp;<img src="x.png"></p><p>Hi&nbsp;</p><div id="cm-original"><p>&nbsp;</p></div>`);
    expect(out).toBe(`<p style="margin:0;" dir="auto"><br></p><p dir="auto">&nbsp;<img src="x.png"></p><p dir="auto">Hi&nbsp;</p><div id="cm-original"><p>&nbsp;</p></div>`);
  });
  it("leaves the original alone", () => {
    const html = `<p></p>${ORIGINAL}`;
    const out = normalizeLoaded(html);
    expect(out.endsWith(parsedOriginal(html))).toBe(true);
    const o = originalOf(root(out))!;
    expect(o.querySelector("p")!.hasAttribute("dir")).toBe(false);
    expect(o.innerHTML).toContain("<p></p>");
    expect(o.innerHTML).not.toContain("dir=");
  });
});

describe("cleanPastedHtml", () => {
  it("drops a web page's floating widgets (fixed or sticky), keeps the rest", () => {
    const out = cleanPastedHtml(
      '<div style="position: fixed; bottom: 10px"><button>ד</button></div><header style="position:sticky;top:0">Menu</header><p style="position:relative">Text</p>',
    );
    expect(out).toBe('<p style="position:relative">Text</p>');
  });

  it("drops Word's extras but keeps classes and styles", () => {
    const out = cleanPastedHtml(
      `<html><head><meta charset="utf-8"><link rel="x" href="y"><style>p.MsoNormal{margin:0}</style></head><body><!--[if gte mso 9]><xml><w:WordDocument/></xml><![endif]-->` +
        `<p class="MsoNormal" style="color:red">Hi<o:p></o:p></p><xml><x/></xml></body></html>`,
    );
    expect(out).toBe('<p class="MsoNormal" style="color:red">Hi</p>');
  });
  it("drops the clipboard's fragment markers", () => {
    expect(cleanPastedHtml("<html><body><!--StartFragment--><p>Hi</p><!--EndFragment--></body></html>")).toBe("<p>Hi</p>");
  });
  it("drops a copied original's id, so the page keeps one #cm-original", () => {
    expect(cleanPastedHtml('<p>Hi</p><div id="cm-original" class="x"><p>From: Eli</p></div>')).toBe('<p>Hi</p><div class="x"><p>From: Eli</p></div>');
  });
  it("keeps other elements and attributes", () => {
    const html = '<table border="1"><tbody><tr><td style="width:10pt">a <b>b</b></td></tr></tbody></table><a href="https://x">y</a>';
    expect(cleanPastedHtml(html)).toBe(html);
  });
});

describe("nextCell", () => {
  const t = root("<table><tbody><tr><td>a</td><td>b<table><tbody><tr><td>in</td></tr></tbody></table></td></tr><tr><td>c</td><td>d</td></tr></tbody></table>");
  const cell = (text: string) => Array.from(t.querySelectorAll("td")).find((c) => c.firstChild?.textContent === text)!;

  it("goes row by row, skipping a nested table's cells", () => {
    expect(nextCell(cell("a"), 1)?.firstChild?.textContent).toBe("b");
    expect(nextCell(cell("b"), 1)?.textContent).toBe("c");
    expect(nextCell(cell("c"), -1)?.firstChild?.textContent).toBe("b");
  });
  it("stops at the table's ends", () => {
    expect(nextCell(cell("d"), 1)).toBeNull();
    expect(nextCell(cell("a"), -1)).toBeNull();
    expect(nextCell(cell("in"), 1)).toBeNull();
  });
});

describe("lists made on a line (<p><ul>…</ul></p>)", () => {
  const listed = `<p>Before</p><p dir="auto"><ul><li>a</li></ul></p><p>After</p>`;

  it("finalizeBody adds no blank line above or below the list", () => {
    const out = finalizeBody(listed + ORIGINAL, FONT);
    expect(out).toContain('<p style="margin:0;">Before</p><ul><li>a</li></ul><p style="margin:0;">After</p>');
    expect(out).not.toContain("&nbsp;</p><ul>");
  });
  it("normalizeLoaded likewise, and a saved draft stays the same", () => {
    expect(normalizeLoaded(listed)).toBe(`<p dir="auto">Before</p><ul dir="auto"><li>a</li></ul><p dir="auto">After</p>`);
    const reopened = normalizeLoaded(finalizeBody(listed, FONT));
    expect(reopened).toContain('Before</p><ul dir="auto"><li>a</li></ul><p style="margin:0;" dir="auto">After</p>');
  });
  it("finalizeBody drops the bare <p> left when the line after the list went inside the <p>", () => {
    const out = finalizeBody(`<p dir="ltr">Before</p><p dir="ltr"><ul><li>a</li></ul><p dir="ltr">After</p></p><p dir="auto"><br></p>`, FONT);
    expect(out).toContain('<ul><li>a</li></ul><p dir="ltr" style="margin:0;">After</p><p dir="auto" style="margin:0;">&nbsp;</p></div>');
  });
  it("keeps an intended empty line next to a list, and the original's empty p", () => {
    const html = `<p><br></p><ul><li>a</li></ul><p>&nbsp;</p><div id="cm-original"><p></p><ul><li>o</li></ul></div>`;
    expect(normalizeLoaded(html)).toBe(`<p dir="auto"><br></p><ul dir="auto"><li>a</li></ul><p dir="auto"><br></p><div id="cm-original"><p></p><ul><li>o</li></ul></div>`);
    const out = finalizeBody(html, FONT);
    expect(out).toContain('<p style="margin:0;">&nbsp;</p><ul><li>a</li></ul><p style="margin:0;">&nbsp;</p>');
    expect(out.endsWith('<div id="cm-original"><p></p><ul><li>o</li></ul></div>')).toBe(true);
  });
});

describe("finalizeBody and zero-width spaces", () => {
  it("strips them from the reply only", () => {
    const z = String.fromCharCode(0x200b);
    const out = finalizeBody(`<p><font style="font-size: 14pt;">${z}x</font></p><p>${z}</p><div id="cm-original"><p>o${z}</p></div>`, FONT);
    expect(out).toContain('<p style="margin:0;"><font style="font-size: 14pt;">x</font></p><p style="margin:0;">&nbsp;</p>');
    expect(out.endsWith(`<div id="cm-original"><p>o${z}</p></div>`)).toBe(true);
  });
});

describe("changeUndoably (execCommand refusing: the plain DOM fallback)", () => {
  const run = (page: HTMLElement, el: Element, mutate: () => void) => {
    document.body.append(page);
    (document as unknown as { execCommand: unknown }).execCommand = vi.fn(() => false);
    try {
      changeUndoably(el, mutate);
    } finally {
      delete (document as unknown as { execCommand?: unknown }).execCommand;
      page.remove();
    }
  };
  it("puts the changed table in", () => {
    const page = root("<p>a</p><table><tbody><tr><td>x</td></tr></tbody></table><p>b</p>");
    const t = page.querySelector("table")!;
    run(page, t, () => (t.rows[0].cells[0].style.backgroundColor = "red"));
    expect(page.innerHTML).toBe('<p>a</p><table><tbody><tr><td style="background-color: red;">x</td></tr></tbody></table><p>b</p>');
  });
  it("deletes a table the change removed (Delete Table)", () => {
    const page = root("<p>a</p><table><tbody><tr><td>x</td></tr></tbody></table><p>b</p>");
    const t = page.querySelector("table")!;
    run(page, t, () => t.remove());
    expect(page.innerHTML).toBe("<p>a</p><p>b</p>");
  });
});

describe("cellCss (what is sent)", () => {
  const cell = (style: string, tableBorder?: string) => {
    const t = document.createElement("table");
    if (tableBorder) t.setAttribute("border", tableBorder);
    const td = t.insertRow().insertCell();
    td.setAttribute("style", style);
    return td;
  };
  it("writes every border out, keeps the cell's own formatting", () => {
    const css = cellCss(cell("background-color: #ffc000; border-top: 2px solid #c00000; border-left: none", "1"));
    expect(css).toContain("border-top:2px solid");
    expect(css).toContain("border-left:none");
    expect(css).toMatch(/border-right:1px solid (#808080|rgb\(128, 128, 128\))/);
    expect(css).toMatch(/background-color: ?(#ffc000|rgb\(255, 192, 0\))/);
    expect(css).toMatch(/padding: ?2pt 4pt/);
  });
  it("a pasted borderless table stays borderless, with its alignment and font", () => {
    const css = cellCss(cell("text-align: right; font-family: Arial; border-bottom: 1px solid red"));
    expect(css).toContain("border-top:none");
    expect(css).toContain("border-bottom:1px solid red");
    expect(css).toMatch(/text-align: ?right/);
    expect(css).toMatch(/font-family: ?Arial/);
  });
});

describe("keepSelection (Bullets / Numbering)", () => {
  /** Chromium's insertUnorderedList: the line's text in new nodes inside <ul><li>, the caret at the line start. */
  const bullets = (p: Element) => () => {
    const ul = document.createElement("ul");
    const li = document.createElement("li");
    li.append(...Array.from(p.childNodes).map((n) => (n.nodeType === Node.TEXT_NODE ? document.createTextNode(n.nodeValue ?? "") : n)));
    ul.append(li);
    p.replaceChildren(ul);
    const r = document.createRange();
    r.setStart(li.firstChild!, 0);
    getSelection()!.removeAllRanges();
    getSelection()!.addRange(r);
  };
  const page = (html: string) => {
    const root = document.createElement("div");
    root.innerHTML = html;
    document.body.append(root);
    return root;
  };
  const caretAt = (node: Node, start: number, endNode: Node = node, end = start) => {
    const r = document.createRange();
    r.setStart(node, start);
    r.setEnd(endNode, end);
    getSelection()!.removeAllRanges();
    getSelection()!.addRange(r);
  };
  it("the caret goes back to its place in the text after the command rebuilds the line", () => {
    const root = page("<p>zero</p><p>one</p><p>two</p>");
    const p = root.children[1];
    caretAt(p.firstChild!, 3);
    keepSelection(root, bullets(p));
    const r = getSelection()!.getRangeAt(0);
    expect(root.querySelector("li")!.textContent).toBe("one");
    expect(r.startContainer).toBe(root.querySelector("li")!.firstChild);
    expect(r.startOffset).toBe(3);
    expect(r.collapsed).toBe(true);
  });
  it("a caret at the start of a line stays on that line, a selection keeps both ends", () => {
    const root = page("<p>zero</p><p>one</p><p>two</p>");
    const p = root.children[1];
    caretAt(p.firstChild!, 0);
    keepSelection(root, bullets(p));
    expect(getSelection()!.getRangeAt(0).startContainer.parentElement!.tagName).toBe("LI");
    caretAt(root.querySelector("li")!.firstChild!, 1, root.children[2].firstChild!, 2);
    keepSelection(root, () => {});
    expect(getSelection()!.toString()).toBe("netw");
  });
  it("leaves the selection alone when the command changed the text or the caret is not in text", () => {
    const root = page("<p>one</p><p><br></p>");
    caretAt(root.firstChild!.firstChild!, 2);
    keepSelection(root, () => {
      root.firstChild!.firstChild!.nodeValue = "x";
      caretAt(root.children[1], 0);
    });
    expect(getSelection()!.getRangeAt(0).startContainer).toBe(root.children[1]);
    caretAt(root.children[1], 0);
    keepSelection(root, () => caretAt(root.firstChild!.firstChild!, 0));
    expect(getSelection()!.getRangeAt(0).startContainer).toBe(root.firstChild!.firstChild);
  });
});

describe("listBlockHtml (a reply ending in a list, put back by insertHTML)", () => {
  const block = (html: string) => {
    const d = document.createElement("div");
    d.innerHTML = html;
    return d.firstElementChild!;
  };
  it("a <p> around a list (Bullets on a line): the <p>'s attributes go onto the list", () => {
    // (built as the page has it: parsing "<p><ul>" would close the <p> before the list)
    const b = document.createElement("p");
    b.setAttribute("dir", "rtl");
    b.setAttribute("style", "margin-left:36pt");
    b.innerHTML = "<ul><li>שלום</li><li>עולם</li></ul>";
    expect(listBlockHtml(b)).toBe('<ul dir="rtl" style="margin-left:36pt"><li>שלום</li><li>עולם</li></ul>');
  });
  it("the list's own attributes win; text after the list is kept", () => {
    const b = document.createElement("p");
    b.setAttribute("dir", "auto");
    b.innerHTML = '<ol dir="ltr"><li>a</li></ol>After';
    expect(listBlockHtml(b)).toBe('<ol dir="ltr"><li>a</li></ol>After');
  });
  it("a list block, or a <div> around one, is used as it is", () => {
    expect(listBlockHtml(block('<ul dir="auto"><li>a</li></ul>'))).toBe('<ul dir="auto"><li>a</li></ul>');
    expect(listBlockHtml(block('<div dir="rtl"><ul><li>a</li></ul></div>'))).toBe('<div dir="rtl"><ul><li>a</li></ul></div>');
  });
});
