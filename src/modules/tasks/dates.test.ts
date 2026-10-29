import { describe, expect, it } from "vitest";
import { dueGroup, flagDates, isOverdue, parseDateInput, dueInfo, dayKey } from "./dates";

// Wednesday 14 Oct 2026, 10:00
const now = new Date(2026, 9, 14, 10, 0);

describe("dueGroup", () => {
  it("groups like Outlook's Arranged By: Due Date", () => {
    expect(dueGroup(null, now)).toBe("noDate");
    expect(dueGroup("2026-10-10", now)).toBe("today"); // overdue sits in Today
    expect(dueGroup("2026-10-14", now)).toBe("today");
    expect(dueGroup("2026-10-15", now)).toBe("tomorrow");
    expect(dueGroup("2026-10-17", now)).toBe("thisWeek"); // Saturday
    expect(dueGroup("2026-10-18", now)).toBe("nextWeek"); // Sunday
    expect(dueGroup("2026-10-24", now)).toBe("nextWeek");
    expect(dueGroup("2026-10-25", now)).toBe("later");
  });
  it("tomorrow wins over next week on Saturday", () => {
    const sat = new Date(2026, 9, 17);
    expect(dueGroup("2026-10-18", sat)).toBe("tomorrow");
    expect(dueGroup("2026-10-19", sat)).toBe("nextWeek");
  });
});

describe("flags", () => {
  it("sets Outlook's start/due dates", () => {
    expect(flagDates("today", now)).toEqual({ start: "2026-10-14", due: "2026-10-14" });
    expect(flagDates("tomorrow", now)).toEqual({ start: "2026-10-15", due: "2026-10-15" });
    expect(flagDates("thisWeek", now)).toEqual({ start: "2026-10-16", due: "2026-10-16" });
    expect(flagDates("nextWeek", now)).toEqual({ start: "2026-10-19", due: "2026-10-23" });
    expect(flagDates("noDate", now)).toEqual({ start: null, due: null });
  });
  it("overdue", () => {
    expect(isOverdue("2026-10-13", false, now)).toBe(true);
    expect(isOverdue("2026-10-13", true, now)).toBe(false);
    expect(isOverdue("2026-10-14", false, now)).toBe(false);
  });
});

describe("parseDateInput", () => {
  it("understands words and relative dates", () => {
    expect(parseDateInput("None", now)).toBeNull();
    expect(parseDateInput("", now)).toBeNull();
    expect(parseDateInput("today", now)).toBe("2026-10-14");
    expect(parseDateInput("Tomorrow", now)).toBe("2026-10-15");
    expect(parseDateInput("fri", now)).toBe("2026-10-16");
    expect(parseDateInput("wed", now)).toBe("2026-10-21");
    expect(parseDateInput("+3", now)).toBe("2026-10-17");
    expect(parseDateInput("10 days", now)).toBe("2026-10-24");
    expect(parseDateInput("2026-12-01", now)).toBe("2026-12-01");
    expect(parseDateInput("banana", now)).toBeUndefined();
  });
  it("round-trips the displayed format", () => {
    const d = new Date(2026, 10, 3);
    const shown = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "numeric", year: "numeric" }).format(d);
    expect(parseDateInput(`Tue ${shown}`, now)).toBe(dayKey(d));
  });
});

describe("dueInfo", () => {
  it("describes the due date", () => {
    expect(dueInfo("2026-10-14", false, null, now)).toBe("Due today.");
    expect(dueInfo("2026-10-16", false, null, now)).toBe("Due in 2 days.");
    expect(dueInfo("2026-10-11", false, null, now)).toBe("Overdue by 3 days.");
    expect(dueInfo(null, false, null, now)).toBeNull();
  });
});
