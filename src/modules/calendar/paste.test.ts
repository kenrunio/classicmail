import { describe, expect, it } from "vitest";
import { pasteTimes } from "./actions";
import type { CalEvent } from "../../api/calendar";

const at = (d: number, h: number, m = 0) => new Date(2026, 10, d, h, m).getTime();
const item = (id: number, start: number, end: number, allDay = false) => ({ id, start, end, allDay }) as CalEvent;

describe("pasteTimes", () => {
  it("keeps the copied items the same time apart, the first at the slot", () => {
    const clip = [item(1, at(9, 10), at(9, 11)), item(2, at(9, 13, 30), at(9, 14))];
    const t = pasteTimes(clip, { start: at(12, 15), end: at(12, 15, 30), allDay: false });
    expect(t).toEqual([
      { start: at(12, 15), end: at(12, 16) },
      { start: at(12, 18, 30), end: at(12, 19) },
    ]);
  });

  it("a day picked (Month, the all-day row): the items' own times there; all-day ones on that day", () => {
    const clip = [item(3, at(9, 0), at(10, 0), true), item(1, at(9, 10), at(9, 11)), item(2, at(9, 14), at(9, 15))];
    const t = pasteTimes(clip, { start: at(20, 0), end: at(21, 0), allDay: true });
    expect(t).toEqual([
      { start: at(20, 0), end: at(21, 0) },
      { start: at(20, 10), end: at(20, 11) },
      { start: at(20, 14), end: at(20, 15) },
    ]);
  });
});
