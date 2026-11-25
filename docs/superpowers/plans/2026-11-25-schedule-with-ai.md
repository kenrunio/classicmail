# Schedule with AI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Schedule with AI button that reads the selected email and opens a new, unsent meeting
filled in with the agreed time, the people, the call link and templated notes. The clash and link
checks are done by the app.

**Architecture:** The new Rust module `src-tauri/src/ai/schedule/` holds three pure parts and an
rpc, `ai.schedule.fromMessage`. The pure parts are links, people and time; each is unit-tested.
The rpc makes one AI call to read the email, then a second AI call only when the template has
`{ }` parts besides `{call link}`. Everything else (checks, sort, link) is done in Rust against the
calendar DB. The appointment window gets a `scheduleFrom=<messageId>` mode. It shows "Reading the
email with AI…", calls the rpc, asks the already-invited / clash questions in its own message
boxes, and fills its form. This matches how Reply with AI drafts inside the compose window.

**Tech Stack:** Rust (tokio, rusqlite, chrono / chrono-tz, url, regex, serde), React +
TypeScript, Vitest.

**Spec:** `docs/superpowers/specs/2026-11-25-schedule-with-ai-design.md`

## Global Constraints

- Nothing is sent or saved by the feature. The meeting window opens as a new unsaved meeting, and
  invitations go out only when Kenny presses Send.
- Calendar for the meeting **and for every check** = the primary calendar of the account the email
  arrived in (`calendars.account_id = src.account_id AND is_primary = 1`). No other calendar is
  ever read.
- Attendee order: company A–Z (case-insensitive), then first name. No company → last, by first
  name. There is no client list.
- The AI never sees or writes the call link. The app replaces `{call link}` (case-insensitive,
  spaces inside the braces allowed).
- Busy = `tools::is_busy` (timed, not transparent, not `show_as = free`). Events that are declined,
  cancelled, or `is_master = 1` are ignored (as in `tools::events`).
- "Straight before/after" = within `calendar.meetingGapMinutes` (default 15; 0 = touching only).
- No end time → start + `calendar.meetingMinutes` (default 45).
- Settings keys: `calendar.callLink`, `calendar.backupCallLink`, `calendar.meetingNotesTemplate`.
  The template defaults to `"{call link}\n\nCall between {attendees with their positions and companies}."`
- The button is disabled without a single selected message, or without AI set up. Its tooltip
  then reads "Set up AI in Tools › Options › AI".
- Tests never call real AI, Google or email. They use the fake provider (`ai/fake.rs`).
- Never `git push`. Commit each task to `main` locally.

## Review Focus

1. **The email has several proposed times** (they offered two, the other side picked one). The
   AI must return the one agreed in the newest message. This is covered by the prompt, plus a
   fake-provider test where the thread's newest line wins (Task 4).
2. **A recurring weekly meeting uses the same Zoom link.** Occurrences are `is_master = 0` rows,
   so the link check must look at occurrences, not the master. Task 1 tests a link found in
   `conference` text, and Task 4's fixture includes an occurrence row.
3. **A Zoom link pasted with a different `pwd=`, the `us02web.` subdomain or a trailing slash**
   must still count as the same room (Task 1).
4. **An email from someone whose company is unknown** (a gmail.com address, no signature). They
   go last, and the note leaves out their title and company instead of inventing one (Tasks 2
   and 4: the fake returns no company for gmail addresses).
5. **A date over a DST change** (US and Israel change on different weeks). The conversion uses
   the stated zone's rules for that date (Task 3: an America/New_York date on 2026-11-02, the
   week the US has changed and Israel has too, and 2026-03-16, when only the US has).

---

### Task 1: Call links — same-room matching and picking main / backup

**Files:**
- Create: `src-tauri/src/ai/schedule/links.rs`
- Create: `src-tauri/src/ai/schedule/mod.rs` (just `mod links; mod people; mod when;` for now, plus `#[cfg(test)] mod tests;`)
- Create: `src-tauri/src/ai/schedule/tests.rs`
- Modify: `src-tauri/src/ai/mod.rs` (`pub mod schedule;`)
- Modify: `src-tauri/src/ai/tools.rs`: make `is_busy`, `events`, `fmt_range` and `local_ms` `pub(crate)`

**Interfaces:**
- Produces:
  - `pub(crate) fn link_key(url: &str) -> Option<String>`: the room key. For a host ending in
    `zoom.us` with path `/j/<digits>`, the key is `"zoom:<digits>"`. Otherwise it is the lower-case
    host without `www.`, plus the path without a trailing `/`, with the query dropped. `None` if
    it isn't an http(s) URL.
  - `pub(crate) fn uses_link(ev: &EventRow, key: &str) -> bool`: any URL in
    `ev.location`, `ev.description` (after `conference::notes_text`) or `ev.conference` has that key.
  - `pub(crate) struct LinkPick { pub link: Option<String>, pub note: Option<String>, pub warn: bool }`
  - `pub(crate) fn pick_link(main: &str, backup: &str, events: &[EventRow], start: i64, end: i64, gap_min: i64, tz: &Tz) -> LinkPick`.
    `events` are timed, already filtered events of the organiser calendar around the slot. A link
    is taken by an event with `ev.end_ms > start - gap && ev.start_ms < end + gap` (gap in ms; with
    gap 0 an event ending exactly at `start` counts as touching, so use `>=` / `<=` for the gap
    edges).

- [ ] **Step 1: Write the failing tests** (in `schedule/tests.rs`, building `EventRow`s with `..Default::default()` if it derives Default, else a small helper `ev(summary, start, end, location, description, conference)`):

```rust
#[test]
fn link_key_matches_same_zoom_room() {
    let k = link_key("https://zoom.us/j/81234567890?pwd=abc").unwrap();
    assert_eq!(k, "zoom:81234567890");
    assert_eq!(link_key("https://us02web.zoom.us/j/81234567890?pwd=zzz#success").unwrap(), k);
    assert_eq!(link_key("https://www.example.com/room/").unwrap(), "example.com/room");
    assert_eq!(link_key("not a link"), None);
}

#[test]
fn uses_link_reads_location_notes_and_video_details() {
    let k = link_key("https://zoom.us/j/111").unwrap();
    assert!(uses_link(&ev("A", 0, 1, "https://zoom.us/j/111", "", ""), &k));
    assert!(uses_link(&ev("B", 0, 1, "", "<p>Join <a href=\"https://zoom.us/j/111?pwd=q\">here</a></p>", ""), &k));
    assert!(uses_link(&ev("C", 0, 1, "", "", "Join Zoom Meeting\nhttps://us06web.zoom.us/j/111"), &k));
    assert!(!uses_link(&ev("D", 0, 1, "https://zoom.us/j/222", "", ""), &k));
}

#[test]
fn pick_link_main_backup_warn_none() {
    let tz = chrono_tz::Asia::Jerusalem;
    let (s, e) = (h(10, 0), h(10, 30)); // helper: 2026-12-08 Israel time → ms
    let main = "https://zoom.us/j/111";
    let backup = "https://zoom.us/j/222";
    // free
    let p = pick_link(main, backup, &[], s, e, 15, &tz);
    assert_eq!((p.link.as_deref(), p.warn, p.note.is_none()), (Some(main), false, true));
    // main ends 10 minutes before (within gap 15) → backup, note names the meeting
    let busy = [ev("Weekly sync", h(9, 20), h(9, 50), main, "", "")];
    let p = pick_link(main, backup, &busy, s, e, 15, &tz);
    assert_eq!(p.link.as_deref(), Some(backup));
    assert_eq!(p.note.as_deref(), Some("Using the backup video link: the main one is in “Weekly sync” (09:20–09:50)."));
    assert!(!p.warn);
    // gap 0: 09:20–09:50 is not touching → main; 09:30–10:00 touches → backup
    assert_eq!(pick_link(main, backup, &busy, s, e, 0, &tz).link.as_deref(), Some(main));
    let touching = [ev("Prep", h(9, 30), h(10, 0), main, "", "")];
    assert_eq!(pick_link(main, backup, &touching, s, e, 0, &tz).link.as_deref(), Some(backup));
    // both taken → main + warning
    let both = [ev("Weekly sync", h(9, 30), h(10, 0), main, "", ""), ev("Other", h(10, 30), h(11, 0), backup, "", "")];
    let p = pick_link(main, backup, &both, s, e, 15, &tz);
    assert_eq!((p.link.as_deref(), p.warn), (Some(main), true));
    assert_eq!(p.note.as_deref(), Some("The video link is also used by “Weekly sync” (09:30–10:00). Change it before sending."));
    // main taken, no backup → main + warning
    assert!(pick_link(main, "", &busy, s, e, 15, &tz).warn);
    // no main link set
    let p = pick_link("", backup, &[], s, e, 15, &tz);
    assert_eq!(p.link, None);
    assert_eq!(p.note.as_deref(), Some("No video link set (Tools › Options › Calendar)."));
}
```

- [ ] **Step 2: Run them and check they fail**
Run: `cargo test --manifest-path src-tauri/Cargo.toml ai::schedule`
Expected: compile errors (`link_key` / `uses_link` / `pick_link` not found).

- [ ] **Step 3: Implement `links.rs`.** Use `url::Url` for parsing, and the same URL regex as
  `conference::LINK` to find URLs in text. `notes_text` turns description HTML into text first.
  Format times with `tools::fmt_range` (it must print `09:20–09:50`; if it prints something else,
  format with `HH:MM–HH:MM` in `tz` here instead). When a backup is used or a warning is given,
  the note names the **first** event (by start) that takes the main link.

- [ ] **Step 4: Run the tests and check they pass**
Run: `cargo test --manifest-path src-tauri/Cargo.toml ai::schedule`
Expected: PASS

- [ ] **Step 5: Commit**: `git add src-tauri/src/ai && git commit -m "Schedule with AI: call-link matching and main/backup choice"`

---

### Task 2: People order and the notes template

**Files:**
- Create: `src-tauri/src/ai/schedule/people.rs`
- Test: `src-tauri/src/ai/schedule/tests.rs`

**Interfaces:**
- Produces:
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] #[serde(rename_all = "camelCase")] pub struct Person { pub name: String, pub email: String, pub title: Option<String>, pub company: Option<String>, pub optional: bool }`
  - `pub(crate) fn first_name(p: &Person) -> String`: the first word of `name`, else the email's local part.
  - `pub(crate) fn sort_people(v: &mut [Person])`: company A–Z (case-insensitive; `None` or
    empty last), then first name (case-insensitive), then email.
  - `pub(crate) const LINK_TOKEN: &str = "\u{2063}CALL-LINK\u{2063}";`
  - `pub(crate) fn mark_link(template: &str) -> String`: replaces every `{ call link }` (regex `(?i)\{\s*call\s+link\s*\}`) with `LINK_TOKEN`.
  - `pub(crate) fn put_link(text: &str, link: Option<&str>) -> String`: replaces `LINK_TOKEN`
    with the link, or with nothing when there's no link. Then trims blank lines at the start and
    end of the text.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn people_sorted_by_company_then_first_name() {
    let p = |n: &str, e: &str, c: Option<&str>| Person { name: n.into(), email: e.into(), title: None, company: c.map(Into::into), optional: false };
    let mut v = vec![
        p("Kimberly Reitano", "kimberly.reitano@tlcap.com", Some("Throughline Capital")),
        p("", "someone@gmail.com", None),
        p("Moshe Eisenberg", "moshee@camtek.com", Some("Camtek")),
        p("Earnestine Boles", "earnestine.boles@tlcap.com", Some("throughline capital")),
        p("Ehud Helft", "ehud@ekgir.com", Some("EK Global IR")),
    ];
    sort_people(&mut v);
    let order: Vec<&str> = v.iter().map(|x| x.email.as_str()).collect();
    assert_eq!(order, ["moshee@camtek.com", "ehud@ekgir.com", "earnestine.boles@tlcap.com", "kimberly.reitano@tlcap.com", "someone@gmail.com"]);
    assert_eq!(first_name(&v[4]), "someone");
}

#[test]
fn call_link_placeholder_is_the_apps() {
    let t = mark_link("{Call Link}\n\nCall between {attendees}. { call  link }");
    assert!(!t.to_lowercase().contains("call link"));
    assert!(t.contains("{attendees}"));
    assert_eq!(put_link(&t, Some("https://zoom.us/j/1")), "https://zoom.us/j/1\n\nCall between {attendees}. https://zoom.us/j/1");
    assert_eq!(put_link(&mark_link("{call link}\n\nHi"), None), "Hi");
}
```

- [ ] **Step 2: Run them and check they fail**: `cargo test --manifest-path src-tauri/Cargo.toml ai::schedule`. Expected: compile errors.
- [ ] **Step 3: Implement `people.rs`** with the signatures above.
- [ ] **Step 4: Run them and check they pass**: same command. Expected: PASS.
- [ ] **Step 5: Commit**: `git commit -am "Schedule with AI: attendee order and call-link placeholder"` (add the new file first).

---

### Task 3: Turning the AI's time into a moment

**Files:**
- Create: `src-tauri/src/ai/schedule/when.rs`
- Test: `src-tauri/src/ai/schedule/tests.rs`

**Interfaces:**
- Produces:
  - `#[derive(Deserialize, Debug, Default)] #[serde(rename_all = "camelCase")] pub struct AiTime { pub date: Option<String> /* YYYY-MM-DD */, pub year_stated: bool, pub start: Option<String> /* HH:MM */, pub end: Option<String>, pub time_zone: Option<String> /* IANA */ }`
  - `pub(crate) struct Slot { pub start: i64, pub end: i64, pub tz: Tz, pub zone_stated: bool }`
  - `pub(crate) fn resolve(t: &AiTime, email_ms: i64, user_tz: Tz, length_min: i64) -> Option<Slot>`. It returns `None`
    when the date or start is missing or doesn't parse. An empty or unparseable zone means
    `user_tz` with `zone_stated = false`. When `!year_stated` and the date is before the email's
    date (in that zone), the year is moved on by one. A missing end, or an end not after the
    start, means start + `length_min`. Use `tools::local_ms` for the wall time → ms.
  - `pub(crate) fn zone_note(s: &Slot, user_tz: Tz) -> String`. When `s.tz == user_tz` and the
    zone was stated, the note is `""`. When the zone wasn't stated, it is
    `"Time zone not stated in the email — read as your time ({user zone name})."`. Otherwise it is
    `"Time read as {HH:MM} {zone name} ({HH:MM} your time)."`. The zone name is the IANA city with
    `_` replaced by a space (`New York`).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn resolves_their_zone_to_ours() {
    let il = chrono_tz::Asia::Jerusalem;
    let email = ms_utc(2026, 11, 25, 14, 45);
    let t = AiTime { date: Some("2026-12-08".into()), year_stated: false, start: Some("10:00".into()), end: Some("10:30".into()), time_zone: Some("America/New_York".into()) };
    let s = resolve(&t, email, il, 45).unwrap();
    assert_eq!(s.start, ms_utc(2026, 12, 8, 15, 0)); // 10:00 EST = 15:00 UTC = 17:00 Israel
    assert_eq!(s.end - s.start, 30 * 60_000);
    assert!(s.zone_stated);
    assert_eq!(zone_note(&s, il), "Time read as 10:00 New York (17:00 your time).");
}

#[test]
fn missing_end_unknown_zone_and_yearless_date() {
    let il = chrono_tz::Asia::Jerusalem;
    let email = ms_utc(2026, 12, 20, 9, 0);
    let t = AiTime { date: Some("2026-01-05".into()), year_stated: false, start: Some("09:00".into()), end: None, time_zone: Some("Mars/Base".into()) };
    let s = resolve(&t, email, il, 45).unwrap();
    assert_eq!(s.start, ms_utc(2027, 1, 5, 7, 0)); // next 5 Jan, Israel winter = UTC+2
    assert_eq!(s.end - s.start, 45 * 60_000);
    assert!(!s.zone_stated);
    assert_eq!(zone_note(&s, il), "Time zone not stated in the email — read as your time (Jerusalem).");
    assert!(resolve(&AiTime { start: Some("09:00".into()), ..Default::default() }, email, il, 45).is_none());
}

#[test]
fn dst_weeks_use_the_stated_zone() {
    let il = chrono_tz::Asia::Jerusalem;
    let t = |d: &str| AiTime { date: Some(d.into()), year_stated: true, start: Some("10:00".into()), end: None, time_zone: Some("America/New_York".into()) };
    // 16 Mar 2026: US already on EDT (UTC-4)
    assert_eq!(resolve(&t("2026-03-16"), 0, il, 30).unwrap().start, ms_utc(2026, 3, 16, 14, 0));
    // 2 Nov 2026: US back on EST (UTC-5)
    assert_eq!(resolve(&t("2026-11-02"), 0, il, 30).unwrap().start, ms_utc(2026, 11, 2, 15, 0));
}
```

- [ ] **Step 2: Run them and check they fail**: `cargo test --manifest-path src-tauri/Cargo.toml ai::schedule`. Expected: compile errors.
- [ ] **Step 3: Implement `when.rs`.**
- [ ] **Step 4: Run them and check they pass.** Expected: PASS.
- [ ] **Step 5: Commit**: `git commit -m "Schedule with AI: reading the agreed time in the sender's zone"` (add the new file first).

---

### Task 4: The rpc `ai.schedule.fromMessage`

**Files:**
- Modify: `src-tauri/src/ai/schedule/mod.rs` (rpc, prompts, orchestration)
- Modify: `src-tauri/src/ai/mod.rs` (`schedule::register(r);` next to `compose::register(r);`)
- Modify: `src-tauri/src/ai/fake.rs` (answers for `schedule.read` and `schedule.notes`)
- Test: `src-tauri/src/ai/schedule/tests.rs`

**Interfaces:**
- Consumes: Tasks 1–3. Also `compose::thread_context`, `compose::instructions(text, &[])`,
  `store::load_source`, `store::own_addresses`, `crate::mail::ensure_body`, `tools::{events,
  is_busy, user_tz, Env}`, and `get_setting` (as `tools::Env::load` uses it).
- Produces: rpc `ai.schedule.fromMessage { messageId: number }` → camelCase JSON:

```rust
pub struct Prepared {
    pub found: bool,
    pub calendar_id: i64,
    pub start: Option<i64>, pub end: Option<i64>,   // None when !found
    pub time_zone: String,                          // IANA, the zone to show the times in
    pub subject: String,
    pub attendees: Vec<Person>,                     // sorted; me (all own_addresses) excluded
    pub location: String,                           // the chosen link or ""
    pub notes: String,
    pub already_invited: Vec<Busy>,                 // overlap + shares a person with the email
    pub clashes: Vec<Busy>,                         // busy overlaps, minus already_invited
    pub info: Vec<InfoLine>,
}
pub struct Busy { pub event_id: i64, pub subject: String, pub when: String /* "09:30–10:00" */ }
pub struct InfoLine { pub icon: String /* "info" | "warning" */, pub text: String }
```

Steps inside the rpc, in order:
1. `ensure_body` (log and go on if it fails), then `load_source`. If the message is missing,
   fail with `user_err!("not_found", "The original message could not be found.")`.
2. Organiser calendar: `SELECT id FROM calendars WHERE account_id=?1 AND is_primary=1`. If there
   is none, fail with `user_err!("no_calendar", "This account has no calendar yet. Press Send/Receive and try again.")`.
3. AI call 1, `complete_json(SCHEDULE_READ_SYSTEM, {"task":"schedule.read","emailDate":"YYYY-MM-DD","people":[{name,email}…],"thread":thread_context(&src)})`.
   `people` is From + To + Cc, minus own addresses. Reply:
   `{found, date, yearStated, start, end, timeZone, subject, people:[{email,title,company}]}`.
   Title and company are attached only to emails that are in `people` (any others are dropped). From/To → `optional:false`,
   Cc → `optional:true` (someone in both → To).
4. `when::resolve`. `None` → `found:false`, no start/end, and the link check uses no slot (main
   link, no note).
5. Read the organiser calendar's events over `[start − gap − 1 day, end + gap + 1 day]` with
   `tools::events`. Then work out, in this order:
   - `already_invited`: the event overlaps `[start, end)` and its organizer or attendee emails
     (lower-cased) share one with `people`.
   - `clashes`: `is_busy` events overlapping the slot that aren't in `already_invited`.
   - The link, with `pick_link` over the timed events.
6. Notes: `mark_link(template)`. If `instructions(&marked, &[])` is not empty, make AI call 2:
   `{"task":"schedule.notes","template":marked,"attendees":[sorted people with title/company],"subject":…}`,
   reply `{"text": …}`. That system prompt says: fill every `{…}` in place, list attendees in
   the order given, leave out unknown titles/companies rather than invent, keep the
   `\u{2063}CALL-LINK\u{2063}` token exactly, plain text. If the reply lost the token, put the
   link at the top. Then `put_link`.
7. `info`, in this order:
   - `"Filled in by AI from {sender name}'s email."`
   - the zone note, if any
   - the link note (icon `warning` when `warn`)
   - when `!found`: `"Couldn't find an agreed meeting time in this email — set the time yourself."` (warning)

`SCHEDULE_READ_SYSTEM` states the spec's rules:
- the time agreed in the newest message wins; a time "we" proposed that "they" accepted counts as agreed
- the zone the time was written in comes from the signature, phone code, domain or a stated zone (IANA), and is empty if unknown
- `yearStated` is false when the email gives no year
- the subject is short ("Throughline Capital / Camtek"), without RE:/FW:/[EXTERNAL]
- titles and companies come only from the thread (signatures, "Name (Company)", the domain as a last resort for the company), never invented
- the answer is JSON only

- [ ] **Step 1: Add the fake answers** in `fake.rs` `compose_answer`:
  - `"schedule.read"`: if the thread contains `NO-TIME`, return `{"found":false}`. Otherwise, take the **first** match (the thread is newest
    first, so the newest wins) of `(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2})-(\d{2}:\d{2}) ([A-Za-z_/]+)`. Return
    `{found:true,date,yearStated:true,start,end,timeZone,subject:"Fake meeting",people:[…]}`, with
    `company` set to the capitalised second-level domain (`tlcap.com` → `Tlcap`), `null` for
    `gmail.com`, and `title:"Title of <local part>"`.
  - `"schedule.notes"`: every `{x}` replaced with the attendees' `"Name (Title, Company)"` joined by `", "`, leaving out the parts that are null.

- [ ] **Step 2: Write the failing tests.** Make a fixture like `ai/tools/tests.rs`'s `setup()`:
  - account `kenny@duvz.demo`, with primary calendar A (`owner`), a second owned calendar B on
    the same account (non-primary), and a second account with its own primary calendar
  - settings `calendar.callLink = https://zoom.us/j/111`, `calendar.backupCallLink = https://zoom.us/j/222`, gap 15
  - a message on account 1: From `Earnestine Boles <earnestine.boles@tlcap.com>`, To Kenny and
    `moshee@camtek.com`, Cc `friend@gmail.com`, Subject `RE: [EXTERNAL] Throughline Capital Call Request - CAMT`.
    Its text includes, newest first: `2026-12-08 10:00-10:30 America/New_York` and then
    `2026-12-09 11:00-11:30 America/New_York`. Insert it the way the existing `ai::compose` tests
    or `mail/compose` tests insert source messages (copy their helper).

```rust
#[tokio::test]
async fn prepares_the_meeting_from_the_email() {
    let (ctx, _g, msg) = setup().await;
    let r = call(&ctx, "ai.schedule.fromMessage", json!({ "messageId": msg })).await.unwrap();
    assert_eq!(r["found"], true);
    assert_eq!(r["calendarId"], CAL_A);
    assert_eq!(r["start"], ms_utc(2026, 12, 8, 15, 0)); // the newest line wins
    assert_eq!(r["timeZone"], "America/New_York");
    let emails: Vec<&str> = r["attendees"].as_array().unwrap().iter().map(|a| a["email"].as_str().unwrap()).collect();
    assert_eq!(emails, ["moshee@camtek.com", "earnestine.boles@tlcap.com", "friend@gmail.com"]); // Camtek, Tlcap, no company; Kenny left out
    assert_eq!(r["attendees"][2]["optional"], true);
    assert_eq!(r["location"], "https://zoom.us/j/111");
    let notes = r["notes"].as_str().unwrap();
    assert!(notes.starts_with("https://zoom.us/j/111\n\nCall between Moshe Eisenberg (Title of moshee, Camtek), "), "{notes}");
    assert!(!notes.contains("null") && !notes.contains("Title of friend, "), "{notes}"); // unknown company not invented
    assert_eq!(r["info"][1]["text"], "Time read as 10:00 New York (17:00 your time).");
}

#[tokio::test]
async fn only_the_organiser_calendar_is_checked() {
    // busy events at 17:00–17:30 Israel on calendar B and on the other account's calendar → no clashes,
    // and the main link used there at that time doesn't count
    let (ctx, _g, msg) = setup_with(|f| { f.event(CAL_B, "Elsewhere", il(8,17,0), il(8,17,30), "https://zoom.us/j/111"); f.event(CAL_OTHER, "Other acct", il(8,17,0), il(8,17,30), ""); }).await;
    let r = call(&ctx, "ai.schedule.fromMessage", json!({ "messageId": msg })).await.unwrap();
    assert_eq!(r["clashes"], json!([]));
    assert_eq!(r["location"], "https://zoom.us/j/111");
}

#[tokio::test]
async fn already_invited_clashes_and_backup_link() {
    let (ctx, _g, msg) = setup_with(|f| {
        f.event_with_attendee(CAL_A, "Throughline / Camtek", il(8,17,0), il(8,17,30), "earnestine.boles@tlcap.com");
        f.event(CAL_A, "Dentist", il(8,17,15), il(8,18,0), "");
        f.occurrence(CAL_A, "Weekly sync", il(8,16,20), il(8,16,50), "https://us02web.zoom.us/j/111?pwd=x"); // is_master=0, recurring
    }).await;
    let r = call(&ctx, "ai.schedule.fromMessage", json!({ "messageId": msg })).await.unwrap();
    assert_eq!(r["alreadyInvited"][0]["subject"], "Throughline / Camtek");
    let clashes: Vec<&str> = r["clashes"].as_array().unwrap().iter().map(|c| c["subject"].as_str().unwrap()).collect();
    assert_eq!(clashes, ["Dentist"]);
    assert_eq!(r["location"], "https://zoom.us/j/222");
}

#[tokio::test]
async fn no_time_found_still_prepares_people_and_notes() {
    let (ctx, _g, msg) = setup_with_text("Let's find a time. NO-TIME").await;
    let r = call(&ctx, "ai.schedule.fromMessage", json!({ "messageId": msg })).await.unwrap();
    assert_eq!(r["found"], false);
    assert_eq!(r["start"], Value::Null);
    assert!(r["notes"].as_str().unwrap().contains("Call between"));
    assert!(r["info"].as_array().unwrap().iter().any(|l| l["icon"] == "warning"));
}
```

- [ ] **Step 3: Run them and check they fail**: `cargo test --manifest-path src-tauri/Cargo.toml ai::schedule`. Expected: FAIL (method not found).
- [ ] **Step 4: Implement the rpc and the prompts** in `schedule/mod.rs`. Use `CompleteOpts` with temperature 0.1 and a 90 s timeout.
- [ ] **Step 5: Run them and check they pass**, then run `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`. Expected: PASS, no warnings.
- [ ] **Step 6: Commit**: `git commit -m "Schedule with AI: ai.schedule.fromMessage reads the email, checks the calendar and picks the link"`

---

### Task 5: Options › Calendar — call links and notes template

**Files:**
- Modify: `src/api/settings.ts`: add the three keys with defaults `""`, `""` and the template from Global Constraints
- Modify: `src/windows/OptionsWindow.tsx`: add rows to the Calendar section after "Gap between meetings"

**Interfaces:**
- Produces: the settings keys `calendar.callLink`, `calendar.backupCallLink` and
  `calendar.meetingNotesTemplate`. The back end reads them through `get_setting`; check that the
  setting saved from the front end is a JSON string, the same as other string settings.

- [ ] **Step 1: Add the keys and rows.**
  - `Video call link:` and `Backup video call link:` are full-width text inputs with placeholder `https://zoom.us/j/…`.
  - `Meeting notes:` is a `<textarea rows={4}>`.
  - A muted hint line under them: `Used by Schedule with AI. Text in { } is filled in by AI; {call link} is the video link above (or the backup when it's in use).`
- [ ] **Step 2: Check**: `npm run typecheck` and `npx eslint src/windows/OptionsWindow.tsx src/api/settings.ts`. Expected: no errors.
- [ ] **Step 3: Commit**: `git commit -am "Options › Calendar: video call links and meeting notes template"`

---

### Task 6: The appointment window's `scheduleFrom` mode

**Files:**
- Create: `src/api/aiSchedule.ts`: the `Prepared` / `Busy` / `InfoLine` / `Person` types, and `fromMessage(messageId)` calling the rpc
- Create: `src/modules/calendar/schedule.ts`: pure helpers
- Create: `src/modules/calendar/schedule.test.ts`
- Modify: `src/modules/calendar/windows/AppointmentWindow.tsx`: the new-item branch (around line 154) and `infoLines`

**Interfaces:**
- Consumes: the rpc `ai.schedule.fromMessage` (Task 4).
- Produces:
  - `export function prefillForm(base: Form, p: Prepared): Form` (in `schedule.ts`; export the
    `Form` type from AppointmentWindow, or move it into `schedule.ts`). It sets summary,
    location, description (= notes), `meeting: true`, `calendarId`, `timeZone`, and
    start/end via `wallOf(start, timeZone)`. Attendees are `Attendee`s built the same way
    `parseAddrs` builds them, with `optional`.
  - `export function clashText(c: Busy[]): string`, for example
    `You already have “Dentist” (17:15–18:00) at that time. Set up this meeting anyway?`. Several
    clashes are listed one per line before the question.

- [ ] **Step 1: Write the failing Vitest tests**

```ts
it("fills the new meeting from the prepared result", () => {
  const f = prefillForm(blankForm(), {
    found: true, calendarId: 7, start: Date.UTC(2026, 11, 8, 15), end: Date.UTC(2026, 11, 8, 15, 30),
    timeZone: "America/New_York", subject: "Throughline / Camtek", location: "https://zoom.us/j/111", notes: "https://zoom.us/j/111\n\nCall between …",
    attendees: [{ name: "Moshe Eisenberg", email: "moshee@camtek.com", optional: false }, { name: "", email: "friend@gmail.com", optional: true }],
    alreadyInvited: [], clashes: [], info: [],
  });
  expect([f.startDate, f.startMin, f.endMin, f.timeZone]).toEqual(["2026-12-08", 600, 630, "America/New_York"]);
  expect(f.meeting).toBe(true);
  expect(f.calendarId).toBe(7);
  expect(f.attendees.map((a) => [a.email, !!a.optional])).toEqual([["moshee@camtek.com", false], ["friend@gmail.com", true]]);
});
it("not found keeps the base time", () => { /* found:false → startDate/startMin unchanged from base */ });
it("clash text lists each clash", () => {
  expect(clashText([{ eventId: 1, subject: "Dentist", when: "17:15–18:00" }])).toBe("You already have “Dentist” (17:15–18:00) at that time. Set up this meeting anyway?");
});
```

- [ ] **Step 2: Run them and check they fail**: `npx vitest run src/modules/calendar/schedule.test.ts`. Expected: FAIL (module not found).
- [ ] **Step 3: Implement `schedule.ts` and `aiSchedule.ts`.**
- [ ] **Step 4: Wire up the window.** When `id === null` and `params.get("scheduleFrom")` is set:
  1. Build the blank form as now and show it, with the info line `Reading the email with AI…`.
  2. `fromMessage`. On error → `errorBox`, close the window. If `found` is false:
     `messageBox({ title: "Schedule with AI", icon: "warning", text: "Couldn't find a meeting time in this email.", buttons: ["Open anyway", "Cancel"] })`.
     **Cancel** → close. **Open anyway** → skip steps 3–4 and fill in, keeping the blank form's time.
  3. If `alreadyInvited` is not empty: `messageBox({ title: "Schedule with AI", icon: "help", text: "Looks like you're already invited: “<subject>” (<when>).", buttons: ["Open it", "Set up a new one anyway", "Cancel"] })`.
     - **Open it** → `openEvent` (the calendar action at `actions.ts:50`) for that id, then close this window.
     - **Cancel** / Esc → close.
  4. If `clashes` is not empty: `messageBox` with `clashText` and the buttons `Yes` / `No`. **No** → close.
  5. `setForm(prefillForm(f, p))`. Call `setShowZones(true)` when `p.timeZone !== localZone()`.
     Leave `initial` as the blank form, so closing asks whether to save, as for any edited new
     meeting. Show `p.info` as info lines (`icon` as given).
- [ ] **Step 5: Run the tests and checks**: `npx vitest run src/modules/calendar` and `npm run typecheck`. Expected: PASS.
- [ ] **Step 6: Commit**: `git commit -m "Appointment window: fill a new meeting from Schedule with AI"` (add new files first).

---

### Task 7: The buttons, the changelog and a manual check

**Files:**
- Modify: `src/modules/mail/actions.tsx`: `export function scheduleWithAi(id: number)` next to
  `replyWithAi`, plus a right-click / Actions item after "Reply with AI" (line ~281)
- Modify: `src/modules/mail/index.tsx`: a toolbar `TbButton` after Reply with AI (line ~66), and the Actions-menu item (line ~178)
- Modify: `src/modules/mail/windows/MessageWindow.tsx`: a `RibbonBig` after Reply with AI (line ~93)
- Modify: `src/ui/icons.tsx`: a `"schedule-ai"` icon (a calendar with the sparkle used by `reply-ai`)
- Modify: `CHANGELOG.md`: follow how the last feature commit (`b5f7031`) added its entry

**Interfaces:**
- Consumes: the window mode from Task 6.
- Produces: `scheduleWithAi(id)` = `openWindow("appointment", { scheduleFrom: id, meeting: 1 }, { title: "Untitled - Meeting" })`.

- [ ] **Step 1: Add the button and menu items.**
  - Label `Schedule with AI`.
  - Tooltip when ready: `Set up a meeting from this email: time, people, call link and notes`. Otherwise: `Set up AI in Tools › Options › AI`.
  - Disabled exactly like Reply with AI.
- [ ] **Step 2: Add the CHANGELOG entry.** Use plain words, the way the existing entries are written.
- [ ] **Step 3: Run the full checks**: `npm run check`. Expected: typecheck, eslint + clippy, prettier/rustfmt and vitest all pass. Then run `cargo test --manifest-path src-tauri/Cargo.toml`. Expected: PASS.
- [ ] **Step 4: Manual check** in a dev slot (`npm run dev:app`) with the demo account and the fake AI provider:
  1. Set both links in Options.
  2. On a demo message, press Schedule with AI. The window opens filled in: time, the zone row,
     attendees in order, Location, notes and info lines.
  3. Close it without sending.
  4. Take a screenshot (`scripts/shot.mjs`) and send it to Kenny.
- [ ] **Step 5: Commit**: `git commit -am "Schedule with AI button in the mail toolbar, menus and message window"`

Do not push. When all tasks are done, tell Kenny the stack is ready for him to review and push.
