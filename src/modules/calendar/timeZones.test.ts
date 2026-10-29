import { describe, expect, it } from "vitest";
import { epochOf, offsetAt, wallOf, zoneLabel, zoneList } from "./timeZones";

describe("time zones", () => {
  it("10:00 in New York is 17:00 in Jerusalem (winter)", () => {
    const t = epochOf("2026-12-07", 10 * 60, "America/New_York");
    expect(new Date(t).toISOString()).toBe("2026-12-07T15:00:00.000Z");
    expect(wallOf(t, "Asia/Jerusalem")).toEqual({ date: "2026-12-07", min: 17 * 60 });
    expect(wallOf(t, "America/New_York")).toEqual({ date: "2026-12-07", min: 10 * 60 });
  });

  it("follows daylight saving time on the day itself", () => {
    // New York is UTC-4 in summer, UTC-5 in winter (changes 1 Nov 2026)
    expect(new Date(epochOf("2026-10-30", 9 * 60, "America/New_York")).toISOString()).toBe("2026-10-30T13:00:00.000Z");
    expect(new Date(epochOf("2026-11-02", 9 * 60, "America/New_York")).toISOString()).toBe("2026-11-02T14:00:00.000Z");
    expect(offsetAt(Date.UTC(2026, 6, 1), "America/New_York")).toBe(-4 * 3600_000);
  });

  it("a late evening in one zone is the next day in another", () => {
    const t = epochOf("2026-12-07", 20 * 60, "America/New_York");
    expect(wallOf(t, "Asia/Jerusalem")).toEqual({ date: "2026-12-08", min: 3 * 60 });
  });

  it("offers a short list of regions west to east, by general names", () => {
    expect(zoneLabel("America/New_York", Date.UTC(2026, 11, 1))).toBe("(UTC-05:00) Eastern Time (US & Canada)");
    expect(zoneLabel("Asia/Kolkata", Date.UTC(2026, 11, 1))).toBe("(UTC+05:30) India");
    const list = zoneList(Date.UTC(2026, 11, 1));
    expect(list.length).toBeLessThan(40);
    const ny = list.findIndex((z) => z.tz === "America/New_York");
    const jer = list.findIndex((z) => z.tz === "Asia/Jerusalem");
    expect(ny).toBeGreaterThanOrEqual(0);
    expect(ny).toBeLessThan(jer);
    expect(list[jer].label).toBe("(UTC+02:00) Israel");
  });

  it("a zone the list doesn't name still shows, by its city", () => {
    const list = zoneList(Date.UTC(2026, 11, 1), ["America/Toronto"]);
    expect(list.find((z) => z.tz === "America/Toronto")?.label).toBe("(UTC-05:00) Toronto");
  });
});
