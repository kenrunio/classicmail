//! Small shared helpers for SQL work.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};

pub fn now_ms() -> i64 {
    crate::clock::now_ms()
}

pub fn get_setting<T: DeserializeOwned>(c: &Connection, key: &str) -> anyhow::Result<Option<T>> {
    let raw: Option<String> = c.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0)).optional()?;
    Ok(match raw {
        Some(s) => Some(serde_json::from_str(&s)?),
        None => None,
    })
}

pub fn set_setting<T: Serialize>(c: &Connection, key: &str, value: &T) -> anyhow::Result<()> {
    c.execute(
        "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, serde_json::to_string(value)?],
    )?;
    Ok(())
}

pub fn get_sync_state(c: &Connection, account_id: i64, kind: &str) -> anyhow::Result<Option<String>> {
    Ok(c.query_row("SELECT value FROM sync_state WHERE account_id=?1 AND kind=?2", params![account_id, kind], |r| r.get(0)).optional()?)
}

pub fn set_sync_state(c: &Connection, account_id: i64, kind: &str, value: Option<&str>) -> anyhow::Result<()> {
    match value {
        Some(v) => c.execute(
            "INSERT INTO sync_state(account_id, kind, value) VALUES (?1, ?2, ?3) \
             ON CONFLICT(account_id, kind) DO UPDATE SET value=excluded.value",
            params![account_id, kind, v],
        )?,
        None => c.execute("DELETE FROM sync_state WHERE account_id=?1 AND kind=?2", params![account_id, kind])?,
    };
    Ok(())
}

/// Escape a user string for an FTS5 MATCH: each word becomes a quoted prefix term.
pub fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| c == '"' || c == '*'))
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() { None } else { Some(terms.join(" ")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_query_quotes_terms() {
        assert_eq!(fts_query("hello wor\"ld").unwrap(), "\"hello\"* \"wor\"\"ld\"*");
        assert!(fts_query("   ").is_none());
    }
}
