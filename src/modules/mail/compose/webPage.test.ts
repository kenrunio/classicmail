import { describe, expect, it } from "vitest";
import { inlinePage, inlinePageScript } from "./webPage";

// a real window's document (inlinePage reads computed styles through it)
const page = (body: string, css = "") => {
  const frame = document.createElement("iframe");
  document.body.append(frame);
  const doc = frame.contentDocument!;
  const base = doc.createElement("base");
  base.href = "https://site.example/docs/";
  doc.head.append(base);
  const st = doc.createElement("style");
  st.textContent = css;
  doc.head.append(st);
  doc.body.innerHTML = body;
  return doc;
};

describe("inlinePage (Include › Web Page)", () => {
  it("keeps the content with its look on each element, drops what mail can't use", () => {
    const doc = page(
      `<h1>Sheet</h1><div class="bar"><input placeholder="Search"><button>Go</button></div>
       <p class="note">Read <a href="more.html">more</a> <img src="pic.png" alt="pic"></p>
       <div style="position:fixed">floating</div><div style="display:none">hidden</div>
       <script>x()</script><custom-tag>own tag</custom-tag>`,
      ".note { color: rgb(91, 103, 100); background-color: rgb(246, 230, 214); padding-top: 8px; }",
    );
    const html = inlinePage(doc) as string;
    expect(typeof html).toBe("string");
    expect(html).not.toMatch(/<input|<button|<script|floating|hidden/);
    expect(html).toContain("<h1");
    expect(html).toMatch(/<p style="[^"]*color:rgb\(91, 103, 100\)[^"]*background-color:rgb\(246, 230, 214\)[^"]*padding-top:8px/);
    expect(html).toContain('href="https://site.example/docs/more.html"');
    expect(html).toContain('src="https://site.example/docs/pic.png"');
    expect(html).toMatch(/<(span|div) style="[^"]*">own tag</);
  });

  it("a grid becomes a table, its cells side by side", () => {
    const doc = page(`<dl class="g"><dt>确定</dt><dd>OK</dd><dt>取消</dt><dd>Cancel</dd></dl>`, ".g { display: grid; grid-template-columns: 120px 300px; }");
    const html = inlinePage(doc) as string;
    const d = document.createElement("div");
    d.innerHTML = html;
    const rows = Array.from(d.querySelectorAll("table tr")).map((tr) => Array.from(tr.querySelectorAll("td")).map((td) => td.textContent));
    expect(rows).toEqual([
      ["确定", "OK"],
      ["取消", "Cancel"],
    ]);
  });

  it("a flex row is one table row, aligned as the page had it; a bold title made a block stays one", () => {
    const doc = page(
      `<header class="h"><h1>Fusion 360</h1><p class="sub">How to get a job onto the CNC</p></header><div><b class="t">Work in millimetres</b>Document units mm.</div>`,
      ".h { display: flex; justify-content: space-between; align-items: flex-end; column-gap: 20px; } .t { display: block; }",
    );
    const d = document.createElement("div");
    d.innerHTML = inlinePage(doc) as string;
    const cells = Array.from(d.querySelectorAll("table tr td")) as HTMLElement[];
    expect(cells.map((c) => c.textContent)).toEqual(["Fusion 360", "How to get a job onto the CNC"]);
    expect(cells[0].getAttribute("style")).toContain("vertical-align:bottom");
    expect(cells[0].getAttribute("style")).toContain("padding-right:20px");
    expect(cells[1].getAttribute("style")).toContain("text-align:right");
    expect(d.querySelector("b")!.getAttribute("style")).toContain("display:block");
  });

  it("a flex row whose first item is loose text (a section bar) still splits into its two cells", () => {
    const doc = page(
      `<h2 class="bar">1 · In Fusion 360 <small>Manufacture workspace</small></h2>`,
      ".bar { display: flex; justify-content: space-between; align-items: baseline; }",
    );
    const d = document.createElement("div");
    d.innerHTML = inlinePage(doc) as string;
    expect(Array.from(d.querySelectorAll("td")).map((c) => c.textContent?.trim())).toEqual(["1 · In Fusion 360", "Manufacture workspace"]);
  });

  it("a centred page grid is a table no wider than it was, without side margins", () => {
    const doc = page(`<div class="pg"><p>a</p><p>b</p></div>`, ".pg { display: grid; width: 300px; margin: 0 auto; row-gap: 10px; }");
    const d = document.createElement("div");
    d.innerHTML = inlinePage(doc) as string;
    const style = d.querySelector("table")!.getAttribute("style")!;
    expect(style).not.toMatch(/margin-(left|right):\d/);
    expect(d.querySelector("td")!.getAttribute("style")).toContain("padding-bottom:10px");
  });

  it("is sent to the page as one self-contained script", () => {
    const s = inlinePageScript();
    expect(inlinePage(page("<p>x</p>"))).not.toHaveProperty("error");
    expect(s.startsWith("(")).toBe(true);
    expect(s.endsWith(")(document)")).toBe(true);
    // runs on its own, without this module around it
    const doc = page("<p>Hi</p>");
    const run = new Function("document", `return ${s}`);
    expect(run(doc)).toContain(">Hi</p>");
  });
});
