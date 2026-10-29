import { describe, expect, it } from "vitest";
import type { EventDetail } from "../../api/calendar";
import { fmtLongDate } from "./dates";
import { itemMail, notesText, whenText } from "./itemMail";

const at = (h: number) => new Date(2026, 9, 14, h).getTime();

const meeting = (over: Partial<EventDetail> = {}): EventDetail =>
  ({
    id: 7,
    accountId: 1,
    calendarId: 1,
    calendarColor: "#000",
    summary: "Acme <> Investors",
    location: "Zoom",
    start: at(15),
    end: at(16),
    allDay: false,
    isRecurring: false,
    isMeeting: true,
    myResponse: "needsAction",
    busyStatus: "tentative",
    isPrivate: false,
    hasReminder: true,
    isException: false,
    importance: "normal",
    isOrganizer: false,
    readOnly: false,
    categories: [],
    description: "Agenda:\n1. Results <Q3> & more",
    attendees: [
      { email: "org@bank.com", name: "Organiser", response: "accepted", organizer: true, optional: false, isSelf: false },
      { email: "kenny@duvz.demo", name: "Kenny", response: "needsAction", organizer: false, optional: false, isSelf: true },
      { email: "a@fund.com", name: "Ann", response: "accepted", organizer: false, optional: false, isSelf: false },
      { email: "b@fund.com", name: "", response: "tentative", organizer: false, optional: true, isSelf: false },
      { email: "c_123@resource.calendar.google.com", name: "Room 4", response: "accepted", organizer: false, optional: false, isSelf: false },
    ],
    organizerEmail: "org@bank.com",
    organizerName: "Organiser",
    visibility: "default",
    calendarName: "Kenny",
    accountEmail: "Kenny@duvz.demo",
    isMaster: false,
    pending: false,
    ...over,
  }) as EventDetail;

describe("itemMail", () => {
  it("Reply goes to everyone but me and the room; Reply to One is the organizer", () => {
    const m = itemMail(meeting(), "reply");
    expect(m.to.map((a) => a.email)).toEqual(["org@bank.com", "a@fund.com", "b@fund.com"]);
    expect(m.subject).toBe("RE: Acme <> Investors");
    expect(m.targets?.one.to.map((a) => a.email)).toEqual(["org@bank.com"]);
    expect(m.targets?.all.to).toEqual(m.to);
  });

  it("Forward has no recipients and quotes the meeting, escaped", () => {
    const m = itemMail(meeting(), "forward");
    expect(m.to).toEqual([]);
    expect(m.targets).toBeUndefined();
    expect(m.subject).toBe("FW: Acme <> Investors");
    expect(m.quotedHtml).toContain('id="cm-original"');
    expect(m.quotedHtml).toContain("Acme &lt;&gt; Investors");
    expect(m.quotedHtml).toContain("<b>Where:</b> Zoom; Room 4");
    expect(m.quotedHtml).toContain("<b>Required Attendees:</b> Kenny; Ann");
    expect(m.quotedHtml).toContain("<b>Optional Attendees:</b> b@fund.com");
    // plain-text notes keep their "<" as text, never as markup
    expect(m.quotedHtml).toContain("Results &lt;Q3&gt; &amp; more");
  });

  it("my own meeting: Reply goes to the attendees, nothing to narrow it to", () => {
    const m = itemMail(meeting({ isOrganizer: true, organizerEmail: "kenny@duvz.demo" }), "reply");
    expect(m.to.map((a) => a.email)).toEqual(["org@bank.com", "a@fund.com", "b@fund.com"]);
    expect(m.targets?.one).toEqual(m.targets?.all);
  });
});

describe("notesText", () => {
  it("turns Google Calendar's HTML notes into text, leaves plain text alone", () => {
    expect(notesText("Line 1<br>Line <b>2</b>")).toBe("Line 1\nLine 2");
    expect(notesText("a < b and c > d")).toBe("a < b and c > d");
  });
  it("keeps a link's address when its words differ", () => {
    expect(notesText('<p>Call: <a href="https://zoom.us/j/1">Join Zoom</a></p><p><a href="https://x.com/a">https://x.com/a</a></p>')).toBe(
      "Call: Join Zoom <https://zoom.us/j/1>\nhttps://x.com/a",
    );
  });
  it("reads an Outlook series' converted notes", () => {
    const outlook = "<!-- Converted from text/plain format --><P><FONT SIZE=2>Meeting ID: 812 555 0147<BR><BR>Passcode: 7319<BR><BR>+15555550100,,8125550147# US<BR></FONT></P>";
    expect(notesText(outlook)).toBe("Meeting ID: 812 555 0147\n\nPasscode: 7319\n\n+15555550100,,8125550147# US");
  });
});

describe("whenText", () => {
  it("names the right last day of an all-day item ending on Israel's 23-hour day (27 March 2026)", () => {
    const start = new Date(2026, 2, 26).getTime();
    const end = new Date(2026, 2, 28).getTime();
    expect(whenText({ start, end, allDay: true })).toBe(`${fmtLongDate(start)} - ${fmtLongDate(new Date(2026, 2, 27))}`);
    expect(whenText({ start, end: new Date(2026, 2, 27).getTime(), allDay: true })).toBe(fmtLongDate(start));
  });
});
