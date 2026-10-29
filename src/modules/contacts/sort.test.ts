import { describe, expect, it } from "vitest";
import { emptyContact, type Contact } from "../../api/contacts";
import { jumpToKey } from "./CardView";
import { collator, compareText, indexKeyFor, shownFileAs, sortContacts } from "./sort";

function ct(id: number, givenName: string, familyName: string, company = "", firstSeenAt = 0, updated = 0, fileAs?: string): Contact {
  return {
    ...emptyContact(),
    id,
    givenName,
    familyName,
    company,
    firstSeenAt,
    updated,
    fileAs: fileAs ?? ([familyName, givenName].filter(Boolean).join(", ") || company),
    displayName: [givenName, familyName].filter(Boolean).join(" ") || company,
  };
}

const ALL = [
  ct(1, "Sarah", "Cohen", "Harbor Capital", 300, 10),
  ct(2, "Adam", "Zimmer", "", 100, 30),
  ct(3, "משה", "כהן", "בנק לאומי", 200, 20),
  ct(4, "Noa", "Abramson", "Atlas Fund", 400, 40),
  ct(5, "Élodie", "Ében", "", 500, 50),
];
const ids = (key: string, desc = false, order: "lastFirst" | "firstLast" = "lastFirst") => sortContacts(ALL, { key, desc }, order).map((c) => c.id);

describe("contact sorting", () => {
  it("collates case/accent-insensitively with Hebrew after Latin", () => {
    const words = ["zed", "כהן, משה", "Élodie", "3M", "adam", "אבוטבול", "Eli"];
    expect([...words].sort(compareText)).toEqual(["3M", "adam", "Eli", "Élodie", "zed", "אבוטבול", "כהן, משה"]);
    // vowel points and final letters don't matter at the primary level
    expect(collator.compare("שָׁלוֹם", "שלום")).toBe(0);
    expect(collator.compare("מלך", "מלכ")).toBe(0);
  });

  it("sorts by each key, both directions", () => {
    expect(ids("fileAs")).toEqual([4, 1, 5, 2, 3]);
    expect(ids("fileAs", true)).toEqual([3, 2, 5, 1, 4]);
    expect(ids("firstName")).toEqual([2, 5, 4, 1, 3]);
    expect(ids("lastName")).toEqual([4, 1, 5, 2, 3]);
    expect(ids("company")).toEqual([4, 1, 3, 5, 2]); // no company: last, by File As
    expect(ids("company", true)).toEqual([3, 1, 4, 2, 5]); // …still last when descending
    expect(ids("added")).toEqual([2, 3, 1, 4, 5]);
    expect(ids("modified", true)).toEqual([5, 4, 2, 3, 1]);
    expect(ids("mobile")).toEqual(ids("fileAs")); // unknown keys: File As
  });

  it("First Last display order changes File As text and order", () => {
    expect(shownFileAs(ALL[0], "firstLast")).toBe("Sarah Cohen");
    expect(shownFileAs({ ...ALL[0], fileAs: "Harbor Capital (Cohen, Sarah)" }, "firstLast")).toBe("Harbor Capital (Cohen, Sarah)");
    expect(ids("fileAs", false, "firstLast")).toEqual([2, 5, 4, 1, 3]);
  });

  it("index tabs follow the sort key", () => {
    const sarah = ALL[0];
    expect(indexKeyFor(sarah, "fileAs", "lastFirst")).toBe("c");
    expect(indexKeyFor(sarah, "firstName", "lastFirst")).toBe("s");
    expect(indexKeyFor(sarah, "company", "lastFirst")).toBe("h");
    expect(indexKeyFor(ALL[1], "company", "lastFirst")).toBeNull();
    expect(indexKeyFor(ALL[2], "lastName", "lastFirst")).toBe("א");
    expect(indexKeyFor(sarah, "added", "lastFirst")).toBeNull();
  });

  it("index tab jumps work ascending and descending", () => {
    const asc = sortContacts(ALL, { key: "company", desc: false }, "lastFirst").map((c) => indexKeyFor(c, "company", "lastFirst"));
    expect(asc).toEqual(["a", "h", "א", null, null]);
    expect(jumpToKey(asc, "b", false)).toBe(1);
    expect(jumpToKey(asc, "א", false)).toBe(2);
    expect(jumpToKey(asc, "z", false)).toBe(2);
    const desc = sortContacts(ALL, { key: "lastName", desc: true }, "lastFirst").map((c) => indexKeyFor(c, "lastName", "lastFirst"));
    expect(desc).toEqual(["א", "z", "e", "c", "a"]);
    expect(jumpToKey(desc, "d", true)).toBe(3);
    expect(jumpToKey(desc, "123", true)).toBe(4);
  });
});
