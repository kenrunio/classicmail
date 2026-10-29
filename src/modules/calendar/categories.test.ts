import { describe, expect, it } from "vitest";
import { effectiveCategories, itemColors, matchedCategories } from "./categories";
import type { Category } from "../../api/calendar";

const cats: Category[] = [
  { name: "Acme", color: 1, shortcut: "", matchWords: ["ACMX"] },
  { name: "Orion", color: 8, shortcut: "" },
  { name: "Travel", color: 5, shortcut: "", autoMatch: false },
  { name: "ישיבה", color: 9, shortcut: "" },
  { name: "NoColour", color: 0, shortcut: "" },
];

describe("category auto-match", () => {
  it("matches whole words in any case, including match words and Hebrew", () => {
    expect(matchedCategories("Acme analyst day", cats)).toEqual(["Acme"]);
    expect(matchedCategories("call re acmx pre-announcement", cats)).toEqual(["Acme"]);
    expect(matchedCategories("Acmeology summit", cats)).toEqual([]);
    expect(matchedCategories("Orion / Acme joint dinner", cats)).toEqual(["Acme", "Orion"]);
    expect(matchedCategories("ישיבה עם המשקיעים", cats)).toEqual(["ישיבה"]);
    expect(matchedCategories("Travel to NYC", cats)).toEqual([]); // auto-match off
    expect(matchedCategories("NoColour meeting", cats)).toEqual([]); // no colour → nothing to show
  });
  it("puts assigned categories first and adds matched ones, without duplicates", () => {
    const ev = { summary: "Orion review", categories: ["Travel", "orion"], calendarColor: "#000" };
    expect(effectiveCategories(ev, cats)).toEqual(["Travel", "orion"]);
    expect(itemColors({ summary: "Acme x Orion", calendarColor: "#000" }, cats)).toEqual(["#e7a1a2", "#9db7e8"]);
    expect(itemColors({ summary: "Lunch", calendarColor: "#123456" }, cats)).toEqual(["#123456"]);
  });
});
