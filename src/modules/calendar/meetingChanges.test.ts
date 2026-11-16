import { describe, expect, it } from "vitest";
import { meetingChanges, tellChoices, type MeetingShape } from "./meetingChanges";

const base: MeetingShape = {
  summary: "Q4 strategy",
  location: "Zoom",
  description: "Agenda",
  allDay: false,
  startDate: "2026-11-24",
  startMin: 17 * 60,
  endDate: "2026-11-24",
  endMin: 17 * 60 + 30,
  timeZone: "Asia/Jerusalem",
  recurrence: null,
  attendees: [{ email: "sarah@harbor.example" }, { email: "noa@orion.example" }],
};

describe("meetingChanges", () => {
  it("notes edited: something to tell everyone", () => {
    const c = meetingChanges(base, { ...base, description: "Agenda\nBudget" });
    expect(c).toEqual({ content: true, added: [], removed: [] });
    expect(tellChoices(c).map((o) => o.value)).toEqual(["all", "none"]);
  });

  it("only people added and removed: only them", () => {
    const c = meetingChanges(base, { ...base, attendees: [{ email: "Sarah@Harbor.example" }, { email: "dana@orbit.example" }] });
    expect(c).toEqual({ content: false, added: ["dana@orbit.example"], removed: ["noa@orion.example"] });
    expect(tellChoices(c).map((o) => o.value)).toEqual(["changes", "none"]);
  });

  it("both: everyone first, then only the people changed", () => {
    const c = meetingChanges(base, { ...base, endMin: 18 * 60, attendees: [{ email: "sarah@harbor.example" }] });
    expect(tellChoices(c).map((o) => o.value)).toEqual(["all", "changes", "none"]);
  });

  it("nothing attendees see (a reminder, a category; line endings): nothing to ask", () => {
    expect(tellChoices(meetingChanges({ ...base, description: "a\r\nb" }, { ...base, description: "a\nb" }))).toEqual([]);
    expect(tellChoices(meetingChanges(base, { ...base }))).toEqual([]);
  });
});
