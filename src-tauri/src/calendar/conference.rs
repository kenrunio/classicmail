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
static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://[^\s<>\]\[]+").unwrap());

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
