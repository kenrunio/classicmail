//! The app's clock. The app uses the real time; the unit tests run on a fixed date — a Wednesday in
//! mid-October, late morning in the US and evening in Israel, so "today" is the same day in both —
//! plus the real time elapsed since the tests started, so timers still advance. The demo mailbox and
//! calendar are seeded relative to "now", so with a fixed now they are the same on every run (a
//! test can't start failing because today is a Monday, or the first of the month).

use chrono::{DateTime, Local, NaiveDate, Utc};

#[cfg(not(test))]
pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// Wednesday 2026-10-14 18:00 UTC (11:00 in California, 21:00 in Israel).
#[cfg(test)]
pub const TEST_START_MS: i64 = 1_792_000_800_000;

#[cfg(test)]
pub fn now() -> DateTime<Utc> {
    use std::{sync::OnceLock, time::Instant};
    static STARTED: OnceLock<Instant> = OnceLock::new();
    let elapsed = STARTED.get_or_init(Instant::now).elapsed();
    DateTime::from_timestamp_millis(TEST_START_MS).unwrap() + elapsed
}

pub fn now_ms() -> i64 {
    now().timestamp_millis()
}

/// Today's date on this computer.
pub fn today() -> NaiveDate {
    now().with_timezone(&Local).date_naive()
}

/// Now in this computer's time zone (e.g. the Date: header of a message being sent).
pub fn now_local() -> DateTime<Local> {
    now().with_timezone(&Local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_run_on_a_fixed_wednesday() {
        let d = DateTime::from_timestamp_millis(TEST_START_MS).unwrap();
        assert_eq!(d.to_rfc3339(), "2026-10-14T18:00:00+00:00");
        assert_eq!(d.format("%A").to_string(), "Wednesday");
        assert!(now_ms() >= TEST_START_MS && now_ms() < TEST_START_MS + 3_600_000);
    }
}
