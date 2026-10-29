import { describe, expect, it } from "vitest";
import { paintedHtml, paintRange, type Paint } from "./painter";

const P: Paint = { fontFamily: "Arial", fontSize: "14pt", color: null, highlight: null, bold: true, italic: false, underline: false, strike: false };

const page = (html: string) => {
  const d = document.createElement("div");
  d.innerHTML = html;
  document.body.replaceChildren(d);
  return d;
};

describe("Format Painter keeps what it paints", () => {
  it("paints part of a line with a link in it, the link stays", () => {
    const d = page('<p>one <a href="https://x.example">two</a> three</p>');
    const p = d.querySelector("p")!;
    const r = document.createRange();
    r.setStart(p.firstChild!, 2); // "e two th"
    r.setEnd(p.lastChild!, 3);
    paintRange(r, P);
    expect(d.querySelector("a")?.getAttribute("href")).toBe("https://x.example");
    expect(d.textContent).toBe("one two three");
    expect(Array.from(d.querySelectorAll("span")).map((s) => s.textContent)).toEqual(["e ", "two", " th"]);
    expect(d.querySelector("span")?.getAttribute("style")).toContain("font-family:Arial");
  });

  it("paints within one text node", () => {
    const d = page("<p>hello world</p>");
    const t = d.querySelector("p")!.firstChild!;
    const r = document.createRange();
    r.setStart(t, 6);
    r.setEnd(t, 11);
    paintRange(r, P);
    expect(d.innerHTML).toMatch(/^<p>hello <span style="[^"]+">world<\/span><\/p>$/);
  });

  it("over several paragraphs and a list, each keeps its block", () => {
    const d = page("<p>first</p><ul><li>item</li></ul><p>last</p>");
    const r = document.createRange();
    r.setStart(d.firstChild!.firstChild!, 2);
    r.setEnd(d.lastChild!.firstChild!, 2);
    const html = paintedHtml(r, P);
    expect(html).toMatch(/^<p><span[^>]*>rst<\/span><\/p><ul><li><span[^>]*>item<\/span><\/li><\/ul><p><span[^>]*>la<\/span><\/p>$/);
  });
});
