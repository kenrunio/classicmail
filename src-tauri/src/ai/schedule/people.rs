//! The people on the meeting in order, and the app's own `{call link}` placeholder.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Someone on the meeting: To people are required, Cc people optional.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub name: String,
    pub email: String,
    pub title: Option<String>,
    pub company: Option<String>,
    pub optional: bool,
}

/// The first word of the name, else the address before the @.
pub(crate) fn first_name(p: &Person) -> String {
    match p.name.split_whitespace().next() {
        Some(w) => w.to_string(),
        None => p.email.split('@').next().unwrap_or("").to_string(),
    }
}

/// Company A–Z, then first name; people with no company known last.
pub(crate) fn sort_people(v: &mut [Person]) {
    v.sort_by_key(|p| {
        let company = p.company.as_deref().map(str::trim).filter(|c| !c.is_empty()).map(str::to_lowercase);
        (company.is_none(), company.unwrap_or_default(), first_name(p).to_lowercase(), p.email.to_lowercase())
    });
}

/// Stands for the call link while the AI fills the rest of the notes (it never sees the link).
pub(crate) const LINK_TOKEN: &str = "\u{2063}CALL-LINK\u{2063}";

static CALL_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\{\s*call\s+link\s*\}").unwrap());

/// The template with `{call link}` (any case, any spacing) as [`LINK_TOKEN`].
pub(crate) fn mark_link(template: &str) -> String {
    CALL_LINK.replace_all(template, LINK_TOKEN).into_owned()
}

/// The notes with the link in place of [`LINK_TOKEN`] (nothing when there's no link), and no blank
/// lines left at the start or end.
pub(crate) fn put_link(text: &str, link: Option<&str>) -> String {
    let s = text.replace(LINK_TOKEN, link.unwrap_or(""));
    s.trim_matches(|c: char| c == '\n' || c == '\r' || c == ' ').to_string()
}
