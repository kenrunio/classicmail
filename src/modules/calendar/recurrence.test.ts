import { describe, expect, it } from "vitest";
import { format } from "date-fns";
import { buildRRule, describeRecurrence, parseRecurrence } from "./recurrence";
import { SHORT_DATE, shortDatePattern } from "./dates";

const mon = new Date(2026, 9, 12, 9, 0); // Monday Oct 12 2026

describe("recurrence model", () => {
  it("round-trips weekly with interval", () => {
    const p = parseRecurrence(["RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE"], mon);
    expect(p.freq).toBe("weekly");
    expect(p.interval).toBe(2);
    expect(p.weekDays).toEqual([1, 3]);
    expect(buildRRule(p, false)).toBe("RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE");
  });

  it("maps every weekday", () => {
    const p = parseRecurrence(["RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"], mon);
    expect(p.freq).toBe("daily");
    expect(p.everyWeekday).toBe(true);
    expect(buildRRule(p, false)).toBe("RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR");
  });

  it("monthly nth and last weekday", () => {
    expect(parseRecurrence(["RRULE:FREQ=MONTHLY;BYDAY=2TU"], mon)).toMatchObject({ freq: "monthly", mode: "nth", nth: 2, nthKind: "TU" });
    const last = parseRecurrence(["RRULE:FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1"], mon);
    expect(last).toMatchObject({ mode: "nth", nth: -1, nthKind: "weekday" });
    expect(buildRRule(last, false)).toBe("RRULE:FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1");
  });

  it("count / until", () => {
    const p = parseRecurrence(["RRULE:FREQ=DAILY;COUNT=5"], mon);
    expect(p.end).toBe("count");
    expect(p.count).toBe(5);
    const u = parseRecurrence(["RRULE:FREQ=DAILY;UNTIL=20261231"], mon);
    expect(u.until).toBe("2026-12-31");
    expect(buildRRule(u, true)).toBe("RRULE:FREQ=DAILY;UNTIL=20261231");
  });

  it("describes like Outlook", () => {
    const t = describeRecurrence(["RRULE:FREQ=WEEKLY;BYDAY=MO"], mon.getTime(), mon.getTime() + 3600_000, false);
    // the date follows the machine's regional format (13/10/2026 in Israel, 10/12/2026 in the US)
    const d = format(mon, SHORT_DATE);
    expect(t.startsWith(`Occurs every Monday effective ${d} from `)).toBe(true);
    expect(describeRecurrence(["RRULE:FREQ=YEARLY;BYMONTH=10;BYMONTHDAY=12"], mon.getTime(), mon.getTime(), true)).toBe(`Occurs every October 12 effective ${d}.`);
  });
});

describe("regional date format", () => {
  it("derives the short date pattern from the locale", () => {
    expect(shortDatePattern("en-US")).toBe("M/d/yyyy");
    expect(shortDatePattern("en-IL")).toBe("dd/MM/yyyy");
    expect(shortDatePattern("en-GB")).toBe("dd/MM/yyyy");
    expect(shortDatePattern("de-DE")).toBe("d.M.yyyy");
  });
});
