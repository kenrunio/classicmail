import { describe, expect, it } from "vitest";
import { braceUnits, machineMade, maskedHtml, unitsToReview, unmask } from "./quotedInstructions";

const body = (html: string) => {
  const d = document.createElement("div");
  d.innerHTML = html;
  return d;
};

// an Outlook original reused as a template: its own braces, square brackets and CSS
const ORIGINAL = `<div id="cm-original"><p>Dear all,</p><div class="WordSection1"><p class="MsoNormal"><span>Please book the call for
November 19.</span></p><p>Mail [mailto:a@b.com] ref {0}</p><p>Subject: [EXTERNAL] {Q3 call}</p></div><style>p{margin:0}</style></div>`;

describe("braceUnits", () => {
  it("finds the blocks holding {…}, not [ ] or CSS", () => {
    const u = braceUnits(body(ORIGINAL));
    expect(u.map((x) => x.found)).toEqual([["0"], ["Q3 call"]]);
  });
  it("reads an instruction split over formatting and source line breaks", () => {
    const u = braceUnits(body("<p>for {please add <b>day</b> of the\nweek} November</p>"));
    expect(u[0].found).toEqual(["please add day of the week"]);
  });
  it("leaves mail-merge fields alone", () => {
    expect(braceUnits(body("<p>Hi {First Name}</p>"), ["first name"])).toEqual([]);
  });
});

describe("unitsToReview", () => {
  const before = braceUnits(body(ORIGINAL)).flatMap((u) => u.found);
  const typed = ORIGINAL.replace("for\nNovember 19.", "for {please add day of the week} November 19.");

  it("picks only what the user added to the original", () => {
    const r = unitsToReview(braceUnits(body(typed)), before);
    expect(r).toHaveLength(1);
    expect(r[0].el.textContent).toContain("Please book the call");
    expect(r[0].keep).toEqual([]);
  });
  it("finds nothing in an untouched original", () => {
    expect(unitsToReview(braceUnits(body(ORIGINAL)), before)).toEqual([]);
  });
  it("a reopened draft (no baseline) skips machine-made braces", () => {
    const r = unitsToReview(braceUnits(body(typed.replace("{Q3 call}", "{ID}"))), null);
    expect(r.map((x) => x.el.textContent)).toEqual(["Please book the call for {please add day of the week} November 19."]);
  });
  it("keeps the original's braces that share a block with a new one", () => {
    const r = unitsToReview(braceUnits(body("<p>Hi {first names}, ref {0}</p>")), ["0"]);
    expect(r[0].keep).toEqual(["0"]);
  });
});

describe("masking", () => {
  it("hides kept braces from the AI and brings them back", () => {
    const p = body("<p>Hi {first names}, <b>ref {0}</b></p>").firstElementChild!;
    const m = maskedHtml(p, ["0"]);
    expect(m).toBe("Hi {first names}, <b>ref ⦃0⦄</b>");
    expect(unmask(m)).toBe("Hi {first names}, <b>ref {0}</b>");
  });
});

describe("machineMade", () => {
  it("tells codes and links from a person's instruction", () => {
    for (const s of ["0", "ID", "https://x.com", "a@b.com", "mailto:a@b.com"]) expect(machineMade(s)).toBe(true);
    for (const s of ["please add day of the week", "thank John", "Dan's title"]) expect(machineMade(s)).toBe(false);
  });
});
