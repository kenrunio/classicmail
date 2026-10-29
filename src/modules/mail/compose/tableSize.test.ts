import { describe, expect, it } from "vitest";
import { lineAt } from "./tableSize";

/** A 2×2 table whose cells sit on a 100×20 grid (jsdom has no layout). */
function table() {
  const root = document.createElement("div");
  root.innerHTML = "<table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>";
  document.body.appendChild(root);
  root.querySelectorAll("td").forEach((td, i) => {
    const left = (i % 2) * 100;
    const top = Math.floor(i / 2) * 20;
    td.getBoundingClientRect = () => ({ left, right: left + 100, top, bottom: top + 20, width: 100, height: 20, x: left, y: top, toJSON: () => ({}) });
  });
  return { root, cells: root.querySelectorAll("td") };
}

const at = (target: Element, clientX: number, clientY: number) => ({ target, clientX, clientY }) as unknown as MouseEvent;

describe("lineAt", () => {
  it("finds the column / row line under the pointer", () => {
    const { root, cells } = table();
    expect(lineAt(at(cells[0], 99, 10), root)).toMatchObject({ kind: "col", index: 0 });
    expect(lineAt(at(cells[1], 101, 10), root)).toMatchObject({ kind: "col", index: 0 });
    expect(lineAt(at(cells[0], 50, 19), root)).toMatchObject({ kind: "row", index: 0 });
  });

  it("is nothing away from the lines, and on the table's own outer left / top edge", () => {
    const { root, cells } = table();
    expect(lineAt(at(cells[0], 50, 10), root)).toBeNull();
    expect(lineAt(at(cells[0], 1, 10), root)).toBeNull();
    expect(lineAt(at(cells[0], 50, 1), root)).toBeNull();
  });
});
