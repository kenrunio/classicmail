import { describe, expect, it } from "vitest";
import { withoutImages } from "./aiImages";

const PNG = "data:image/png;base64," + "A".repeat(30000);

describe("pictures kept away from the AI", () => {
  it("sends short references and puts the pictures back", () => {
    const html = `<p>Attached below is the approval.</p><p><img src="${PNG}" width="600"></p><p>Regards</p>`;
    const w = withoutImages(html);
    expect(w.html.length).toBeLessThan(200);
    expect(w.html).toContain('src="cid:cm-img-0"');
    // the AI's (translated) answer, the reference left where it was
    const answer = w.html.replace("Attached below is the approval.", "מצורף אישור ההעברה.").replace("Regards", "בברכה");
    const back = w.restore(answer);
    expect(back).toContain(PNG);
    expect(back).toContain("מצורף אישור ההעברה.");
    expect(back).toContain('width="600"');
  });
  it("a picture the AI dropped goes at the end instead of being lost", () => {
    const w = withoutImages(`<p>Hi</p><p><img src="${PNG}"></p>`);
    expect(w.restore("<p>שלום</p>")).toBe(`<p>שלום</p><p><img src="${PNG}"></p>`);
  });
  it("leaves HTML without pictures alone", () => {
    const w = withoutImages("<p>Hi</p>");
    expect(w.html).toBe("<p>Hi</p>");
    expect(w.restore("<p>שלום</p>")).toBe("<p>שלום</p>");
  });
});
