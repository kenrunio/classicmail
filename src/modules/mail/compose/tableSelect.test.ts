import { describe, expect, it } from "vitest";
import { gridOf } from "./pageTables";
import { pickFor } from "./tableSelect";

const table = (rows: number, cols: number) => {
  const t = document.createElement("table");
  for (let r = 0; r < rows; r++) {
    const tr = t.insertRow();
    for (let c = 0; c < cols; c++) tr.insertCell().textContent = `${r}${c}`;
  }
  return gridOf(t);
};

describe("picking table cells", () => {
  it("a selection across two or more rows takes the whole rows", () => {
    expect(pickFor(table(4, 3), { left: 1, right: 2, top: 1, bottom: 3 })).toEqual({ rect: { left: 0, right: 3, top: 1, bottom: 3 }, whole: "rows" });
  });
  it("cells in one row stay just those cells; the whole row counts as a row", () => {
    expect(pickFor(table(4, 3), { left: 0, right: 2, top: 1, bottom: 2 })).toEqual({ rect: { left: 0, right: 2, top: 1, bottom: 2 }, whole: null });
    expect(pickFor(table(4, 3), { left: 0, right: 3, top: 1, bottom: 2 }).whole).toBe("rows");
  });
});
