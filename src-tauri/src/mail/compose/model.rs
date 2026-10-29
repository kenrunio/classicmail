//! The compose model exchanged with the compose window (camelCase JSON).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Addr {
    #[serde(default)]
    pub name: String,
    pub email: String,
}

impl Addr {
    pub fn new(name: impl Into<String>, email: impl Into<String>) -> Self {
        Addr { name: name.into(), email: email.into() }
    }
    /// `Name <email>` (or just the address).
    pub fn display(&self) -> String {
        if self.name.trim().is_empty() || self.name.eq_ignore_ascii_case(&self.email) {
            self.email.clone()
        } else {
            format!("{} <{}>", self.name, self.email)
        }
    }
    /// Outlook's "Name [mailto:email]" form used in quoted headers.
    pub fn outlook_display(&self) -> String {
        if self.name.trim().is_empty() || self.name.eq_ignore_ascii_case(&self.email) {
            self.email.clone()
        } else {
            format!("{} [mailto:{}]", self.name, self.email)
        }
    }
}

/// One attachment of a compose. Exactly one source is used: `path` (local file), `attachmentId`
/// (local `attachments.id` of an existing message, for forwards) or `dataUrl`.
/// With `cid` set the part is an inline image referenced from the HTML as `cid:<cid>`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cid: Option<String>,
    /// file name to use (defaults to the path's file name / the original attachment's name)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Importance {
    Low,
    #[default]
    Normal,
    High,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    #[default]
    New,
    Reply,
    ReplyAll,
    Forward,
    Draft,
}

impl Mode {
    /// `messages.last_verb` value set on the source message after sending.
    pub fn verb(self) -> Option<&'static str> {
        match self {
            Mode::Reply => Some("replied"),
            Mode::ReplyAll => Some("repliedAll"),
            Mode::Forward => Some("forwarded"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Compose {
    /// sending account
    pub account_id: i64,
    /// send-as address (defaults to the account's address)
    pub from: Option<String>,
    pub to: Vec<Addr>,
    pub cc: Vec<Addr>,
    pub bcc: Vec<Addr>,
    pub subject: String,
    pub html: String,
    /// plain-text alternative; derived from `html` when absent
    pub text: Option<String>,
    /// Format Text › Plain Text: send text/plain only
    pub plain_text: bool,
    pub attachments: Vec<AttachmentSpec>,
    pub importance: Importance,
    pub request_read_receipt: bool,
    pub request_delivery_receipt: bool,
    /// "Do not deliver before" (epoch ms)
    pub defer_until: Option<i64>,
    /// "Have replies sent to"
    pub reply_to: Vec<Addr>,
    /// Follow Up › Flag for Recipients ("Follow up", "Call", ...)
    pub flag_for_recipients: Option<String>,
    pub mode: Mode,
    pub source_message_id: Option<i64>,
    /// Gmail draft id this compose was saved as / opened from
    pub draft_id: Option<String>,
}

impl Compose {
    pub fn all_recipients(&self) -> impl Iterator<Item = &Addr> {
        self.to.iter().chain(self.cc.iter()).chain(self.bcc.iter())
    }
}

/// Very small address check (what Outlook's "Check Names" would reject).
pub fn valid_email(s: &str) -> bool {
    let s = s.trim();
    let Some((local, domain)) = s.rsplit_once('@') else { return false };
    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !s.chars().any(|c| c.is_whitespace() || matches!(c, '<' | '>' | ',' | ';' | '"' | '(' | ')' | '[' | ']'))
}

/// Parse "Name <a@b>; c@d, \"Last, First\" <e@f>" into addresses (Outlook accepts ; and ,).
pub fn parse_address_list(s: &str) -> Vec<Addr> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut in_angle = false;
    for ch in s.chars() {
        match ch {
            '"' => {
                in_quote = !in_quote;
                cur.push(ch);
            }
            '<' if !in_quote => {
                in_angle = true;
                cur.push(ch);
            }
            '>' if !in_quote => {
                in_angle = false;
                cur.push(ch);
            }
            ';' | ',' if !in_quote && !in_angle => {
                if let Some(a) = parse_one(&cur) {
                    out.push(a);
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if let Some(a) = parse_one(&cur) {
        out.push(a);
    }
    out
}

fn parse_one(s: &str) -> Option<Addr> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let (Some(lt), Some(gt)) = (s.rfind('<'), s.rfind('>'))
        && lt < gt
    {
        let email = s[lt + 1..gt].trim().to_string();
        let name = s[..lt].trim().trim_matches('"').trim().to_string();
        return Some(Addr { name, email });
    }
    Some(Addr { name: String::new(), email: s.trim_matches('"').to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lists() {
        let v = parse_address_list("Sarah Cohen <sarah@x.com>; bob@y.com, \"Levi, David\" <d@z.co.il>;");
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], Addr::new("Sarah Cohen", "sarah@x.com"));
        assert_eq!(v[1], Addr::new("", "bob@y.com"));
        assert_eq!(v[2], Addr::new("Levi, David", "d@z.co.il"));
        assert!(valid_email("a.b@c.example"));
        assert!(!valid_email("a b@c.example"));
        assert!(!valid_email("abc"));
        assert!(!valid_email("a@b"));
    }
}
