//! Schedule with AI (docs/superpowers/specs/2026-11-25-schedule-with-ai-design.md): a new meeting
//! filled in from an email — the agreed time, the people, the call link and the notes.
//!
//! `ai.schedule.fromMessage { messageId }` → [`Prepared`]. The AI only reads (the time, the zone it
//! was written in, each person's title and company) and fills the `{…}` parts of the notes
//! template; the calendar checks, the call link and the order of people are the app's. Only the
//! calendar the meeting goes into is looked at: the primary calendar of the account the email
//! arrived in.

pub(crate) mod chain;
pub(crate) mod links;
pub(crate) mod people;
pub(crate) mod when;

use std::{collections::HashSet, sync::Arc};

use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::json;

use self::{
    links::pick_link,
    people::{LINK_TOKEN, Person, mark_link, put_link, sort_people},
    when::{AiTime, resolve, zone_note},
};
use super::{
    CompleteOpts, complete_json,
    compose::instructions,
    tools::{events, fmt_time, is_busy, user_tz},
};
use crate::{calendar::model::EventRow, ctx::AppCtx, db::util::get_setting, mail::compose::store, rpc::Registry};

pub fn register(r: &mut Registry) {
    r.add("ai.schedule.fromMessage", from_message);
}

pub(crate) const DEFAULT_TEMPLATE: &str = "{call link}\n\nCall between {attendees with their positions and companies}.";
const DAY: i64 = 86_400_000;

const READ_SYSTEM: &str = "You read an e-mail conversation (newest message first) for the user (\"me\") and find the meeting \
the people on it have agreed to have. The time agreed in the newest message wins over earlier proposals; a time one side proposed \
and the other accepted counts as agreed. If no single date and start time has been agreed, set \"found\" to false. \
Give the date as YYYY-MM-DD (\"emailSent\" is when the newest message arrived, in the user's time zone; set \"yearStated\" to false when the e-mail gives no year), \
start and end as 24-hour HH:MM in the time zone the time was written in, and that zone as an IANA name (\"America/New_York\"), \
judged from an explicit zone (\"ET\", \"Israel time\"), the writer's signature, address, phone code or e-mail domain; \
leave \"timeZone\" empty if you can't tell. Leave \"end\" empty if no end or length was given. \
\"subject\" is a short meeting title from the conversation, e.g. \"Throughline Capital / Camtek\" (no RE:, FW:, [EXTERNAL]). \
Read the whole conversation, earlier messages included, to work out who the call is between and who takes part. \
\"attendees\" is everyone who should be invited (not me): the people on each side who take part, including someone the e-mails \
only name (\"Moshe is traveling\"), whose address you take from \"knownPeople\" (people on my earlier meetings about the same \
company or subject) by name and company. A colleague who is part of the conversation is a full attendee even if only copied; \
set \"optional\" to true only for someone plainly copied for information. Only use addresses from \"people\" and \"knownPeople\"; \
never make one up, and leave out a known person the conversation gives no reason to invite. \
\"between\" says in one line who the call is between, side by side, e.g. \"Moshe Eisenberg, CFO of Camtek, and Camtek IR with \
Fubon Securities (Melody Teng) for its investors\". \
For each address in \"people\" and each attendee, give the person's job title and company as written in the conversation (signatures, \
\"Name (Company)\" display names; for the company, the e-mail domain as a last resort, never a free mail domain like gmail.com). \
Never invent a title or company: use null when unknown. \
Answer with JSON only: {\"found\": true, \"date\": \"\", \"yearStated\": true, \"start\": \"\", \"end\": \"\", \"timeZone\": \"\", \
\"subject\": \"\", \"between\": \"\", \"attendees\": [{\"email\": \"\", \"optional\": false}], \
\"people\": [{\"email\": \"\", \"title\": null, \"company\": null}]}.";

const NOTES_SYSTEM: &str = "You fill in a meeting-notes template. Text inside curly braces {like this} is an instruction: \
replace each one, braces included, with the text it asks for, using the meeting's \"attendees\" (listed in the order to use), \"subject\" \
and \"between\" (who the call is between, when known: say it that way, side by side, with titles and companies). \
When listing people, keep the given order and give each one's title and company when known; leave out a title or company that is \
null rather than invent one. Keep everything outside the braces exactly as written, including the marker \u{2063}CALL-LINK\u{2063} \
and line breaks. Plain text, no markdown. Answer with JSON only: {\"text\": \"<the filled-in notes>\"}.";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Params {
    message_id: i64,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct ReadReply {
    // every field may come back null from a model: Option, never a hard error
    found: Option<bool>,
    #[serde(flatten)]
    time: AiTime,
    subject: Option<String>,
    /// who the call is between, in a line
    between: Option<String>,
    /// who to invite, chosen from the people offered
    attendees: Option<Vec<Pick>>,
    people: Option<Vec<PersonFacts>>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Pick {
    email: Option<String>,
    optional: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PersonFacts {
    email: Option<String>,
    title: Option<String>,
    company: Option<String>,
}

#[derive(Deserialize)]
struct NotesReply {
    text: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Busy {
    pub event_id: i64,
    pub subject: String,
    /// "09:30–10:00", in the user's time
    pub when: String,
}

#[derive(Serialize, Debug)]
pub struct InfoLine {
    /// "info" | "warning"
    pub icon: &'static str,
    pub text: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Prepared {
    pub found: bool,
    pub calendar_id: i64,
    pub start: Option<i64>,
    pub end: Option<i64>,
    /// IANA zone to show the times in
    pub time_zone: String,
    pub subject: String,
    pub attendees: Vec<Person>,
    pub location: String,
    pub notes: String,
    pub already_invited: Vec<Busy>,
    pub clashes: Vec<Busy>,
    pub info: Vec<InfoLine>,
}

/// When the email arrived, in the user's time zone: "2026-12-08 00:30 (Asia/Jerusalem)".
pub(crate) fn email_sent(ms: i64, tz: Tz) -> String {
    use chrono::TimeZone;
    tz.timestamp_millis_opt(ms).single().map(|d| format!("{} ({})", d.format("%Y-%m-%d %H:%M"), tz.name())).unwrap_or_default()
}

fn opts() -> CompleteOpts {
    CompleteOpts { temperature: Some(0.1), timeout: std::time::Duration::from_secs(90), ..Default::default() }
}

fn nonempty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && s != "null")
}

/// From and To are required, Cc optional (someone in both: required); my own addresses left out.
fn people_on(src: &store::SourceMsg, own: &HashSet<String>) -> Vec<Person> {
    let mut seen = HashSet::new();
    let mut out = vec![];
    let required = std::iter::once(&src.from).chain(src.to.iter()).map(|a| (a, false));
    for (a, optional) in required.chain(src.cc.iter().map(|a| (a, true))) {
        let email = a.email.trim().to_lowercase();
        if email.is_empty() || own.contains(&email) || !seen.insert(email) {
            continue;
        }
        out.push(Person { name: a.name.trim().to_string(), email: a.email.trim().to_string(), title: None, company: None, optional });
    }
    out
}

fn busy(tz: &Tz, e: &EventRow) -> Busy {
    Busy { event_id: e.id, subject: e.summary.clone(), when: format!("{}–{}", fmt_time(tz, e.start_ms), fmt_time(tz, e.end_ms)) }
}

/// Whether one of the email's people is the organizer or a guest of the meeting.
fn shares_someone(e: &EventRow, emails: &HashSet<String>) -> bool {
    e.organizer_email.iter().chain(e.attendees.iter().map(|a| &a.email)).any(|m| emails.contains(&m.trim().to_lowercase()))
}

async fn from_message(ctx: Arc<AppCtx>, p: Params) -> anyhow::Result<Prepared> {
    let id = p.message_id;
    if let Err(e) = crate::mail::ensure_body(&ctx, id).await {
        log::debug!("schedule: ensure_body({id}): {}", crate::logging::err(&e));
    }
    struct Loaded {
        src: store::SourceMsg,
        older: Vec<store::SourceMsg>,
        own: HashSet<String>,
        calendar_id: Option<i64>,
        user_tz: Tz,
        main: String,
        backup: String,
        template: String,
        gap: i64,
        length: i64,
    }
    let l = ctx
        .db
        .read(move |c| {
            let Some(src) = store::load_source(c, id)? else { return Ok(None) };
            let older = chain::older_messages(c, &src)?;
            let calendar_id = c
                .query_row("SELECT id FROM calendars WHERE account_id=?1 AND is_primary=1 ORDER BY id LIMIT 1", [src.account_id], |r| {
                    r.get(0)
                })
                .ok();
            Ok(Some(Loaded {
                older,
                own: store::own_addresses(c)?,
                calendar_id,
                user_tz: user_tz(c),
                main: get_setting(c, "calendar.callLink")?.unwrap_or_default(),
                backup: get_setting(c, "calendar.backupCallLink")?.unwrap_or_default(),
                template: get_setting(c, "calendar.meetingNotesTemplate")?.unwrap_or_else(|| DEFAULT_TEMPLATE.to_string()),
                gap: get_setting(c, "calendar.meetingGapMinutes")?.unwrap_or(15i64).clamp(0, 240),
                length: get_setting(c, "calendar.meetingMinutes")?.unwrap_or(45i64).clamp(5, 600),
                src,
            }))
        })
        .await?
        .ok_or_else(|| crate::user_err!("not_found", "The original message could not be found."))?;
    let calendar_id = l
        .calendar_id
        .ok_or_else(|| crate::user_err!("no_calendar", "This account has no calendar yet. Press Send/Receive and try again."))?;

    // 1. the AI reads the whole conversation and picks who is on the call: people on its emails, or
    // on my earlier meetings about the same company (someone the emails only name)
    let mut people = people_on(&l.src, &l.own);
    let mut offered = people.clone();
    chain::add_thread_people(&mut offered, &l.older, &l.own);
    let known = {
        let (acct, own, offered) = (l.src.account_id, l.own.clone(), offered.clone());
        let words = chain::subject_words(std::iter::once(l.src.subject.as_str()).chain(l.older.iter().map(|m| m.subject.as_str())));
        ctx.db.read(move |c| chain::known_people(c, acct, &words, &own, &offered, crate::db::util::now_ms())).await?
    };
    let brief = |v: &[Person]| v.iter().map(|p| json!({ "name": p.name, "email": p.email })).collect::<Vec<_>>();
    let prompt = json!({
        "task": "schedule.read",
        "emailSent": email_sent(l.src.date, l.user_tz),
        "people": brief(&offered),
        "knownPeople": brief(&known),
        "thread": chain::chain_text(&l.src, &l.older),
    })
    .to_string();
    let r: ReadReply = complete_json(&ctx, READ_SYSTEM, &prompt, opts()).await?;
    let mut picked: Vec<Person> = vec![];
    for a in r.attendees.unwrap_or_default() {
        let Some(email) = a.email.as_deref().map(str::trim) else { continue };
        if picked.iter().any(|p| p.email.eq_ignore_ascii_case(email)) {
            continue;
        }
        // only someone offered: never an address the AI made up
        if let Some(p) = offered.iter().chain(known.iter()).find(|p| p.email.eq_ignore_ascii_case(email)) {
            picked.push(Person { optional: a.optional == Some(true), ..p.clone() });
        }
    }
    // no choice from the AI: the newest email's From / To (required) and Cc (optional), as before
    if !picked.is_empty() {
        people = picked;
    }
    let between = nonempty(r.between).unwrap_or_default();
    for f in r.people.unwrap_or_default() {
        let Some(email) = f.email else { continue };
        if let Some(p) = people.iter_mut().find(|p| p.email.eq_ignore_ascii_case(email.trim())) {
            p.title = nonempty(f.title);
            p.company = nonempty(f.company);
        }
    }
    sort_people(&mut people);
    let slot = if r.found == Some(true) { resolve(&r.time, l.src.date, l.user_tz, l.length) } else { None };
    let subject = nonempty(r.subject).unwrap_or_else(|| l.src.subject.clone());

    // 2. the calendar: already invited, clashes, which call link
    let emails: HashSet<String> = people.iter().map(|p| p.email.to_lowercase()).collect();
    let (mut already_invited, mut clashes) = (vec![], vec![]);
    let link;
    let mut info = vec![InfoLine {
        icon: "info",
        text: format!(
            "Filled in by AI from {}'s email.",
            if l.src.from.name.trim().is_empty() { &l.src.from.email } else { &l.src.from.name }
        ),
    }];
    if let Some(s) = &slot {
        let (from, to, gap) = (s.start - l.gap * 60_000 - DAY, s.end + l.gap * 60_000 + DAY, l.gap);
        let evs = ctx.db.read(move |c| events(c, &[calendar_id], from, to)).await?;
        let overlapping = |e: &&EventRow| !e.all_day && e.end_ms > s.start && e.start_ms < s.end;
        for e in evs.iter().filter(overlapping) {
            if shares_someone(e, &emails) {
                already_invited.push(busy(&l.user_tz, e));
            } else if is_busy(e) {
                clashes.push(busy(&l.user_tz, e));
            }
        }
        let note = zone_note(s, l.user_tz);
        if !note.is_empty() {
            info.push(InfoLine { icon: "info", text: note });
        }
        let pick = pick_link(&l.main, &l.backup, &evs, s.start, s.end, gap, &l.user_tz);
        if let Some(n) = pick.note {
            info.push(InfoLine { icon: if pick.warn { "warning" } else { "info" }, text: n });
        }
        link = pick.link;
    } else {
        let pick = pick_link(&l.main, &l.backup, &[], 0, 0, 0, &l.user_tz);
        if let Some(n) = pick.note {
            info.push(InfoLine { icon: "info", text: n });
        }
        link = pick.link;
        info.push(InfoLine {
            icon: "warning", text: "Couldn't find an agreed meeting time in this email — set the time yourself.".into()
        });
    }

    // 3. the notes: the AI fills the {…} parts, the app puts in the link
    let template = if l.template.trim().is_empty() { "{call link}".to_string() } else { l.template.clone() };
    let mut marked = mark_link(&template);
    if !instructions(&marked, &[]).is_empty() {
        let prompt = json!({ "task": "schedule.notes", "template": marked, "attendees": people, "subject": subject, "between": between })
            .to_string();
        // a failed notes call keeps the meeting: the template stays, { } parts and all
        match complete_json::<NotesReply>(&ctx, NOTES_SYSTEM, &prompt, opts()).await {
            Ok(n) => {
                let had_token = marked.contains(LINK_TOKEN);
                marked = n.text;
                if had_token && !marked.contains(LINK_TOKEN) {
                    marked = format!("{LINK_TOKEN}\n\n{marked}");
                }
            }
            Err(e) => {
                log::debug!("schedule: notes: {}", crate::logging::err(&e));
                info.push(InfoLine {
                    icon: "warning", text: "AI couldn't fill in the notes — the parts in { } are left for you.".into()
                });
            }
        }
    }
    let notes = put_link(&marked, link.as_deref());

    Ok(Prepared {
        found: slot.is_some(),
        calendar_id,
        start: slot.as_ref().map(|s| s.start),
        end: slot.as_ref().map(|s| s.end),
        time_zone: slot.as_ref().map(|s| s.tz).unwrap_or(l.user_tz).name().to_string(),
        subject,
        attendees: people,
        location: link.unwrap_or_default(),
        notes,
        already_invited,
        clashes,
        info,
    })
}

#[cfg(test)]
mod tests;
