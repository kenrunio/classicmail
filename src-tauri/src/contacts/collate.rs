//! Locale-aware ordering of contacts ("Sort By" in the views, `contacts.list`).
//!
//! A small ICU-root-like collation, matching what the front end gets from `Intl.Collator("en")`:
//! case- and accent-insensitive, Hebrew vowel points ignored and final letters equal to their
//! normal forms (ך = כ), and scripts in the order spaces/punctuation < digits < Latin < other
//! alphabets < Hebrew. The order is fixed (not the system locale) so Hebrew names always come
//! after the Latin ones, wherever ClassicMail runs.

use std::cmp::Ordering;

use super::model::Contact;

/// The sort orders offered in View › Sort By.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    #[default]
    FileAs,
    FirstName,
    LastName,
    Company,
    Added,
    Modified,
}

impl SortKey {
    pub fn parse(s: &str) -> SortKey {
        match s {
            "firstName" => SortKey::FirstName,
            "lastName" => SortKey::LastName,
            "company" => SortKey::Company,
            "added" => SortKey::Added,
            "modified" => SortKey::Modified,
            _ => SortKey::FileAs,
        }
    }
}

fn is_hebrew_mark(c: char) -> bool {
    matches!(c, '\u{0591}'..='\u{05BD}' | '\u{05BF}' | '\u{05C1}' | '\u{05C2}' | '\u{05C4}' | '\u{05C5}' | '\u{05C7}')
}

fn is_combining(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036F}') || is_hebrew_mark(c)
}

/// An accented Latin letter's base letter (é → e, ł → l); other characters as they are.
pub fn fold_latin(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' | 'ħ' => 'h',
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' | 'ŧ' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        other => other,
    }
}

fn fold_hebrew_final(c: char) -> char {
    match c {
        'ך' => 'כ',
        'ם' => 'מ',
        'ן' => 'נ',
        'ף' => 'פ',
        'ץ' => 'צ',
        other => other,
    }
}

/// Primary weight of one (already lower-cased and folded) character: (script group, char).
fn weight(c: char) -> (u8, u32) {
    let group = if c.is_whitespace() {
        0
    } else if c.is_ascii_digit() || c.is_numeric() {
        2
    } else if c.is_ascii_alphabetic() || (c.is_alphabetic() && (c as u32) < 0x0370) {
        3
    } else if ('\u{05D0}'..='\u{05EA}').contains(&c) {
        5
    } else if c.is_alphabetic() && (c as u32) < 0x0590 {
        4 // Greek, Cyrillic, Armenian: between Latin and Hebrew, as in ICU
    } else if c.is_alphabetic() {
        6
    } else {
        1 // punctuation and symbols
    };
    (group, c as u32)
}

/// The primary collation key of a string.
pub fn key(s: &str) -> Vec<(u8, u32)> {
    s.trim().chars().filter(|c| !is_combining(*c)).flat_map(char::to_lowercase).map(|c| weight(fold_hebrew_final(fold_latin(c)))).collect()
}

/// Compare two strings: primary key first, then the exact text so equal keys are stable.
pub fn compare(a: &str, b: &str) -> Ordering {
    key(a).cmp(&key(b)).then_with(|| a.cmp(b))
}

fn first_name(c: &Contact) -> &str {
    if c.given_name.is_empty() { &c.file_as } else { &c.given_name }
}

fn last_name(c: &Contact) -> &str {
    if c.family_name.is_empty() { &c.file_as } else { &c.family_name }
}

/// One text field as `compare` orders it: the collation key, then the exact text.
type Field = (Vec<(u8, u32)>, String);

fn field(s: &str) -> Field {
    (key(s), s.to_string())
}

/// Everything `sort` orders by, most significant first: a date (Added / Modified), the text
/// fields, then File As and the display name, and the id last so the order is total.
fn sort_key(c: &Contact, sort: SortKey) -> (i64, Vec<Field>, i64) {
    let (time, mut fields) = match sort {
        SortKey::FileAs => (0, vec![]),
        SortKey::FirstName => (0, vec![field(first_name(c)), field(last_name(c))]),
        SortKey::LastName => (0, vec![field(last_name(c)), field(first_name(c))]),
        SortKey::Company => (0, vec![field(&c.company)]),
        SortKey::Added => (c.first_seen_at, vec![]),
        SortKey::Modified => (c.updated, vec![]),
    };
    fields.push(field(&c.file_as));
    fields.push(field(&c.display_name));
    (time, fields, c.id)
}

/// Order of two contacts for `sort` (ascending; callers reverse for descending, except that
/// contacts without a company stay at the end).
pub fn compare_contacts(a: &Contact, b: &Contact, sort: SortKey) -> Ordering {
    sort_key(a, sort).cmp(&sort_key(b, sort))
}

/// Sort contacts in place. Contacts without a company sort last when sorting by company, in either
/// direction (otherwise thousands of blank rows would come first). Each contact's keys are built
/// once, not on every comparison.
pub fn sort_contacts(items: &mut [Contact], sort: SortKey, descending: bool) {
    let blank = |c: &Contact| sort == SortKey::Company && c.company.trim().is_empty();
    items.sort_by_cached_key(|c| (blank(c), sort_key(c, sort)));
    if descending {
        // the order is total (ids last), so descending = each part reversed, blanks still last
        let split = items.len() - items.iter().filter(|c| blank(c)).count();
        items[..split].reverse();
        items[split..].reverse();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut v: Vec<&str>) -> Vec<&str> {
        v.sort_by(|a, b| compare(a, b));
        v
    }

    #[test]
    fn latin_is_case_and_accent_insensitive() {
        assert_eq!(sorted(vec!["zeta", "Élodie", "adam", "Eli", "Bob"]), vec!["adam", "Bob", "Eli", "Élodie", "zeta"]);
        assert_eq!(key("Élodie"), key("elodie"));
    }

    #[test]
    fn hebrew_after_latin_and_digits_first() {
        assert_eq!(
            sorted(vec!["כהן, משה", "Cohen, Sarah", "3M", "אבוטבול, דנה", "zed", "ישראלי, יוסי"]),
            vec!["3M", "Cohen, Sarah", "zed", "אבוטבול, דנה", "ישראלי, יוסי", "כהן, משה"]
        );
    }

    #[test]
    fn hebrew_final_letters_and_points() {
        // final kaf = kaf, vowel points ignored
        assert_eq!(key("מלך"), key("מלכ"));
        assert_eq!(key("שָׁלוֹם"), key("שלום"));
        assert_eq!(sorted(vec!["תמר", "אורן", "גל", "בן"]), vec!["אורן", "בן", "גל", "תמר"]);
    }

    fn ct(id: i64, first: &str, last: &str, company: &str, seen: i64, updated: i64) -> Contact {
        let mut c = Contact { id, given_name: first.into(), family_name: last.into(), company: company.into(), ..Default::default() };
        c.normalize();
        c.first_seen_at = seen;
        c.updated = updated;
        c
    }

    #[test]
    fn sorts_by_each_key() {
        let all = vec![
            ct(1, "Sarah", "Cohen", "Harbor Capital", 300, 10),
            ct(2, "Adam", "Zimmer", "", 100, 30),
            ct(3, "משה", "כהן", "בנק לאומי", 200, 20),
            ct(4, "Noa", "Abramson", "Atlas Fund", 400, 40),
        ];
        let ids = |k: SortKey, desc: bool| {
            let mut v = all.clone();
            sort_contacts(&mut v, k, desc);
            v.iter().map(|c| c.id).collect::<Vec<_>>()
        };
        assert_eq!(ids(SortKey::FileAs, false), vec![4, 1, 2, 3]);
        assert_eq!(ids(SortKey::FileAs, true), vec![3, 2, 1, 4]);
        assert_eq!(ids(SortKey::FirstName, false), vec![2, 4, 1, 3]);
        assert_eq!(ids(SortKey::LastName, false), vec![4, 1, 2, 3]);
        assert_eq!(ids(SortKey::Company, false), vec![4, 1, 3, 2]);
        assert_eq!(ids(SortKey::Company, true), vec![3, 1, 4, 2]); // blank company still last
        assert_eq!(ids(SortKey::Added, false), vec![2, 3, 1, 4]);
        assert_eq!(ids(SortKey::Modified, true), vec![4, 2, 3, 1]);
    }
}
