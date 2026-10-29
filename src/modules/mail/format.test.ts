import { describe, expect, it } from "vitest";
import { escapeHtml } from "./format";
import { textToHtml } from "./compose/editor";

describe("escapeHtml", () => {
  it("makes text and attribute values safe", () => {
    expect(escapeHtml(`<a href="x">Tom & Jerry</a>`)).toBe("&lt;a href=&quot;x&quot;&gt;Tom &amp; Jerry&lt;/a&gt;");
  });

  it("is what textToHtml uses for each line", () => {
    expect(textToHtml(`a < b\n"c"\n`)).toBe("<p>a &lt; b</p><p>&quot;c&quot;</p><p></p>");
  });
});
