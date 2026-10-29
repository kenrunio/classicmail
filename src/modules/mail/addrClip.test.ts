import { describe, expect, it } from "vitest";
import { addrClipText, addrsClipText, clickSelect, span } from "./addrClip";

describe("addrClipText", () => {
  it("writes Name <email>", () => {
    expect(addrClipText({ name: "Kenny Green", email: "kenny@duvz.demo" })).toBe("Kenny Green <kenny@duvz.demo>");
  });
  it("leaves out a name that is only the address", () => {
    expect(addrClipText({ name: "", email: "a@b.com" })).toBe("a@b.com");
    expect(addrClipText({ name: "A@B.com", email: "a@b.com" })).toBe("a@b.com");
  });
  it("quotes names with commas or semicolons so they paste back as one name", () => {
    expect(addrClipText({ name: "Green, Kenny", email: "k@x.com" })).toBe('"Green, Kenny" <k@x.com>');
    expect(addrClipText({ name: 'Say "hi"; ok', email: "k@x.com" })).toBe('"Say hi; ok" <k@x.com>');
  });
  it("keeps a name with no address", () => {
    expect(addrClipText({ name: "Sales team", email: "" })).toBe("Sales team");
  });
  it("joins with semicolons", () => {
    expect(
      addrsClipText([
        { name: "A", email: "a@x.com" },
        { name: "", email: "b@x.com" },
      ]),
    ).toBe("A <a@x.com>; b@x.com");
  });
});

describe("clickSelect", () => {
  it("click selects one", () => {
    expect(clickSelect([0, 1], 0, 2, {})).toEqual({ sel: [2], anchor: 2 });
  });
  it("ctrl+click toggles", () => {
    expect(clickSelect([0], 0, 2, { ctrl: true })).toEqual({ sel: [0, 2], anchor: 2 });
    expect(clickSelect([0, 2], 2, 0, { ctrl: true })).toEqual({ sel: [2], anchor: 0 });
  });
  it("shift+click selects from the anchor", () => {
    expect(clickSelect([3], 3, 1, { shift: true })).toEqual({ sel: [1, 2, 3], anchor: 3 });
  });
  it("span goes either way", () => {
    expect(span(2, 0)).toEqual([0, 1, 2]);
  });
});
