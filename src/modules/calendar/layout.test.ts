import { describe, expect, it } from "vitest";
import { buildGroups, gridColumns, groupTarget, layoutDay, layoutLanes, spanInGroup } from "./layout";

describe("layoutDay", () => {
  it("lines up categories: each key keeps its own columns in the cluster", () => {
    const p = layoutDay(
      [
        { id: 1, startMin: 480, endMin: 540, key: "arbe" }, // 8-9
        { id: 2, startMin: 540, endMin: 570, key: "allt" }, // 9-9:30
        { id: 3, startMin: 510, endMin: 600, key: "nxsn" }, // 8:30-10
        { id: 4, startMin: 570, endMin: 600, key: "arbe" }, // 9:30-10 → arbe's column, not the free first one
        { id: 5, startMin: 560, endMin: 620 }, // no category: last
      ],
      30,
      true,
    );
    const byId = Object.fromEntries(p.map((x) => [x.item.id, x]));
    expect(byId[1].col).toBe(0);
    expect(byId[4].col).toBe(0);
    expect(byId[3].col).toBe(1);
    expect(byId[2].col).toBe(2);
    expect(byId[5].col).toBe(3);
    expect(byId[1].cols).toBe(4);
  });

  it("lines up categories: the same columns all day; stretches only over absent categories", () => {
    const p = layoutDay(
      [
        { id: 1, startMin: 480, endMin: 540, key: "a" }, // 8-9
        { id: 2, startMin: 480, endMin: 540, key: "b" }, // 8-9
        { id: 3, startMin: 480, endMin: 540, key: "c" }, // 8-9
        { id: 4, startMin: 600, endMin: 660, key: "b" }, // 10-11, with c only
        { id: 5, startMin: 600, endMin: 660, key: "c" },
        { id: 6, startMin: 720, endMin: 780, key: "c" }, // 12-13 alone: the full width
      ],
      30,
      true,
    );
    const byId = Object.fromEntries(p.map((x) => [x.item.id, x]));
    expect([byId[4].col, byId[4].cols, byId[4].span]).toEqual([1, 3, 1]);
    expect([byId[5].col, byId[5].cols]).toEqual([2, 3]);
    expect([byId[6].col, byId[6].cols]).toEqual([0, 1]);
    // a's item can't stretch over b's (present in its cluster)
    expect(byId[1].span).toBe(1);
  });

  it("ignores keys unless asked", () => {
    const p = layoutDay(
      [
        { id: 1, startMin: 480, endMin: 540, key: "a" },
        { id: 2, startMin: 510, endMin: 600, key: "b" },
        { id: 3, startMin: 570, endMin: 600, key: "b" },
      ],
      30,
    );
    expect(p.find((x) => x.item.id === 3)!.col).toBe(0);
  });

  it("puts non-overlapping items in one column", () => {
    const p = layoutDay([
      { id: 1, startMin: 540, endMin: 600 },
      { id: 2, startMin: 600, endMin: 660 },
    ]);
    expect(p.map((x) => [x.col, x.cols])).toEqual([
      [0, 1],
      [0, 1],
    ]);
  });

  it("splits overlapping items and stretches over free columns", () => {
    const p = layoutDay([
      { id: 1, startMin: 540, endMin: 660 }, // 9-11
      { id: 2, startMin: 570, endMin: 600 }, // 9:30-10
      { id: 3, startMin: 600, endMin: 630 }, // 10-10:30 → reuses item 2's column
      { id: 4, startMin: 570, endMin: 620 }, // 9:30-10:20 → longer, so placed before item 2
    ]);
    const byId = Object.fromEntries(p.map((x) => [x.item.id, x]));
    expect(byId[1].cols).toBe(3);
    expect(byId[1].col).toBe(0);
    expect(byId[4].col).toBe(1);
    expect(byId[2].col).toBe(2);
    expect(byId[3].col).toBe(2);
    // item 2 can't stretch (last column); item 4 can't either (item 2 / 3 beside it)
    expect(byId[4].span).toBe(1);
  });

  it("treats short items as at least one slot long", () => {
    const p = layoutDay([
      { id: 1, startMin: 540, endMin: 545 },
      { id: 2, startMin: 550, endMin: 560 },
    ]);
    expect(p[1].col).toBe(1);
  });
});

describe("layoutLanes", () => {
  it("stacks overlapping spans", () => {
    const l = layoutLanes([
      { id: 1, first: 0, last: 2 },
      { id: 2, first: 1, last: 1 },
      { id: 3, first: 3, last: 4 },
    ]);
    const lane = Object.fromEntries(l.map((x) => [x.item.id, x.lane]));
    expect(lane).toEqual({ 1: 0, 2: 1, 3: 0 });
  });
});

describe("side-by-side groups", () => {
  const cals = [
    { id: 1, canWrite: true },
    { id: 2, canWrite: true },
    { id: 3, canWrite: false },
  ];
  it("one group per calendar side by side, overlay merges into the first", () => {
    expect(buildGroups(cals, "sideBySide", []).map((g) => g.map((c) => c.id))).toEqual([[1], [2], [3]]);
    expect(buildGroups(cals, "sideBySide", [3]).map((g) => g.map((c) => c.id))).toEqual([[1, 3], [2]]);
    expect(buildGroups(cals, "overlay", []).map((g) => g.map((c) => c.id))).toEqual([[1, 2, 3]]);
    expect(buildGroups(cals.slice(0, 1), "sideBySide", []).length).toBe(1);
    expect(buildGroups([], "sideBySide", [])).toEqual([[]]);
  });
  it("columns are group-major and spans map into the group", () => {
    const cols = gridColumns(5, 3);
    expect(cols.length).toBe(15);
    expect(cols[0]).toEqual({ day: 0, group: 0 });
    expect(cols[5]).toEqual({ day: 0, group: 1 });
    expect(cols[14]).toEqual({ day: 4, group: 2 });
    expect(spanInGroup(1, 3, 2, 5)).toEqual({ first: 11, last: 13 });
    expect(gridColumns(7, 0).length).toBe(7);
  });
  it("new items go to the group's first writable calendar", () => {
    expect(groupTarget([cals[2], cals[1]])?.id).toBe(2);
    expect(groupTarget([cals[2]])).toBeUndefined();
  });
});
