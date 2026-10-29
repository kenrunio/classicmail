//! "Resend Invitation" (the meeting's Responses window): a meeting request e-mail (iMIP, RFC 6047)
//! to just the attendees chosen. Google can only re-send to every guest; this sends the same
//! meeting — same UID, the current SEQUENCE, read fresh from Google — from the organiser's mailbox.
//! The attendee answers from their mail program as usual, and Google applies the reply.
//!
//! rpc: `calendar.resendInvite { id, emails }` → `{ sent, error }`: the addresses it went to, and
//! why it stopped if a send failed (the ones before it did go).

use std::sync::Arc;

use chrono::{DateTime, Datelike, Duration, NaiveDate, Offset, TimeZone, Utc};
use chrono_tz::{OffsetComponents, OffsetName, Tz};
use mail_builder::{
    MessageBuilder,
    headers::{content_type::ContentType, message_id::MessageId},
    mime::MimePart,
};
use serde::{Deserialize, Serialize};

use crate::{
    ctx::AppCtx,
    google::calendar::{self as gcal, Event, EventDateTime},
    mail::mime::escape_html,
};

/// RFC 5545 TEXT escaping.
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace("\r\n", "\n").replace('\n', "\\n")
}

/// A parameter value (CN=…): quoted when it holds `,` `;` or `:` (RFC 5545 has no backslash
/// escapes there, and a quoted value can't hold `"`).
fn param(s: &str) -> String {
    let s = s.replace('"', "'");
    if s.contains([',', ';', ':']) { format!("\"{s}\"") } else { s }
}

/// Lines longer than 75 octets continue on the next line after a space.
fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut n = 0;
    for ch in line.chars() {
        if n + ch.len_utf8() > 75 {
            out.push_str("\r\n ");
            n = 1;
        }
        out.push(ch);
        n += ch.len_utf8();
    }
    out
}

fn utc(t: DateTime<Utc>) -> String {
    t.format("%Y%m%dT%H%M%SZ").to_string()
}

fn instant(d: &EventDateTime) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(d.date_time.as_deref()?).ok().map(|t| t.with_timezone(&Utc))
}

fn offset(secs: i32) -> String {
    let (sign, a) = if secs < 0 { ('-', -secs) } else { ('+', secs) };
    format!("{sign}{:02}{:02}", a / 3600, (a % 3600) / 60)
}

/// The zone's rules for `years` years from `from_year` (a recurring meeting's times are local):
/// each change of offset as a STANDARD / DAYLIGHT onset, the later ones as RDATEs.
pub fn vtimezone(tz: Tz, from_year: i32, years: i32) -> String {
    let off = |t: DateTime<Utc>| tz.offset_from_utc_datetime(&t.naive_utc());
    let start = Utc.from_utc_datetime(&NaiveDate::from_ymd_opt(from_year, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap());
    let end = Utc.from_utc_datetime(&NaiveDate::from_ymd_opt(from_year + years, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap());
    // (local time it starts, offset before, offset after, name, daylight?)
    let mut onsets: Vec<(String, i32, i32, String, bool)> = vec![];
    let mut t = start;
    while t < end {
        let next = t + Duration::days(1);
        let (a, b) = (off(t), off(next));
        if a.fix() != b.fix() {
            // the minute it changes (in whole minutes since the epoch)
            let at = |m: i64| Utc.timestamp_opt(m * 60, 0).unwrap();
            let (mut lo, mut hi) = (t.timestamp() / 60, next.timestamp() / 60);
            while hi - lo > 1 {
                let mid = (lo + hi) / 2;
                if off(at(mid)).fix() == a.fix() { lo = mid } else { hi = mid }
            }
            let hi = at(hi);
            let from = a.fix().local_minus_utc();
            let local = (hi + Duration::seconds(from as i64)).format("%Y%m%dT%H%M%S").to_string();
            onsets.push((local, from, b.fix().local_minus_utc(), b.abbreviation().unwrap_or("").to_string(), !b.dst_offset().is_zero()));
        }
        t = next;
    }
    let mut out = format!("BEGIN:VTIMEZONE\r\nTZID:{}\r\n", tz.name());
    if onsets.is_empty() {
        let o = off(start);
        let s = offset(o.fix().local_minus_utc());
        out.push_str(&format!(
            "BEGIN:STANDARD\r\nDTSTART:19700101T000000\r\nTZOFFSETFROM:{s}\r\nTZOFFSETTO:{s}\r\nTZNAME:{}\r\nEND:STANDARD\r\n",
            o.abbreviation().unwrap_or("")
        ));
    }
    for daylight in [true, false] {
        let mine: Vec<_> = onsets.iter().filter(|o| o.4 == daylight).collect();
        let Some(first) = mine.first() else { continue };
        let kind = if daylight { "DAYLIGHT" } else { "STANDARD" };
        out.push_str(&format!(
            "BEGIN:{kind}\r\nDTSTART:{}\r\nTZOFFSETFROM:{}\r\nTZOFFSETTO:{}\r\nTZNAME:{}\r\n",
            first.0,
            offset(first.1),
            offset(first.2),
            first.3
        ));
        if mine.len() > 1 {
            out.push_str(&fold(&format!("RDATE:{}", mine[1..].iter().map(|o| o.0.as_str()).collect::<Vec<_>>().join(","))));
            out.push_str("\r\n");
        }
        out.push_str(&format!("END:{kind}\r\n"));
    }
    out.push_str("END:VTIMEZONE\r\n");
    out
}

/// The meeting's text: its notes (HTML ones as text) and its video-call details.
fn description(ev: &Event) -> String {
    let notes = ev.description.clone().unwrap_or_default();
    let notes = if notes.contains('<') { super::conference::notes_text(&notes) } else { notes };
    let call = super::conference::conference_text(ev.extra.get("conferenceData"), ev.hangout_link.as_deref());
    [notes.trim(), call.trim()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("\n\n")
}

/// The VCALENDAR (METHOD:REQUEST) for a Google event.
pub fn request_ics(ev: &Event, organizer: (&str, &str), now: DateTime<Utc>) -> anyhow::Result<String> {
    let uid = ev.ical_uid.clone().filter(|u| !u.is_empty()).ok_or_else(|| anyhow::anyhow!("the meeting has no iCalUID"))?;
    let (start, end) = (ev.start.as_ref(), ev.end.as_ref());
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "PRODID:-//Duvz Software//ClassicMail//EN".into(),
        "VERSION:2.0".into(),
        "CALSCALE:GREGORIAN".into(),
        "METHOD:REQUEST".into(),
    ];
    let recurring = ev.recurrence.as_ref().is_some_and(|r| !r.is_empty());
    let zone: Option<Tz> = start.and_then(|s| s.time_zone.as_deref()).and_then(|z| z.parse().ok()).filter(|_| recurring);
    let when = |d: Option<&EventDateTime>, name: &str| -> anyhow::Result<String> {
        let d = d.ok_or_else(|| anyhow::anyhow!("the meeting has no {name}"))?;
        if let Some(day) = &d.date {
            return Ok(format!("{name};VALUE=DATE:{}", day.replace('-', "")));
        }
        let t = instant(d).ok_or_else(|| anyhow::anyhow!("the meeting's {name} can't be read"))?;
        Ok(match zone {
            // a series keeps its local time over clock changes
            Some(z) => format!("{name};TZID={}:{}", z.name(), t.with_timezone(&z).format("%Y%m%dT%H%M%S")),
            None => format!("{name}:{}", utc(t)),
        })
    };
    if let (Some(z), Some(t)) = (zone, start.and_then(instant)) {
        // from the series' first year to ten years from now (a long-running series' rules must
        // still cover today)
        let from = t.with_timezone(&z).year();
        lines.push(vtimezone(z, from, (now.year() + 10 - from).max(12)).trim_end().to_string());
    }
    lines.push("BEGIN:VEVENT".into());
    lines.push(when(start, "DTSTART")?);
    lines.push(when(end, "DTEND")?);
    lines.extend(ev.recurrence.clone().unwrap_or_default());
    lines.push(format!("DTSTAMP:{}", utc(now)));
    lines.push(format!("ORGANIZER;CN={}:mailto:{}", param(organizer.1), organizer.0));
    lines.push(format!("UID:{uid}"));
    for a in ev.attendees.iter().flatten().filter(|a| a.resource != Some(true)) {
        let role = if a.optional == Some(true) { "OPT-PARTICIPANT" } else { "REQ-PARTICIPANT" };
        let part = match a.response_status.as_str() {
            "accepted" => "ACCEPTED",
            "declined" => "DECLINED",
            "tentative" => "TENTATIVE",
            _ => "NEEDS-ACTION",
        };
        let cn = a.display_name.as_deref().filter(|n| !n.is_empty()).unwrap_or(&a.email);
        lines.push(format!("ATTENDEE;CUTYPE=INDIVIDUAL;ROLE={role};PARTSTAT={part};RSVP=TRUE;CN={}:mailto:{}", param(cn), a.email));
    }
    lines.push(format!("SEQUENCE:{}", ev.sequence.unwrap_or(0)));
    lines.push(format!("SUMMARY:{}", esc(ev.summary.as_deref().unwrap_or(""))));
    if let Some(l) = ev.location.as_deref().filter(|l| !l.is_empty()) {
        lines.push(format!("LOCATION:{}", esc(l)));
    }
    let d = description(ev);
    if !d.is_empty() {
        lines.push(format!("DESCRIPTION:{}", esc(&d)));
    }
    lines.extend(["STATUS:CONFIRMED".into(), "TRANSP:OPAQUE".into(), "END:VEVENT".into(), "END:VCALENDAR".into()]);
    Ok(lines.iter().map(|l| if l.starts_with("BEGIN:VTIMEZONE") { l.clone() } else { fold(l) }).collect::<Vec<_>>().join("\r\n") + "\r\n")
}

/// The invitation e-mail to one attendee: a short text / HTML summary, and the request itself
/// (the text/calendar part mail programs show as an invitation, and invite.ics). `series`: how a
/// recurring meeting recurs, as the appointment window says it ("Occurs every Tuesday effective
/// 5/3/2024 from 17:00 to 17:30."), used for When instead of the series' first date.
pub fn invite_message(
    ev: &Event,
    organizer: (&str, &str),
    to: (&str, &str),
    tz: Tz,
    now: DateTime<Utc>,
    series: Option<&str>,
) -> anyhow::Result<Vec<u8>> {
    let ics = request_ics(ev, organizer, now)?;
    let summary = ev.summary.clone().unwrap_or_default();
    let recurring = ev.recurrence.as_ref().is_some_and(|r| !r.is_empty());
    let when = match (
        series.map(str::trim).filter(|t| recurring && !t.is_empty()),
        ev.start.as_ref().and_then(instant),
        ev.end.as_ref().and_then(instant),
    ) {
        (Some(t), _, _) => format!("{} ({})", t.trim_end_matches('.'), tz.name()),
        (None, Some(s), Some(e)) => {
            let (s, e) = (s.with_timezone(&tz), e.with_timezone(&tz));
            format!("{} - {} ({})", s.format("%A, %-d %B %Y %H:%M"), e.format("%H:%M"), tz.name())
        }
        _ => ev.start.as_ref().and_then(|s| s.date.clone()).unwrap_or_default(),
    };
    let place = ev.location.clone().unwrap_or_default();
    let notes = description(ev);
    let mut text = format!("{} has invited you to the following meeting.\n\n{summary}\nWhen: {when}\n", organizer.1);
    if !place.is_empty() {
        text.push_str(&format!("Where: {place}\n"));
    }
    if !notes.is_empty() {
        text.push_str(&format!("\n{notes}\n"));
    }
    text.push_str("\nPlease respond to this invitation in your calendar.\n");
    let html = format!(
        "<div style=\"font-family:Calibri,sans-serif;font-size:11pt\"><p>{} has invited you to the following meeting.</p><p><b>{}</b><br>When: {}{}</p>{}<p>Please respond to this invitation in your calendar.</p></div>",
        escape_html(organizer.1),
        escape_html(&summary),
        escape_html(&when),
        if place.is_empty() { String::new() } else { format!("<br>Where: {}", escape_html(&place)) },
        if notes.is_empty() { String::new() } else { format!("<p>{}</p>", escape_html(&notes).replace('\n', "<br>")) },
    );
    let calendar = || ContentType::new("text/calendar").attribute("method", "REQUEST").attribute("charset", "utf-8");
    let body = MimePart::new(
        "multipart/mixed",
        vec![
            MimePart::new(
                "multipart/alternative",
                vec![
                    MimePart::new("text/plain", text.replace('\n', "\r\n")),
                    MimePart::new("text/html", html),
                    MimePart::new(calendar(), ics.clone()),
                ],
            ),
            MimePart::new(ContentType::new("application/ics").attribute("name", "invite.ics"), ics).attachment("invite.ics"),
        ],
    );
    // "Invitation: Weekly call @ every Tuesday effective 5/3/2024 from 17:00 to 17:30 (Asia/Jerusalem)"
    let subject = format!("Invitation: {summary} @ {}", when.strip_prefix("Occurs ").unwrap_or(&when));
    let msg = MessageBuilder::new()
        .from((organizer.1.to_string(), organizer.0.to_string()))
        .to((to.1.to_string(), to.0.to_string()))
        .subject(subject)
        .message_id(MessageId::new(crate::mail::compose::mime::new_message_id(organizer.0)))
        .body(body);
    Ok(msg.write_to_vec()?)
}

#[derive(Deserialize)]
pub struct ResendParams {
    id: i64,
    emails: Vec<String>,
    /// how the series recurs, from the app (see `invite_message`)
    #[serde(default)]
    series: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct Resent {
    /// the addresses the invitation went to
    sent: Vec<String>,
    /// why it stopped, when a send failed (the ones in `sent` still went)
    error: Option<String>,
}

pub async fn resend_invite(ctx: Arc<AppCtx>, p: ResendParams) -> anyhow::Result<Resent> {
    let (row, cal, me) = ctx
        .db
        .read(move |c| {
            let row = super::load_row(c, p.id)?;
            let cal = super::model::get_cal(c, row.calendar_id)?;
            let me = super::account_identity(c, row.account_id)?;
            Ok((row, cal, me))
        })
        .await?;
    if !row.organizer_self {
        return Err(crate::user_err!("not_organizer", "Only the meeting's organizer can resend its invitation."));
    }
    // an occurrence of a series: the series' invitation
    let gid = row
        .recurring_event_id
        .clone()
        .or(row.gcal_id.clone())
        .ok_or_else(|| crate::user_err!("not_sent", "This meeting hasn't reached Google yet: send it first."))?;
    let api = ctx.api(row.account_id).await?;
    let ev = gcal::get_event(&api, &cal.gcal_id, &gid).await?;
    let organizer = ev.organizer.as_ref().and_then(|o| o.email.clone()).unwrap_or(me.0.clone());
    let now = crate::clock::now();
    let mut sent = vec![];
    for email in &p.emails {
        let Some(a) = ev.attendees.iter().flatten().find(|a| a.email.eq_ignore_ascii_case(email)) else { continue };
        let name = a.display_name.clone().unwrap_or_default();
        let raw = invite_message(&ev, (&organizer, &me.1), (&a.email, &name), cal.tz(), now, p.series.as_deref())?;
        if let Err(e) = api.gmail_send(&raw, None).await {
            // nothing sent yet: a plain failure; else say which ones went, so they aren't sent twice
            if sent.is_empty() {
                return Err(e);
            }
            log::warn!("resending the invitation of event {} stopped after {} attendee(s): {}", p.id, sent.len(), crate::logging::err(&e));
            return Ok(Resent { sent, error: Some(e.to_string()) });
        }
        sent.push(email.clone());
    }
    log::info!("resent the invitation of event {} to {} attendee(s)", p.id, sent.len());
    Ok(Resent { sent, error: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::calendar::{Attendee, Person};
    use mail_parser::MimeHeaders;

    fn meeting(recurring: bool) -> Event {
        Event {
            ical_uid: Some("abc123@google.com".into()),
            summary: Some("Orion - Post Results Update".into()),
            location: Some("https://zoom.us/j/8125550147".into()),
            description: Some("<p>Meeting ID: 812 555 0147<br>Passcode: 7319</p>".into()),
            start: Some(EventDateTime {
                date_time: Some("2026-11-24T17:00:00+02:00".into()),
                time_zone: Some("Asia/Jerusalem".into()),
                date: None,
            }),
            end: Some(EventDateTime {
                date_time: Some("2026-11-24T17:30:00+02:00".into()),
                time_zone: Some("Asia/Jerusalem".into()),
                date: None,
            }),
            recurrence: recurring.then(|| vec!["RRULE:FREQ=WEEKLY;BYDAY=TU".to_string()]),
            sequence: Some(3),
            organizer: Some(Person {
                email: Some("kenny@duvz.demo".into()),
                display_name: Some("Kenny Green".into()),
                is_self: Some(true),
            }),
            attendees: Some(vec![
                Attendee {
                    email: "noa.s@oriontech.example".into(),
                    display_name: Some("Noa Shapiro".into()),
                    response_status: "accepted".into(),
                    ..Default::default()
                },
                Attendee {
                    email: "yoav@oriontech.example".into(),
                    display_name: Some("Yoav Barak".into()),
                    response_status: "needsAction".into(),
                    optional: Some(true),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        }
    }

    #[test]
    fn a_request_for_the_same_meeting() {
        let now = Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap();
        let ics = request_ics(&meeting(false), ("kenny@duvz.demo", "Kenny Green"), now).unwrap();
        let unfolded = ics.replace("\r\n ", "");
        for want in [
            "METHOD:REQUEST",
            "UID:abc123@google.com",
            "SEQUENCE:3",
            "DTSTART:20261124T150000Z",
            "DTEND:20261124T153000Z",
            "ORGANIZER;CN=Kenny Green:mailto:kenny@duvz.demo",
            "ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED;RSVP=TRUE;CN=Noa Shapiro:mailto:noa.s@oriontech.example",
            "ROLE=OPT-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;CN=Yoav Barak:mailto:yoav@oriontech.example",
            "SUMMARY:Orion - Post Results Update",
            "DESCRIPTION:Meeting ID: 812 555 0147\\nPasscode: 7319",
        ] {
            assert!(unfolded.contains(want), "missing {want}:\n{ics}");
        }
        assert!(ics.lines().all(|l| l.len() <= 76), "folded at 75 octets");
        assert!(!ics.contains("VTIMEZONE"), "a single meeting goes in UTC");
    }

    #[test]
    fn names_with_commas_are_quoted() {
        let mut ev = meeting(false);
        ev.attendees.as_mut().unwrap()[0].display_name = Some("Shapiro, Noa".into());
        let now = Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap();
        let ics = request_ics(&ev, ("kenny@duvz.demo", "Green; Kenny"), now).unwrap().replace(
            "
 ", "",
        );
        assert!(ics.contains("CN=\"Shapiro, Noa\":mailto:noa.s@oriontech.example"), "{ics}");
        assert!(ics.contains("ORGANIZER;CN=\"Green; Kenny\":mailto:"), "{ics}");
    }

    #[test]
    fn html_notes_keep_links_as_words() {
        let mut ev = meeting(false);
        ev.description =
            Some(r#"<p>Agenda</p><p><a href="https://www.google.com/url?q=https://zoom.us/j/1&amp;sa=D">Join Zoom</a></p>"#.into());
        let d = description(&ev);
        assert!(d.contains("Join Zoom <https://zoom.us/j/1>"), "{d}");
        assert!(!d.contains("[1]") && !d.contains("google.com/url"), "{d}");
    }

    #[test]
    fn an_old_series_still_has_todays_rules() {
        let mut ev = meeting(true);
        ev.start.as_mut().unwrap().date_time = Some("2014-03-04T17:00:00+02:00".into());
        ev.end.as_mut().unwrap().date_time = Some("2014-03-04T17:30:00+02:00".into());
        let now = Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap();
        let ics = request_ics(&ev, ("kenny@duvz.demo", "Kenny Green"), now).unwrap().replace(
            "
 ", "",
        );
        assert!(ics.contains("20261025T020000"), "covers this year's clock change");
        assert!(ics.contains("RDATE") && ics.contains("2035"), "and the next ten years");
    }

    #[test]
    fn a_series_keeps_its_local_time() {
        let now = Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap();
        let ics = request_ics(&meeting(true), ("kenny@duvz.demo", "Kenny Green"), now).unwrap();
        assert!(ics.contains("DTSTART;TZID=Asia/Jerusalem:20261124T170000"), "{ics}");
        assert!(ics.contains("RRULE:FREQ=WEEKLY;BYDAY=TU"));
        assert!(ics.contains("BEGIN:VTIMEZONE\r\nTZID:Asia/Jerusalem"));
        // Israel's summer time starts on the Friday before the last Sunday of March, 02:00
        let tz = vtimezone(chrono_tz::Asia::Jerusalem, 2026, 2);
        assert!(tz.contains("BEGIN:DAYLIGHT\r\nDTSTART:20260327T020000\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0300"), "{tz}");
        assert!(tz.contains("BEGIN:STANDARD\r\nDTSTART:20261025T020000\r\nTZOFFSETFROM:+0300\r\nTZOFFSETTO:+0200"), "{tz}");
        assert!(tz.contains("RDATE:20270326T020000"), "{tz}");
    }

    #[test]
    fn a_series_invitation_says_how_it_recurs() {
        let now = Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap();
        let text = "Occurs every Tuesday effective 24/11/2026 from 17:00 to 17:30.";
        let raw = invite_message(
            &meeting(true),
            ("kenny@duvz.demo", "Kenny Green"),
            ("yoav@oriontech.example", "Yoav Barak"),
            chrono_tz::Asia::Jerusalem,
            now,
            Some(text),
        )
        .unwrap();
        let m = mail_parser::MessageParser::default().parse(&raw).unwrap();
        assert_eq!(
            m.subject(),
            Some("Invitation: Orion - Post Results Update @ every Tuesday effective 24/11/2026 from 17:00 to 17:30 (Asia/Jerusalem)")
        );
        assert!(m.body_text(0).unwrap().contains("When: Occurs every Tuesday effective 24/11/2026 from 17:00 to 17:30 (Asia/Jerusalem)"));
        // a single meeting keeps its date, whatever is passed
        let one = invite_message(
            &meeting(false),
            ("kenny@duvz.demo", "Kenny Green"),
            ("a@x.example", ""),
            chrono_tz::Asia::Jerusalem,
            now,
            Some(text),
        )
        .unwrap();
        let m = mail_parser::MessageParser::default().parse(&one).unwrap();
        assert!(m.subject().unwrap().ends_with("@ Tuesday, 24 November 2026 17:00 - 17:30 (Asia/Jerusalem)"));
    }

    #[test]
    fn the_email_carries_the_request() {
        let now = Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap();
        let raw = invite_message(
            &meeting(false),
            ("kenny@duvz.demo", "Kenny Green"),
            ("yoav@oriontech.example", "Yoav Barak"),
            chrono_tz::Asia::Jerusalem,
            now,
            None,
        )
        .unwrap();
        // read back as a mail program would
        let m = mail_parser::MessageParser::default().parse(&raw).unwrap();
        assert_eq!(m.subject(), Some("Invitation: Orion - Post Results Update @ Tuesday, 24 November 2026 17:00 - 17:30 (Asia/Jerusalem)"));
        assert_eq!(m.to().and_then(|a| a.first()).and_then(|a| a.address()), Some("yoav@oriontech.example"));
        let types: Vec<String> =
            m.parts.iter().filter_map(|p| p.content_type()).map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or(""))).collect();
        assert!(types.contains(&"text/calendar".to_string()) && types.contains(&"application/ics".to_string()), "{types:?}");
        let cal = m.parts.iter().find(|p| p.content_type().is_some_and(|c| c.subtype() == Some("calendar"))).unwrap();
        assert_eq!(cal.content_type().unwrap().attribute("method"), Some("REQUEST"));
        assert!(String::from_utf8_lossy(cal.contents()).contains("UID:abc123@google.com"));
    }
}
