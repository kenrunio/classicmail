//! Outlook 2007 "Color Categories": the master list (names, colours, shortcut keys) kept in the
//! settings table, and how an item's categories map onto Google Calendar:
//! `extendedProperties.private.cmCategories` = comma-separated names, `colorId` = the Google event
//! colour nearest to the first category's colour (so the colour shows on the web / phone too).
//!
//! The list is app-wide (not calendar-specific) so mail / contacts / tasks can reuse it later.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::util::{get_setting, set_setting};

pub const SETTING: &str = "categories.list";
pub const PROP: &str = "cmCategories";

/// Outlook 2007's 25 category colours: (preset, name, hex, nearest Google event colorId).
pub const PALETTE: &[(u8, &str, &str, &str)] = &[
    (1, "Red", "#e7a1a2", "11"),
    (2, "Orange", "#f9ba89", "6"),
    (3, "Peach", "#f7dd8f", "5"),
    (4, "Yellow", "#fcfa90", "5"),
    (5, "Green", "#78d168", "10"),
    (6, "Teal", "#9fdcc9", "2"),
    (7, "Olive", "#c6d2b0", "2"),
    (8, "Blue", "#9db7e8", "9"),
    (9, "Purple", "#b5a1e2", "3"),
    (10, "Maroon", "#daaec2", "4"),
    (11, "Steel", "#dad9dc", "8"),
    (12, "Dark Steel", "#6b7994", "8"),
    (13, "Gray", "#bfbfbf", "8"),
    (14, "Dark Gray", "#6f6f6f", "8"),
    (15, "Black", "#4f4f4f", "8"),
    (16, "Dark Red", "#c11a25", "11"),
    (17, "Dark Orange", "#e2620d", "6"),
    (18, "Dark Peach", "#c79930", "5"),
    (19, "Dark Yellow", "#b9b300", "5"),
    (20, "Dark Green", "#368f2b", "10"),
    (21, "Dark Teal", "#329b7a", "7"),
    (22, "Dark Olive", "#778b45", "2"),
    (23, "Dark Blue", "#2858a5", "9"),
    (24, "Dark Purple", "#5c3fa3", "3"),
    (25, "Dark Maroon", "#93446b", "4"),
];

/// Google Calendar's 11 event colours (Colors API `event` palette, background).
pub const GOOGLE_EVENT_COLORS: &[(&str, &str, &str)] = &[
    ("1", "Lavender", "#a4bdfc"),
    ("2", "Sage", "#7ae7bf"),
    ("3", "Grape", "#dbadff"),
    ("4", "Flamingo", "#ff887c"),
    ("5", "Banana", "#fbd75b"),
    ("6", "Tangerine", "#ffb878"),
    ("7", "Peacock", "#46d6db"),
    ("8", "Graphite", "#e1e1e1"),
    ("9", "Blueberry", "#5484ed"),
    ("10", "Basil", "#51b749"),
    ("11", "Tomato", "#dc2127"),
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub name: String,
    /// Outlook colour preset 1..=25, 0 = none
    pub color: u8,
    /// "F2".."F12" (used with Ctrl) or empty
    #[serde(default)]
    pub shortcut: String,
    /// colour calendar items whose subject contains the category's name (or a match word)
    #[serde(default = "default_true")]
    pub auto_match: bool,
    /// extra words that also match (e.g. "ACMX" for an "Acme" category)
    #[serde(default)]
    pub match_words: Vec<String>,
}

fn default_true() -> bool {
    true
}

pub fn defaults() -> Vec<Category> {
    [(1, "Red"), (2, "Orange"), (4, "Yellow"), (5, "Green"), (8, "Blue"), (9, "Purple")]
        .iter()
        .map(|(c, n)| Category { name: format!("{n} Category"), color: *c, shortcut: String::new(), auto_match: true, match_words: vec![] })
        .collect()
}

pub fn load(c: &Connection) -> anyhow::Result<Vec<Category>> {
    Ok(get_setting::<Vec<Category>>(c, SETTING)?.unwrap_or_else(defaults))
}

pub fn store(c: &Connection, list: &[Category]) -> anyhow::Result<()> {
    set_setting(c, SETTING, &list)
}

/// A category name as it can be stored in a comma-separated property.
pub fn clean_name(s: &str) -> String {
    s.replace([',', ';'], " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// "A, B" / "A;B" → ["A", "B"] (trimmed, no empties, no duplicates — case-insensitive).
pub fn parse_list(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in s.split([',', ';']) {
        let p = p.trim();
        if !p.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(p)) {
            out.push(p.to_string());
        }
    }
    out
}

pub fn join_list(v: &[String]) -> String {
    v.join(", ")
}

pub fn normalise(v: &[String]) -> Vec<String> {
    parse_list(&v.iter().map(|s| clean_name(s)).collect::<Vec<_>>().join(","))
}

/// The Google colorId for a category list: the first category with a colour decides.
pub fn color_id_for(cats: &[String], defs: &[Category]) -> Option<String> {
    cats.iter().find_map(|name| {
        let d = defs.iter().find(|d| d.name.eq_ignore_ascii_case(name))?;
        PALETTE.iter().find(|p| p.0 == d.color).map(|p| p.3.to_string())
    })
}

fn rgb(hex: &str) -> Option<(i32, i32, i32)> {
    let h = hex.trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let n = i32::from_str_radix(h, 16).ok()?;
    Some(((n >> 16) & 255, (n >> 8) & 255, n & 255))
}

/// Nearest Google event colorId for any hex colour (weighted RGB distance).
pub fn nearest_google_color(hex: &str) -> Option<String> {
    let (r, g, b) = rgb(hex)?;
    GOOGLE_EVENT_COLORS
        .iter()
        .filter_map(|(id, _, h)| {
            let (r2, g2, b2) = rgb(h)?;
            let rm = (r + r2) / 2;
            let (dr, dg, db) = (r - r2, g - g2, b - b2);
            // "redmean" approximation of perceived distance
            let d = (((512 + rm) * dr * dr) >> 8) + 4 * dg * dg + (((767 - rm) * db * db) >> 8);
            Some((d, *id))
        })
        .min_by_key(|x| x.0)
        .map(|x| x.1.to_string())
}

/// What colorId an item should carry after its categories change from `old` to `new`:
/// the new first category's colour; when the categories are cleared, a colour that only came
/// from the old first category is removed, a colour set in Google by hand is kept.
pub fn next_color_id(old: &[String], new: &[String], cur: Option<&str>, defs: &[Category]) -> Option<String> {
    if let Some(c) = color_id_for(new, defs) {
        return Some(c);
    }
    let from_old = color_id_for(old, defs);
    match cur {
        Some(c) if from_old.as_deref() == Some(c) => None,
        other => other.map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_parsing_and_colour_mapping() {
        assert_eq!(parse_list(" Red Category, Blue Category;red category,, "), vec!["Red Category", "Blue Category"]);
        assert_eq!(clean_name("  A, b ;c "), "A b c");
        let defs = defaults();
        assert_eq!(defs.len(), 6);
        assert_eq!(color_id_for(&["Unknown".into(), "Green Category".into()], &defs).as_deref(), Some("10"));
        assert_eq!(color_id_for(&["Purple Category".into()], &defs).as_deref(), Some("3"));
        assert_eq!(color_id_for(&[], &defs), None);
        // clearing removes a category colour but keeps a hand-picked Google colour
        assert_eq!(next_color_id(&["Red Category".into()], &[], Some("11"), &defs), None);
        assert_eq!(next_color_id(&["Red Category".into()], &[], Some("7"), &defs).as_deref(), Some("7"));
        assert_eq!(nearest_google_color("#dc2127").as_deref(), Some("11"));
        assert_eq!(nearest_google_color("#50b748").as_deref(), Some("10"));
        // every palette colour's mapped Google colour is a real one
        for p in PALETTE {
            assert!(GOOGLE_EVENT_COLORS.iter().any(|g| g.0 == p.3), "{p:?}");
        }
    }
}
