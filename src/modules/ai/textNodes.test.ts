import { describe, expect, it } from "vitest";
import { applyTranslation, batchTexts, collectTextNodes, worthTranslating } from "./textNodes";

function doc(html: string) {
  return new DOMParser().parseFromString(`<!doctype html><html><head><style>p{color:red}</style></head><body>${html}</body></html>`, "text/html");
}

describe("worthTranslating", () => {
  it("skips blanks, numbers, URLs and addresses", () => {
    expect(worthTranslating("שלום")).toBe(true);
    expect(worthTranslating("  Hello ")).toBe(true);
    expect(worthTranslating("   \n ")).toBe(false);
    expect(worthTranslating("12.5% / 2026")).toBe(false);
    expect(worthTranslating("https://example.com/ir?x=1")).toBe(false);
    expect(worthTranslating("www.example.com")).toBe(false);
    expect(worthTranslating("yael@tamar.example")).toBe(false);
  });
});

describe("collectTextNodes", () => {
  it("finds visible text in document order and skips styles, scripts, code and hidden parts", () => {
    const d = doc(`<div dir="rtl"><p>קני <b>שלום</b>,</p><p style="display:none">preheader</p><pre>code()</pre>
      <span hidden>secret</span><script>var x</script><p>תודה, <a href="https://x.example">https://x.example</a></p></div>`);
    const texts = collectTextNodes(d.body).map((n) => n.nodeValue);
    expect(texts).toEqual(["קני ", "שלום", "תודה, "]);
  });
});

describe("batchTexts", () => {
  it("splits by size and count, covering every item once", () => {
    expect(batchTexts([])).toEqual([]);
    expect(batchTexts(["aaa", "bbb", "ccc", "ddd"], 7, 10)).toEqual([
      [0, 2],
      [2, 4],
    ]);
    expect(batchTexts(["a", "b", "c", "d", "e"], 1000, 2)).toEqual([
      [0, 2],
      [2, 4],
      [4, 5],
    ]);
    // an over-long item gets a batch of its own
    expect(batchTexts(["a", "x".repeat(50), "b"], 10, 10)).toEqual([
      [0, 1],
      [1, 2],
      [2, 3],
    ]);
    const many = Array.from({ length: 333 }, (_, i) => "w".repeat((i * 17) % 120));
    const b = batchTexts(many);
    expect(b[0][0]).toBe(0);
    expect(b[b.length - 1][1]).toBe(333);
    for (let i = 1; i < b.length; i++) expect(b[i][0]).toBe(b[i - 1][1]);
  });
});

describe("applyTranslation", () => {
  it("replaces texts, flips RTL blocks to LTR, and restores everything", () => {
    const d = doc(`<div class="w" dir="auto"><div dir="rtl" align="right"><p>קני <b>שלום</b>,</p></div></div>`);
    const nodes = collectTextNodes(d.body);
    const before = d.body.innerHTML;
    const a = applyTranslation(d, nodes, ["Kenny ", "hello"], false);
    expect(d.body.textContent).toContain("Kenny hello,");
    expect(d.querySelector("[align]")!.getAttribute("dir")).toBe("ltr");
    expect(d.querySelector("[align]")!.getAttribute("align")).toBe("left");
    expect(d.querySelector(".w")!.getAttribute("dir")).toBe("ltr");
    a.restore();
    expect(d.body.innerHTML).toBe(before);
    expect(d.body.hasAttribute("dir")).toBe(false);
  });

  it("keeps the original text where a translation is missing", () => {
    const d = doc(`<p>אחת</p><p>שתיים</p>`);
    const nodes = collectTextNodes(d.body);
    applyTranslation(d, nodes, ["one"], false);
    expect(Array.from(d.querySelectorAll("p")).map((p) => p.textContent)).toEqual(["one", "שתיים"]);
  });
});
