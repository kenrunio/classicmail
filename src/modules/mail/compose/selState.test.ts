import { afterEach, describe, expect, it } from "vitest";
import { resetSelState, trackSelection, useSelState } from "./selState";

const frame = () => new Promise((r) => requestAnimationFrame(() => r(null)));

describe("selection state for the ribbon", () => {
  let stop = () => {};
  afterEach(() => {
    stop();
    resetSelState();
  });

  it("follows the caret into a table, and leaves the store alone when nothing it shows changed", async () => {
    const page = document.createElement("div");
    page.innerHTML = "<p>one two</p><table><tbody><tr><td>a</td><td>b</td></tr></tbody></table>";
    document.body.replaceChildren(page);
    let saved: Range | null = null;
    stop = trackSelection(
      () => page,
      (r) => (saved = r),
    );
    let updates = 0;
    const unsub = useSelState.subscribe(() => updates++);
    const caret = (n: Node, o: number) => {
      const r = document.createRange();
      r.setStart(n, o);
      getSelection()!.removeAllRanges();
      getSelection()!.addRange(r);
      document.dispatchEvent(new Event("selectionchange"));
    };

    caret(page.querySelector("td")!.firstChild!, 0);
    await frame();
    expect(saved).not.toBeNull();
    expect(useSelState.getState().table).toBe("cell");
    const after = updates;

    // another place in the same kind of cell: same ribbon, no update
    caret(page.querySelectorAll("td")[1].firstChild!, 1);
    await frame();
    expect(updates).toBe(after);

    caret(page.querySelector("p")!.firstChild!, 2);
    await frame();
    expect(useSelState.getState().table).toBe("");
    unsub();
  });
});
