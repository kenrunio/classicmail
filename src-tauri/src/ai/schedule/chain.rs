//! Who could be on the call: the whole conversation (every message of the thread, not only the
//! newest) and the people on your earlier meetings about the same thing, so the AI can find the
//! address of someone the emails only name ("Moshe is travelling").

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, params};

use super::people::Person;
use crate::{ai::compose::thread_context, google::calendar::Attendee, mail::compose::store};

/// The most of the conversation the AI is sent (characters).
const MAX_CHAIN: usize = 60_000;
/// Earlier messages of the thread read, newest first.
const MAX_OLDER: usize = 20;
/// People from earlier meetings offered to the AI.
const MAX_KNOWN: usize = 30;
const DAY: i64 = 86_400_000;

/// The thread's other messages, newest first.
pub(crate) fn older_messages(c: &Connection, src: &store::SourceMsg) -> anyhow::Result<Vec<store::SourceMsg>> {
    if src.thread_id.is_empty() {
        return Ok(vec![]);
    }
    let mut st = c.prepare(
        "SELECT id FROM messages WHERE account_id=?1 AND thread_id=?2 AND id!=?3 AND is_draft=0 \
         ORDER BY COALESCE(sent_date, internal_date) DESC LIMIT ?4",
    )?;
    let ids = st
        .query_map(params![src.account_id, src.thread_id, src.id, MAX_OLDER as i64], |r| r.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut out = vec![];
    for id in ids {
        if let Some(m) = store::load_source(c, id)? {
            out.push(m);
        }
    }
    Ok(out)
}

/// The conversation for the AI: the newest message, then each earlier one (headers and text).
pub(crate) fn chain_text(src: &store::SourceMsg, older: &[store::SourceMsg]) -> String {
    let mut s = thread_context(src);
    for m in older {
        if s.chars().count() >= MAX_CHAIN {
            break;
        }
        s.push_str("\n\n----- Earlier message in this conversation -----\n");
        s.push_str(&thread_context(m));
    }
    s.chars().take(MAX_CHAIN).collect()
}

/// Everyone on From / To / Cc of the earlier messages who isn't in `people` yet (or me).
pub(crate) fn add_thread_people(people: &mut Vec<Person>, older: &[store::SourceMsg], own: &HashSet<String>) {
    for m in older {
        for a in std::iter::once(&m.from).chain(m.to.iter()).chain(m.cc.iter()) {
            let email = a.email.trim();
            if email.is_empty() || own.contains(&email.to_lowercase()) || people.iter().any(|p| p.email.eq_ignore_ascii_case(email)) {
                continue;
            }
            people.push(Person { name: a.name.trim().to_string(), email: email.to_string(), title: None, company: None, optional: false });
        }
    }
}

const COMMON: &[&str] = &[
    "request",
    "conference",
    "call",
    "calls",
    "meeting",
    "meetings",
    "invitation",
    "invite",
    "with",
    "from",
    "about",
    "update",
    "follow",
    "external",
    "ltd",
    "inc",
    "corp",
    "group",
    "and",
    "the",
    "for",
    "your",
    "our",
    "zoom",
    "teams",
    "video",
    "online",
    "accepted",
    "declined",
    "tentative",
    "updated",
    "new",
    "time",
    "date",
    "fwd",
    "re",
    "fw",
];

/// The words of the subjects that can name a company or a deal ("Camtek", "CAMT").
pub(crate) fn subject_words<'a>(subjects: impl Iterator<Item = &'a str>) -> HashSet<String> {
    subjects
        .flat_map(|s| s.split(|c: char| !c.is_alphanumeric()))
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 3 && !w.chars().all(|c| c.is_ascii_digit()) && !COMMON.contains(&w.as_str()))
        .collect()
}

/// People on your meetings (last year to three months on) whose title shares a word with the
/// conversation's subject, most often met first; me and `exclude` left out.
pub(crate) fn known_people(
    c: &Connection,
    account_id: i64,
    words: &HashSet<String>,
    own: &HashSet<String>,
    exclude: &[Person],
    now: i64,
) -> anyhow::Result<Vec<Person>> {
    if words.is_empty() {
        return Ok(vec![]);
    }
    let mut st = c.prepare(
        "SELECT summary, attendees FROM events WHERE account_id=?1 AND status!='cancelled' AND is_master=0 \
         AND start_ms BETWEEN ?2 AND ?3 AND attendees IS NOT NULL AND attendees!='[]'",
    )?;
    let rows = st
        .query_map(params![account_id, now - 365 * DAY, now + 90 * DAY], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    // email (lower case) → (name, address, how many meetings)
    let mut seen: HashMap<String, (String, String, usize)> = HashMap::new();
    for (summary, attendees) in rows {
        let title_words = subject_words(std::iter::once(summary.as_str()));
        if title_words.is_disjoint(words) {
            continue;
        }
        let list: Vec<Attendee> = attendees.as_deref().and_then(|a| serde_json::from_str(a).ok()).unwrap_or_default();
        for a in list {
            let key = a.email.trim().to_lowercase();
            if key.is_empty()
                || a.is_self == Some(true)
                || a.resource == Some(true)
                || own.contains(&key)
                || exclude.iter().any(|p| p.email.eq_ignore_ascii_case(&key))
            {
                continue;
            }
            let e = seen.entry(key).or_insert_with(|| (String::new(), a.email.trim().to_string(), 0));
            e.2 += 1;
            if e.0.is_empty() {
                // Outlook can keep the quotes: "'Lior Aviram'"
                e.0 = a.display_name.as_deref().unwrap_or("").trim().trim_matches(|c| c == '\'' || c == '"').trim().to_string();
            }
        }
    }
    let mut v: Vec<(String, String, usize)> = seen.into_values().collect();
    v.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    Ok(v.into_iter().take(MAX_KNOWN).map(|(name, email, _)| Person { name, email, title: None, company: None, optional: false }).collect())
}
