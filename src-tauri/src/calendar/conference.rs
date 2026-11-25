//! A meeting's video-call details as text: Google keeps a Zoom / Meet / Teams add-on's link,
//! dial-in numbers and notes in `conferenceData`, not in the description, so an invitation from a
//! Google Calendar user would otherwise show no notes at all. Laid out like the text Google puts in
//! its invitation e-mails (what Outlook shows):
//!
//!   Join Zoom Meeting
//!   https://example.zoom.us/j/81234567890?pwd=… (ID: 81234567890, passcode: 135790)
//!
//!   Join by phone
//!   (GB) +44 20 3901 7895 (passcode: 135790)
//!
//! Kept apart from the notes (read-only, never written back to Google).

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

// compiled once: notes are converted for every synced meeting on every pull
static ANCHOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?is)<a\s[^>]*?href\s*=\s*["']([^"']*)["'][^>]*>(.*?)</a\s*>"#).unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
pub(crate) static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://[^\s<>\]\[]+").unwrap());

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("").trim()
}

/// Google wraps links it shows in calendar text as https://www.google.com/url?q=<the link>&…:
/// the link itself.
fn unredirect(link: &str) -> String {
    let Ok(u) = url::Url::parse(link) else { return link.to_string() };
    if matches!(u.host_str(), Some("www.google.com" | "google.com"))
        && u.path() == "/url"
        && let Some((_, q)) = u.query_pairs().find(|(k, _)| k == "q")
    {
        return q.into_owned();
    }
    link.to_string()
}

/// Calendar notes (HTML) as text: links as their words, Google's redirects unwrapped. Also the
/// meeting's own notes in a resent invitation (invite.rs).
pub(crate) fn notes_text(html: &str) -> String {
    // a link becomes its words (html2text would bracket it); "words <address>" when they differ
    let html = ANCHOR.replace_all(html, |c: &regex::Captures| {
        let href = c[1].replace("&amp;", "&");
        let words = TAG.replace_all(&c[2], "").trim().to_string();
        let shown = href.trim_start_matches("mailto:");
        if words.is_empty() || words == shown || unredirect(&href) == words {
            if words.is_empty() { shown.to_string() } else { words }
        } else {
            format!("{words} &lt;{}&gt;", unredirect(&href))
        }
    });
    let text = html2text::config::plain_no_decorate().no_link_wrapping().string_from_read(html.as_bytes(), 10_000).unwrap_or_default();
    let text = LINK.replace_all(&text, |c: &regex::Captures| unredirect(&c[0]));
    text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// "passcode: 135790" / "PIN: 1234 5678#" — whichever code the entry point has.
fn code(e: &Value) -> Option<String> {
    for (k, label) in [("passcode", "passcode"), ("password", "password"), ("pin", "PIN"), ("accessCode", "access code")] {
        let v = s(e, k);
        if !v.is_empty() {
            return Some(format!("{label}: {v}"));
        }
    }
    None
}

/// The text for `conferenceData` (and/or a bare Google Meet `hangoutLink`); "" when there's none.
pub fn conference_text(conf: Option<&Value>, hangout_link: Option<&str>) -> String {
    let mut blocks: Vec<String> = Vec::new();
    let entries: Vec<&Value> =
        conf.and_then(|c| c.get("entryPoints")).and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    let solution = conf.and_then(|c| c.get("conferenceSolution")).map(|c| s(c, "name")).filter(|n| !n.is_empty()).unwrap_or("the meeting");
    let of = |t: &'static str| entries.iter().copied().filter(move |e| s(e, "entryPointType") == t);

    for e in of("video") {
        let uri = s(e, "uri");
        let mut extra = vec![];
        let id = s(e, "meetingCode");
        if !id.is_empty() {
            extra.push(format!("ID: {id}"));
        }
        extra.extend(code(e));
        let head = if solution.starts_with("Google Meet") { "Join with Google Meet".to_string() } else { format!("Join {solution}") };
        let extra = if extra.is_empty() { String::new() } else { format!(" ({})", extra.join(", ")) };
        blocks.push(format!("{head}\n{uri}{extra}"));
    }
    let phones: Vec<String> = of("phone")
        .map(|e| {
            let number = if s(e, "label").is_empty() { s(e, "uri").trim_start_matches("tel:") } else { s(e, "label") };
            let region = s(e, "regionCode");
            let region = if region.is_empty() { String::new() } else { format!("({region}) ") };
            let code = code(e).map(|c| format!(" ({c})")).unwrap_or_default();
            format!("{region}{number}{code}")
        })
        .collect();
    if !phones.is_empty() {
        blocks.push(format!("Join by phone\n{}", phones.join("\n")));
    }
    let sips: Vec<String> = of("sip")
        .map(|e| {
            let addr = if s(e, "label").is_empty() { s(e, "uri").trim_start_matches("sip:") } else { s(e, "label") };
            format!("{addr}{}", code(e).map(|c| format!(" ({c})")).unwrap_or_default())
        })
        .collect();
    if !sips.is_empty() {
        blocks.push(format!("Join using SIP\n{}", sips.join("\n")));
    }
    for e in of("more") {
        let uri = s(e, "uri");
        if !uri.is_empty() {
            blocks.push(format!("Joining instructions\n{}", unredirect(uri)));
        }
    }
    let notes = conf.map(|c| s(c, "notes")).unwrap_or("");
    if !notes.is_empty() {
        blocks.push(notes_text(notes));
    }
    if blocks.is_empty()
        && let Some(link) = hangout_link.map(str::trim).filter(|l| !l.is_empty())
    {
        blocks.push(format!("Join with Google Meet\n{link}"));
    }
    blocks.join("\n\n")
}

// ---- one video link per meeting ----------------------------------------------------------------
// A meeting set up with a Zoom (or Teams) link in its location or notes often gets a Google Meet
// added by Google as well; two links confuse the people joining. Zoom wins, then Teams, then Meet.

/// The video service a link is for: 0 Zoom, 1 Teams, 2 Google Meet (lower is preferred).
fn video_rank(link: &str) -> Option<u8> {
    let host = url::Url::parse(link).ok()?.host_str()?.to_ascii_lowercase();
    let is = |d: &str| host == d || host.ends_with(&format!(".{d}"));
    if is("zoom.us") || is("zoomgov.com") {
        Some(0)
    } else if is("teams.microsoft.com") || is("teams.live.com") {
        Some(1)
    } else if is("meet.google.com") {
        Some(2)
    } else {
        None
    }
}

/// The best video service linked in this text (Google's redirects unwrapped).
fn best_in_text(text: &str) -> Option<u8> {
    LINK.find_iter(text).filter_map(|m| video_rank(&unredirect(m.as_str().trim_end_matches(['.', ',', ';', ')', '"', '\''])))).min()
}

/// The video service of Google's conference (add-on or Meet).
fn conference_rank(conf: Option<&Value>, hangout_link: Option<&str>) -> Option<u8> {
    let entries = conf.and_then(|c| c.get("entryPoints")).and_then(Value::as_array);
    let video = entries.into_iter().flatten().filter(|e| s(e, "entryPointType") == "video").filter_map(|e| video_rank(s(e, "uri"))).min();
    video.or_else(|| hangout_link.and_then(video_rank)).or_else(|| {
        // a conference whose video link we can't read: its service's name
        let name = conf.and_then(|c| c.get("conferenceSolution")).map(|c| s(c, "name").to_ascii_lowercase()).unwrap_or_default();
        if name.contains("zoom") {
            Some(0)
        } else if name.contains("teams") {
            Some(1)
        } else if name.contains("meet") {
            Some(2)
        } else {
            None
        }
    })
}

/// Google's conference is a second, less preferred link: the location or notes already link a
/// better service (a Zoom link typed in, and a Meet Google added by itself).
pub fn conference_is_extra(conf: Option<&Value>, hangout_link: Option<&str>, location: &str, description: &str) -> bool {
    let Some(theirs) = conference_rank(conf, hangout_link) else { return false };
    [location, description].iter().filter_map(|t| best_in_text(t)).min().is_some_and(|text| text < theirs)
}

/// `conference_text`, or "" when it is the extra link (`conference_is_extra`).
pub fn shown_conference(conf: Option<&Value>, hangout_link: Option<&str>, location: &str, description: &str) -> String {
    if conference_is_extra(conf, hangout_link, location, description) {
        return String::new();
    }
    conference_text(conf, hangout_link)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn one_video_link_zoom_then_teams_then_meet() {
        let meet = json!({ "conferenceSolution": { "name": "Google Meet" }, "entryPoints": [{ "entryPointType": "video", "uri": "https://meet.google.com/abc-defg-hij" }] });
        let hl = Some("https://meet.google.com/abc-defg-hij");
        // a Zoom link in the location: the Meet is the extra one
        assert!(conference_is_extra(Some(&meet), hl, "https://us02web.zoom.us/j/271019?pwd=x", ""));
        assert_eq!(shown_conference(Some(&meet), hl, "https://zoom.us/j/1", ""), "");
        // Teams in the notes (HTML, behind Google's redirect) also beats Meet
        let notes = r#"<a href="https://www.google.com/url?q=https://teams.microsoft.com/l/meetup-join/19%3ameeting&amp;sa=D">Join</a>"#;
        assert!(conference_is_extra(Some(&meet), hl, "", notes));
        // only Meet, or a Meet link in the text too: kept
        assert!(!conference_is_extra(Some(&meet), hl, "", ""));
        assert!(!conference_is_extra(Some(&meet), hl, "https://meet.google.com/abc-defg-hij", "Agenda"));
        assert!(!conference_is_extra(None, hl, "", "") && !conference_is_extra(None, None, "https://zoom.us/j/1", ""));
        // a bare hangoutLink counts as Meet
        assert!(conference_is_extra(None, hl, "Zoom: https://zoom.us/j/1.", ""));
        // a Zoom add-on beats a Teams link in the notes
        let zoom = json!({ "conferenceSolution": { "name": "Zoom Meeting" }, "entryPoints": [{ "entryPointType": "video", "uri": "https://example.zoom.us/j/8" }] });
        assert!(!conference_is_extra(Some(&zoom), None, "", "https://teams.microsoft.com/l/x"));
        // look-alike hosts aren't Zoom
        assert!(!conference_is_extra(Some(&meet), hl, "https://notzoom.us/j/1", ""));
    }

    #[test]
    fn zoom_from_a_google_calendar_invitation() {
        let conf = json!({
            "conferenceSolution": { "name": "Zoom Meeting", "key": { "type": "addOn" } },
            "conferenceId": "81234567890",
            "entryPoints": [
                { "entryPointType": "video", "uri": "https://example.zoom.us/j/81234567890?pwd=abc", "label": "example.zoom.us/j/81234567890?pwd=abc", "meetingCode": "81234567890", "passcode": "135790" },
                { "entryPointType": "phone", "uri": "tel:+442039017895,,81234567890#", "label": "+44 20 3901 7895", "regionCode": "GB", "passcode": "135790" },
                { "entryPointType": "phone", "uri": "tel:+13126266799,,81234567890#", "label": "+1 312-626-6799", "regionCode": "US", "passcode": "135790" },
                { "entryPointType": "sip", "uri": "sip:81234567890@zoomcrc.com", "label": "81234567890@zoomcrc.com", "passcode": "135790" },
                { "entryPointType": "more", "uri": "https://applications.zoom.us/addon/invitation/detail?meetingUuid=x" }
            ]
        });
        assert_eq!(
            conference_text(Some(&conf), None),
            "Join Zoom Meeting\nhttps://example.zoom.us/j/81234567890?pwd=abc (ID: 81234567890, passcode: 135790)\n\n\
             Join by phone\n(GB) +44 20 3901 7895 (passcode: 135790)\n(US) +1 312-626-6799 (passcode: 135790)\n\n\
             Join using SIP\n81234567890@zoomcrc.com (passcode: 135790)\n\n\
             Joining instructions\nhttps://applications.zoom.us/addon/invitation/detail?meetingUuid=x"
        );
    }

    #[test]
    fn zoom_notes_read_as_text_with_real_links() {
        let conf = json!({
            "conferenceSolution": { "name": "Zoom Meeting" },
            "entryPoints": [
                { "entryPointType": "more", "uri": "https://www.google.com/url?q=https://applications.zoom.us/addon/invitation/detail?meetingUuid%3DqWx%252B%253D%26v%3D1&sa=D&source=calendar&usg=AOv" }
            ],
            "notes": "Meeting host: <a href=\"mailto:host@acme.example\">host@acme.example</a><br /><br />Join Zoom Meeting: <br /><a href=\"https://www.google.com/url?q=https://example.zoom.us/j/841?pwd%3Dabc&amp;sa=D\">https://example.zoom.us/j/841?pwd=abc</a>"
        });
        assert_eq!(
            conference_text(Some(&conf), None),
            "Joining instructions\nhttps://applications.zoom.us/addon/invitation/detail?meetingUuid=qWx%2B%3D&v=1\n\n\
             Meeting host: host@acme.example\n\nJoin Zoom Meeting:\nhttps://example.zoom.us/j/841?pwd=abc"
        );
    }

    #[test]
    fn google_meet_and_nothing() {
        let conf = json!({
            "conferenceSolution": { "name": "Google Meet" },
            "entryPoints": [
                { "entryPointType": "video", "uri": "https://meet.google.com/abc-defg-hij", "label": "meet.google.com/abc-defg-hij" },
                { "entryPointType": "phone", "uri": "tel:+1-555-0100", "label": "+1 555-0100", "regionCode": "US", "pin": "123456789" }
            ]
        });
        assert_eq!(
            conference_text(Some(&conf), Some("https://meet.google.com/abc-defg-hij")),
            "Join with Google Meet\nhttps://meet.google.com/abc-defg-hij\n\nJoin by phone\n(US) +1 555-0100 (PIN: 123456789)"
        );
        assert_eq!(conference_text(None, Some("https://meet.google.com/xyz")), "Join with Google Meet\nhttps://meet.google.com/xyz");
        assert_eq!(conference_text(None, None), "");
    }
}
