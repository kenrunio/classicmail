import { describe, expect, it } from "vitest";
import { allDayRange, atMinutes, DAY_FIRST, parseDateText, sameLengthEnd, wallMinutes } from "./dates";

// Israel's clocks go back an hour at 02:00 on Sunday 25 October 2026 (a 25-hour day).
const israel = Intl.DateTimeFormat().resolvedOptions().timeZone === "Asia/Jerusalem";

describe("wall-clock minutes", () => {
  it("places items by the clock, not by elapsed time", () => {
    const day = new Date(2026, 9, 25);
    const tenAm = new Date(2026, 9, 25, 10, 0).getTime();
    expect(wallMinutes(tenAm, day)).toBe(600);
    expect(atMinutes(day, 600)).toBe(tenAm);
    if (israel) expect((tenAm - day.getTime()) / 60000).toBe(660); // the bug this guards against
  });

  it("clamps to the day", () => {
    const day = new Date(2026, 9, 12);
    expect(wallMinutes(new Date(2026, 9, 11, 23).getTime(), day)).toBe(0);
    expect(wallMinutes(new Date(2026, 9, 13, 0, 0).getTime(), day)).toBe(1440);
    expect(wallMinutes(new Date(2026, 9, 12, 13, 30).getTime(), day)).toBe(810);
  });
});

// Israel's clocks go forward at 02:00 on Friday 27 March 2026 (a 23-hour day).
describe("whole days across DST", () => {
  it("keeps the last day of a selection that ends on the 25-hour day", () => {
    const r = allDayRange(new Date(2026, 9, 24).getTime(), new Date(2026, 9, 26).getTime());
    expect(r).toEqual({ start: new Date(2026, 9, 24).getTime(), end: new Date(2026, 9, 26).getTime() });
  });

  it("gives at least one day", () => {
    const r = allDayRange(new Date(2026, 2, 27, 9).getTime(), new Date(2026, 2, 27, 9, 30).getTime());
    expect(r).toEqual({ start: new Date(2026, 2, 27).getTime(), end: new Date(2026, 2, 28).getTime() });
  });

  it("moves an all-day item by days, not hours", () => {
    // a one-day item moved onto the 23-hour day, and off the 25-hour day, stays one day
    expect(sameLengthEnd("2026-03-20", "2026-03-20", "2026-03-27")).toBe("2026-03-27");
    expect(sameLengthEnd("2026-10-25", "2026-10-25", "2026-10-27")).toBe("2026-10-27");
    expect(sameLengthEnd("2026-10-24", "2026-10-26", "2026-11-01")).toBe("2026-11-03");
  });
});

describe("parseDateText", () => {
  // Wednesday 14 Oct 2026
  const now = new Date(2026, 9, 14, 10, 0);
  const ymd = (d: Date | null) => d && [d.getFullYear(), d.getMonth() + 1, d.getDate()];

  it("reads a two-digit year as this century, not the year 26", () => {
    expect(ymd(parseDateText(DAY_FIRST ? "13/1/26" : "1/13/26", now))).toEqual([2026, 1, 13]);
    expect(ymd(parseDateText(DAY_FIRST ? "13/01/2026" : "01/13/2026", now))).toEqual([2026, 1, 13]);
  });

  it("ignores a leading weekday that no longer matches the date", () => {
    // "Tue 13/10/2026" edited to the 21st (a Wednesday) is the 21st, not that week's Tuesday
    expect(ymd(parseDateText(DAY_FIRST ? "Tue 21/10/2026" : "Tue 10/21/2026", now))).toEqual([2026, 10, 21]);
  });

  it("understands words, weekdays and relative days", () => {
    expect(ymd(parseDateText("tomorrow", now))).toEqual([2026, 10, 15]);
    expect(ymd(parseDateText("fri", now))).toEqual([2026, 10, 16]);
    expect(ymd(parseDateText("+3", now))).toEqual([2026, 10, 17]);
    expect(ymd(parseDateText("2026/12/1", now))).toEqual([2026, 12, 1]);
    expect(ymd(parseDateText(DAY_FIRST ? "1/12" : "12/1", now))).toEqual([2026, 12, 1]);
    expect(parseDateText("banana", now)).toBeNull();
  });
});
