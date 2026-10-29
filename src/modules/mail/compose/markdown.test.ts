import { describe, expect, it } from "vitest";
import { htmlToMarkdown, looksLikeMarkdown, markdownToHtml } from "./markdown";

describe("Markdown in a message", () => {
  it("tells Markdown from an ordinary note", () => {
    expect(looksLikeMarkdown("# Results\n\n- Revenue up\n- Margin flat")).toBe(true);
    expect(looksLikeMarkdown("| Item | Value |\n|---|---|\n| A | 1 |")).toBe(true);
    expect(looksLikeMarkdown("```\ncode\n```")).toBe(true);
    expect(looksLikeMarkdown("See **this** and [the deck](https://x.example/deck)")).toBe(true);
    // ordinary mail: a dash or a hash alone isn't Markdown
    expect(looksLikeMarkdown("Hi Dan - see you at 3.\n#2 on the list is done.")).toBe(false);
    expect(looksLikeMarkdown("Thanks,\nKenny")).toBe(false);
  });

  it("becomes e-mail formatting: tables with lines, nothing that runs", () => {
    const html = markdownToHtml("## Q3\n\n**Up** 12%\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n<script>x()</script>[x](javascript:alert(1))");
    expect(html).toContain("<h2");
    expect(html).toContain("<strong>Up</strong>");
    expect(html).toMatch(/<td style="border:1px solid #a6a6a6/);
    expect(html).not.toContain("<script");
    expect(html).not.toMatch(/href="\s*javascript:/);
  });

  it("a message turns into Markdown and back", () => {
    const md = htmlToMarkdown("<h2>Q3</h2><p>Revenue <b>up</b>.</p><ul><li>One</li><li>Two</li></ul><table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table>");
    expect(md).toBe("## Q3\n\nRevenue **up**.\n\n-   One\n-   Two\n\n| A | B |\n| --- | --- |\n| 1 | 2 |");
    const back = markdownToHtml(md);
    expect(back).toContain("<table");
    expect(back).toContain("<li");
  });
});
