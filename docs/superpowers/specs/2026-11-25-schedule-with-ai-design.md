# Schedule with AI — design

Date: 2026-11-25 · Status: design, awaiting approval

## Why

Threads often end with "we can do Tuesday 8 Dec 10:00–10:30, can you send an invite?". Today that
means opening a new meeting, retyping the time (converting the other side's time zone), adding
everyone from the thread, pasting the Zoom link and writing the "Call between …" note by hand —
and remembering to check the calendar and that the Zoom room isn't already taken. Schedule with
AI does all of that in one press, and leaves Send to Kenny.

## Goals

- One button reads the selected email and opens a **new meeting window, filled in, not sent**:
  time, time zone, subject, attendees, location (the call link) and notes from a template.
- The calendar clash check and the call-link check are done by the app, the same way every time,
  never left to the AI.
- A notes template in Options › Calendar, with `{ }` parts the AI fills in.

## Non-goals

- No sending or booking without Kenny pressing Send in the meeting window.
- No list of "our clients": attendees are ordered by company A–Z, then first name.
- No creating Zoom meetings through Zoom's API: the links are fixed personal-room links Kenny
  types into Options.
- No proposing times when none is agreed (AI Review already does "suggest times").

## Anti-goals (a failure even if it works)

- Booking on top of something without telling Kenny.
- Putting a link in the invite that another meeting is using at the same time or right next to it.
- Getting the time wrong because the email's time was in the sender's zone.

## Approaches

- **A. The AI only reads; the app checks (chosen).** One AI call turns the email into plain facts
  (time, zone, subject, people with titles and companies) and fills the template's `{ }` parts;
  the app checks the calendar, picks the link, sorts people, opens the window. Predictable and
  testable; a bit more code.
- **B. The AI drives with calendar tools** (those AI Review uses). Less code, but the clash / link
  decisions would depend on the AI's judgement and could be silently wrong.

Kenny picked A.

## What Kenny sees

**Button.** Main mail toolbar: **Schedule with AI** next to Reply with AI; also in the Actions
menu, the message right-click menu and the message window's toolbar (where Reply with AI is).
Disabled without a single selected message, and when AI isn't set up (same tooltip as the other
AI buttons: "Set up AI in Tools › Options › AI").

**Press it.**
1. A busy indicator ("Reading the email…"); the AI call runs.
2. If no agreed date and time can be found: message box "Couldn't find a meeting time in this
   email." with **Open anyway** (meeting window with people and notes filled, time = next free
   slot as for New Meeting) / **Cancel**.
3. **Already invited?** If an event on that calendar (step 6) overlaps the time and has at least one of
   the email's people on it (organizer or guest), it's probably the same meeting: message box
   "Looks like you're already invited: *Subject* (10:00–10:30)." — **Open it** (opens that event)
   / **Set up a new one anyway** (continues to the clash check, without listing this event again)
   / **Cancel**. *(Claude's call.)*
4. **Clash check.** If Kenny has anything busy (same rule as `is_busy` in `ai/tools.rs`: timed,
   not shown as free; tentative and unanswered invitations count) overlapping the time on that
   calendar: message box
   "You already have *Subject* (10:00–11:00) at that time. Set up this meeting anyway?" — lists
   every clash — **Yes** / **No**. No → stop.
5. **Link check** (below) picks the link. (Also for the "Open anyway" slot of step 2.)
6. The meeting window opens as a new meeting on the **primary calendar of the account the email
   arrived in** (the account Reply would send from; that account's addresses are left out of the
   attendees). *(Claude's call.)* Invitations go out only when Kenny presses Send.
   If the time came from another zone, the window shows the time-zone row with that zone (as when
   opening a meeting in another zone), so Kenny sees both times. An info line at the top of the
   window says what was done, e.g. "Filled in by AI from Earnestine Boles's email. Time read as
   10:00 Eastern (17:00 Israel). Using the backup link: the main one is in *Weekly sync* until
   10:00." Kenny can edit everything before sending.

## Options › Calendar — new rows

- **Video call link:** text box (e.g. `https://zoom.us/j/81234567890?pwd=…`).
- **Backup video call link:** text box.
- **Meeting notes:** multi-line text box, the template. Default:
  `{call link}` ⏎⏎ `Call between {attendees with their positions and companies}.`
- A one-line hint under it: "Text in { } is filled in by AI. {call link} is the link chosen above."

Settings keys: `calendar.callLink`, `calendar.backupCallLink`, `calendar.meetingNotesTemplate`.

## Filling the meeting

**Attendees.** Everyone on the email (From, To, Cc) except Kenny's own addresses (all his
accounts' addresses and aliases, as Reply All excludes them). From and To go in To (required);
Cc go in Cc (optional) — the same split as the existing To/Cc boxes of the meeting window. People
named in the email body but not on it are **not** added. Order: by company A–Z (case-insensitive),
then first name; people with no company known go last, by first name.

**Company and title** for each attendee come from the AI (signatures in the thread, the
`Name (Company)` display-name form, the email domain as a last resort for the company). Unknown
title → left out of the note for that person, not invented.

**Subject.** The AI proposes a short subject from the thread, e.g. "Throughline Capital / Camtek",
with RE:/FW:/[EXTERNAL] stripped. Kenny edits it if he wants.

**Time.**
- The AI returns the agreed date, start, end (if said) and the IANA zone the time was written in
  (from the sender's signature, location, phone code, domain, or an explicit "ET"/"Israel time").
  The latest message in the thread wins over older proposals; a time Kenny's side proposed and
  the other side accepted counts as agreed.
- No end given → start + "Usual meeting length" (Options).
- A year-less date → the next such date on or after the email's date.
- Zone unknown → Kenny's own zone, and the info line says "Time zone not stated — read as Israel
  time."
- The app converts to an instant; the meeting is saved with the sender's zone as its zone when it
  differs (Google keeps the zone on the event).

**Location** = the chosen call link.

**Notes** = the template with:
1. `{call link}` replaced by the chosen link **by the app** (the AI never sees or writes the link);
2. every other `{ … }` filled in by the AI, using the attendees *in the app's order* (the app
   passes the sorted list with names, titles and companies to the AI). Same brace rules as AI
   Review (`instructions()` in `ai/compose.rs`).
Result is plain text in the notes box. If the template is empty, notes = the link.

## The link check

"Using" a link = an event whose location, notes or video-call details (`shown_conference`) contain
it. Links are compared normalised: lower-case host, no `www.`, no trailing `/`; for Zoom the
meeting number (`/j/<digits>`) is what's compared, so `pwd=` and different subdomains still match.

Events looked at: timed (all-day ignored), not cancelled, not declined by him.

**"His calendars"** — for the already-invited check, the clash check and the link check alike —
means **only the calendar the meeting is going into** (the organiser calendar, step 6). His other
accounts' calendars, shared, subscribed and holiday calendars are all ignored, even if a colleague
uses the same Zoom room. *(Kenny's decision.)*

The main link is **taken** if an event using it overlaps the new meeting, or ends within
"Gap between meetings" (Options, default 15 min) before it starts, or starts within that gap after
it ends. ("Straight before/after" = within the gap; with the gap set to 0, only touching events.)

- Main free → main.
- Main taken, backup set and free → backup; the info line says why.
- Both taken, or main taken and no backup → main, and the info line warns in red: "The video link
  is also used by *Subject* (09:30–10:00). Change it before sending."
- No main link set → no link; `{call link}` becomes empty; the info line says "No video link set
  (Tools › Options › Calendar)."

## Back end

`src-tauri/src/ai/schedule.rs`, rpc `ai.schedule.fromMessage { messageId }` → 

```
{
  found: bool,
  start: epoch ms | null, end: epoch ms | null, timeZone: string, zoneStated: bool,
  subject: string,
  attendees: [{ name, email, title?, company?, optional: bool }],   // sorted, me excluded
  calendarId: number,                                              // organiser calendar
  alreadyInvited: [{ eventId, subject, start, end }],
  clashes: [{ subject, start, end, calendar }],
  link: string | null, linkNote: string | null,   // why backup / warning
  notes: string,
}
```

Steps: `ensure_body` → `thread_context` (as Reply with AI) → one AI call that returns JSON
(time facts, subject, per-address title/company, and the filled template parts) → validate
(emails must be ones on the message; the zone must parse; date sane) → convert time → clash and
link checks against the calendar DB → sort → fill `{call link}`. Two AI calls are acceptable if
filling the template needs the sorted list first — builder's choice.

The front end shows the clash box and opens the meeting window with the result (new URL params on
the appointment window, or an id the window fetches the prepared result by — builder's choice).

## Testing (no real AI, Google or email)

- Rust, fake AI provider: JSON parsing and validation (unknown emails dropped, bad zone → mine),
  time conversion (ET → Israel across a DST edge), no-end → usual length, year-less date.
- Rust: link normalising (Zoom number with/without pwd, www, trailing slash); taken / adjacent
  within gap / gap 0 touching / declined and all-day ignored; main → backup → warn.
- Rust: clash detection (free / tentative / all-day).
- Vitest or Rust: attendee order (company A–Z, then first name, no-company last).
- Manual check in a dev slot with the demo account and the fake AI provider.

## Left to the builder

- Exact JSON prompt and whether it's one or two AI calls.
- How the prepared result reaches the appointment window.
- Icon for the button.
- Layout of the three new Options rows.
