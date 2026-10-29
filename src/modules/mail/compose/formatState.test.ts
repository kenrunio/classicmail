import { describe, expect, it } from "vitest";
import { combine, firstFamily, ptOf, type Fmt } from "./formatState";

const f = (o: Partial<Fmt>): Fmt => ({ family: "Calibri", size: 11, bold: false, italic: false, underline: false, strike: false, ...o });

describe("Font group state", () => {
  it("one value when the selection agrees, blank when it is mixed", () => {
    expect(combine([f({}), f({})])).toMatchObject({ family: "Calibri", size: 11 });
    expect(combine([f({}), f({ family: "Georgia", size: 14 })])).toMatchObject({ family: null, size: null });
    expect(combine([])).toBeNull();
  });
  it("a button is pressed only when all of the selection has it", () => {
    expect(combine([f({ bold: true }), f({ bold: true })])!.bold).toBe(true);
    expect(combine([f({ bold: true }), f({})])!.bold).toBe(false);
  });
  it("reads font names and sizes", () => {
    expect(firstFamily("'Times New Roman', serif")).toBe("Times New Roman");
    expect(ptOf("11pt")).toBe(11);
    expect(ptOf("14.6667px")).toBe(11);
    expect(ptOf("16px")).toBe(12);
    expect(ptOf(null)).toBeNull();
  });
});
