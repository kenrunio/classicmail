import { describe, expect, it } from "vitest";
import { hourLabel } from "./DayWeekGrid";

describe("second time zone ruler", () => {
  it("shows the other zone's hour beside yours", () => {
    // 15:00 in Israel is 08:00 in New York (7 hours behind)
    expect(hourLabel(15 * 60 - 7 * 60, false)).toEqual({ n: "08", s: "00" });
    // past midnight wraps to the day before / after
    expect(hourLabel(2 * 60 - 7 * 60, false)).toEqual({ n: "19", s: "00" });
    expect(hourLabel(23 * 60 + 150, true)).toEqual({ n: "1", s: "30" });
    expect(hourLabel(13 * 60, true)).toEqual({ n: "1", s: "pm" });
  });
});
