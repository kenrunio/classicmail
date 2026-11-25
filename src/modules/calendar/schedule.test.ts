import { describe, expect, it } from "vitest";
import { clashText, prefillForm, type Form } from "./schedule";
import type { Prepared } from "../../api/aiSchedule";

const blank = (): Form => ({
  summary: "",
  location: "",
  description: "",
  allDay: false,
  startDate: "2026-11-25",
  startMin: 17 * 60,
  endDate: "2026-11-25",
  endMin: 17 * 60 + 30,
  attendees: [],
  reminder: -2,
  busyStatus: "busy",
  isPrivate: false,
  importance: "normal",
  recurrence: null,
  meeting: false,
  categories: [],
  timeZone: "Asia/Jerusalem",
});

const prepared = (over: Partial<Prepared> = {}): Prepared => ({
  found: true,
  calendarId: 7,
  start: Date.UTC(2026, 11, 8, 15),
  end: Date.UTC(2026, 11, 8, 15, 30),
  timeZone: "America/New_York",
  subject: "Throughline / Camtek",
  location: "https://zoom.us/j/111",
  notes: "https://zoom.us/j/111\n\nCall between Moshe Eisenberg (CFO, Camtek)",
  attendees: [
    { name: "Moshe Eisenberg", email: "moshee@camtek.com", title: "CFO", company: "Camtek", optional: false },
    { name: "", email: "friend@gmail.com", title: null, company: null, optional: true },
  ],
  alreadyInvited: [],
  clashes: [],
  info: [],
  ...over,
});

describe("prefillForm", () => {
  it("fills the new meeting from the prepared result", () => {
    const f = prefillForm(blank(), prepared());
    expect([f.startDate, f.startMin, f.endDate, f.endMin, f.timeZone]).toEqual(["2026-12-08", 600, "2026-12-08", 630, "America/New_York"]);
    expect([f.summary, f.location, f.description]).toEqual([
      "Throughline / Camtek",
      "https://zoom.us/j/111",
      "https://zoom.us/j/111\n\nCall between Moshe Eisenberg (CFO, Camtek)",
    ]);
    expect(f.meeting).toBe(true);
    expect(f.calendarId).toBe(7);
    expect(f.attendees.map((a) => [a.name, a.email, !!a.optional])).toEqual([
      ["Moshe Eisenberg", "moshee@camtek.com", false],
      ["", "friend@gmail.com", true],
    ]);
  });

  it("keeps the blank form's time when no time was found", () => {
    const f = prefillForm(blank(), prepared({ found: false, start: null, end: null, timeZone: "Asia/Jerusalem" }));
    expect([f.startDate, f.startMin, f.endMin, f.timeZone]).toEqual(["2026-11-25", 17 * 60, 17 * 60 + 30, "Asia/Jerusalem"]);
    expect(f.summary).toBe("Throughline / Camtek");
    expect(f.attendees).toHaveLength(2);
  });
});

describe("clashText", () => {
  it("names one clash", () => {
    expect(clashText([{ eventId: 1, subject: "Dentist", when: "17:15–18:00" }])).toBe("You already have “Dentist” (17:15–18:00) at that time. Set up this meeting anyway?");
  });
  it("lists several, one per line", () => {
    expect(
      clashText([
        { eventId: 1, subject: "Dentist", when: "17:15–18:00" },
        { eventId: 2, subject: "Board", when: "17:00–19:00" },
      ]),
    ).toBe("You already have at that time:\n“Dentist” (17:15–18:00)\n“Board” (17:00–19:00)\nSet up this meeting anyway?");
  });
});
