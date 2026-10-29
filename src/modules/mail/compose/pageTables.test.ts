import { describe, expect, it } from "vitest";
import {
  deleteCols,
  deleteRows,
  gridOf,
  insertCol,
  insertRow,
  mergeCells,
  setBorders,
  shadeCells,
  sidesFor,
  sizeTable,
  splitCell,
  tableSelection,
  toggleHeaderRow,
  type TableSel,
} from "./pageTables";

// the table Claude wrote: header row of th, 4 columns
function setup(): HTMLElement {
  const root = document.createElement("div");
  const row = (cells: string[], tag = "td") => `<tr>${cells.map((c) => `<${tag}>${c}</${tag}>`).join("")}</tr>`;
  root.innerHTML = `<p>Overall:</p><table><tbody>${row(["", "Before", "After", "Change"], "th")}${row(["Target", "$190.0", "$180.7", "-4.9%"])}${row(["Revenue", "$756.4M", "$801.0M", "+5.9%"])}${row(["EPS", "$4.80", "$5.10", "+6.3%"])}</tbody></table>`;
  document.body.appendChild(root);
  return root;
}
const text = (root: HTMLElement) =>
  Array.from(root.querySelectorAll("tr")).map((tr) =>
    Array.from(tr.cells)
      .map((c) => c.textContent)
      .join("|"),
  );
/** a selection from the cell holding `a` to the one holding `b` */
function sel(root: HTMLElement, a: string, b = a): TableSel {
  const find = (t: string) => Array.from(root.querySelectorAll("td, th")).find((c) => c.textContent === t)!.firstChild!;
  const r = document.createRange();
  r.setStart(find(a), 0);
  r.setEnd(find(b), 1);
  return tableSelection(r, root)!;
}

describe("tables on the page", () => {
  it("finds the table and the selected rectangle", () => {
    const root = setup();
    const s = sel(root, "$190.0", "$801.0M");
    expect(s.rect).toEqual({ left: 1, top: 1, right: 3, bottom: 3 });
    expect(s.multi).toBe(true);
    expect(tableSelection(document.createRange(), root)).toBeNull();
  });
  it("shading and borders", () => {
    const root = setup();
    shadeCells(sel(root, "EPS", "+6.3%"), "#fde9e7");
    expect(Array.from(root.querySelectorAll("tr")[3].cells).every((c) => c.style.backgroundColor === "rgb(253, 233, 231)")).toBe(true);
    // the cursor in one cell: outside borders go round the whole table
    setBorders(sel(root, "$756.4M"), "outside", { width: 2, color: "#c00000" });
    const cells = gridOf(root.querySelector("table")!).grid;
    expect(cells[0][0].style.borderTop).toBe("2px solid rgb(192, 0, 0)");
    expect(cells[3][3].style.borderRight).toBe("2px solid rgb(192, 0, 0)");
    expect(cells[1][1].style.borderTop).toBe("");
    expect(root.querySelector("table")!.style.borderCollapse).toBe("collapse");
  });
  it("insert and delete rows and columns", () => {
    const root = setup();
    insertRow(sel(root, "$756.4M"), "below");
    expect(text(root)).toHaveLength(5);
    expect(text(root)[3]).toBe("|||");
    insertCol(sel(root, "Before"), "left");
    expect(text(root)[0]).toBe("||Before|After|Change");
    expect(root.querySelectorAll("tr")[0].cells[1].tagName).toBe("TH"); // like its neighbour
    deleteCols(sel(root, "Before"));
    deleteRows(sel(root, "Target", "Revenue"));
    expect(text(root)).toEqual(["||After|Change", "|||", "EPS||$5.10|+6.3%"]);
  });
  it("merge and split", () => {
    const root = setup();
    expect(mergeCells(sel(root, "$190.0", "$180.7"))).toBe(true);
    const row = root.querySelectorAll("tr")[1];
    expect(row.cells).toHaveLength(3);
    expect(row.cells[1].colSpan).toBe(2);
    expect(row.cells[1].textContent).toBe("$190.0$180.7");
    // a row inserted under a merged cell follows the grid
    insertRow(sel(root, "Target"), "below");
    expect(root.querySelectorAll("tr")[2].cells[1].colSpan).toBe(2);
    expect(splitCell(sel(root, "$190.0$180.7"))).toBe(true);
    expect(root.querySelectorAll("tr")[1].cells).toHaveLength(4);
  });
  it("header row on and off", () => {
    const root = setup();
    toggleHeaderRow(sel(root, "Target"));
    expect(root.querySelectorAll("tr")[0].cells[1].tagName).toBe("TD");
    toggleHeaderRow(sel(root, "Target"));
    expect(root.querySelectorAll("tr")[0].cells[1].tagName).toBe("TH");
  });
});

describe("sidesFor", () => {
  const rect = { left: 0, top: 0, right: 3, bottom: 3 };
  it("outside: only the edges of the selection", () => {
    expect(sidesFor("outside", { left: 0, top: 0, right: 1, bottom: 1 }, rect)).toEqual(["Top", "Left"]);
    expect(sidesFor("outside", { left: 1, top: 1, right: 2, bottom: 2 }, rect)).toEqual([]);
  });
  it("inside: the edges between cells", () => {
    expect(sidesFor("inside", { left: 0, top: 0, right: 1, bottom: 1 }, rect)).toEqual(["Right", "Bottom"]);
  });
  it("one side", () => {
    expect(sidesFor("bottom", { left: 1, top: 2, right: 2, bottom: 3 }, rect)).toEqual(["Bottom"]);
    expect(sidesFor("bottom", { left: 1, top: 1, right: 2, bottom: 2 }, rect)).toEqual([]);
  });
});

describe("sizes on a table", () => {
  it("widths on a column's cells and its <col>, heights on a row", () => {
    const d = document.createElement("div");
    d.innerHTML = "<table><colgroup><col width='50'><col width='50'></colgroup><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>";
    const t = d.querySelector("table")!;
    sizeTable(t, { cols: new Map([[1, 120]]), rows: new Map([[0, 30]]) });
    expect(t.rows[0].cells[1].style.width).toBe("120px");
    expect(t.rows[1].cells[1].style.width).toBe("120px");
    expect(t.rows[0].cells[0].style.width).toBe("");
    expect((t.querySelectorAll("col")[1] as HTMLElement).style.width).toBe("120px");
    expect(t.querySelectorAll("col")[1].getAttribute("width")).toBeNull();
    expect(t.rows[0].cells[0].style.height).toBe("30px");
    expect(t.rows[1].cells[0].style.height).toBe("");
  });
});
