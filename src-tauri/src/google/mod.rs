//! Google REST access. `client` is the authenticated HTTP layer; each API gets its own module of
//! thin, typed wrappers (owned by the matching domain).

pub mod auth;
pub mod calendar;
pub mod client;
pub mod gmail;
pub mod imap;
pub mod limits;
pub mod people;
pub mod tasks;

pub use client::Api;

/// An RFC 3339 time (Google's `updated`, `created`, `dateTime` …) as epoch ms.
pub fn rfc3339_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis())
}

/// Percent-encode a path segment (ids can contain `@`, `#`, etc. — calendar ids especially).
pub fn seg(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}
