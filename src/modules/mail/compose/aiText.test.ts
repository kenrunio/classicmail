import { describe, expect, it } from "vitest";
import { textForAi } from "./aiText";

describe("textForAi (what Translate sends)", () => {
  it("keeps the text and its structure, drops a web page's markup", () => {
    const page = `
      <style>.x{color:red}</style>
      <div class="wrap" style="position:fixed;right:0"><button aria-label="נגישות">ד</button></div>
      <table role="presentation" width="600"><tr><td class="hero" style="font-family:Roboto">
        <h1 style="color:#123">עדכון 5: בחירות!</h1>
        <span style="color:gray">יום חמישי, 22 באוקטובר 2026</span>
      </td></tr><tr><td>
        <p style="margin:0" dir="rtl">קבוצות <strong>יקרות</strong>,</p>
        <p>פרטים <a href="https://first.org.il/x" class="btn" onclick="go()">כאן</a>.</p>
        <ul><li style="x">אחת</li><li>   </li></ul>
        <img src="cid:cm-img-0" alt="logo" width="100">
      </td></tr></table>
      <script>track()</script>`;
    expect(textForAi(page)).toBe(
      "<h1>עדכון 5: בחירות!</h1><p> יום חמישי, 22 באוקטובר 2026 </p>" +
        '<p>קבוצות <b>יקרות</b>,</p><p>פרטים <a href="https://first.org.il/x">כאן</a>.</p><ul><li>אחת</li></ul><p> <img src="cid:cm-img-0"> </p>',
    );
  });

  it("a plain reply stays as it was, without its attributes", () => {
    expect(textForAi('<p dir="auto" style="margin:0">Hi Dan,</p><p>See <b>attached</b>.</p>')).toBe("<p>Hi Dan,</p><p>See <b>attached</b>.</p>");
  });

  it("hidden text isn't sent", () => {
    expect(textForAi('<p>Shown</p><div style="display:none">preheader</div><p hidden>also hidden</p>')).toBe("<p>Shown</p>");
  });
});
