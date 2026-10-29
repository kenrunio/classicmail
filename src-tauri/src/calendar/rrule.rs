//! A small RFC 5545 RRULE engine — the subset Outlook's recurrence dialog and Google produce:
//! FREQ=DAILY/WEEKLY/MONTHLY/YEARLY with INTERVAL, COUNT, UNTIL, BYDAY (incl. 2TU / -1FR),
//! BYMONTHDAY, BYMONTH, BYSETPOS, WKST, plus EXDATE / RDATE lines.
//! Expansion works on wall-clock times in the event's own time zone (so a weekly 9:00 meeting
//! stays at 9:00 across DST); callers convert to UTC with `chrono-tz`.
//! Used by the fake Google server (singleEvents expansion) and for local placeholders.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Weekday};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub freq: Freq,
    pub interval: u32,
    pub count: Option<u32>,
    /// inclusive, wall-clock in the event zone (UTC "Z" values are converted by the caller via `until_utc`)
    pub until: Option<NaiveDateTime>,
    /// true when UNTIL was given in UTC (…Z) — compare against UTC instants
    pub until_is_utc: bool,
    pub byday: Vec<(Option<i32>, Weekday)>,
    pub bymonthday: Vec<i32>,
    pub bymonth: Vec<u32>,
    pub bysetpos: Vec<i32>,
    pub wkst: Weekday,
}

fn parse_wd(s: &str) -> Option<Weekday> {
    Some(match s {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

pub fn wd_code(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "MO",
        Weekday::Tue => "TU",
        Weekday::Wed => "WE",
        Weekday::Thu => "TH",
        Weekday::Fri => "FR",
        Weekday::Sat => "SA",
        Weekday::Sun => "SU",
    }
}

/// Parse an iCalendar date or date-time value ("20261012", "20261012T090000", "20261012T090000Z").
/// Returns (wall time, is_utc).
pub fn parse_ical_dt(v: &str) -> Option<(NaiveDateTime, bool)> {
    let v = v.trim();
    if v.len() == 8 {
        let d = NaiveDate::parse_from_str(v, "%Y%m%d").ok()?;
        return Some((d.and_hms_opt(0, 0, 0)?, false));
    }
    let utc = v.ends_with('Z');
    let core = v.trim_end_matches('Z');
    let dt = NaiveDateTime::parse_from_str(core, "%Y%m%dT%H%M%S").ok()?;
    Some((dt, utc))
}

impl Rule {
    /// Parse "RRULE:FREQ=WEEKLY;BYDAY=MO,WE" (the "RRULE:" prefix is optional).
    pub fn parse(line: &str) -> anyhow::Result<Rule> {
        let body = line.trim().strip_prefix("RRULE:").unwrap_or(line.trim());
        let mut r = Rule {
            freq: Freq::Daily,
            interval: 1,
            count: None,
            until: None,
            until_is_utc: false,
            byday: vec![],
            bymonthday: vec![],
            bymonth: vec![],
            bysetpos: vec![],
            wkst: Weekday::Mon,
        };
        let mut have_freq = false;
        for part in body.split(';').filter(|p| !p.is_empty()) {
            let (k, v) = part.split_once('=').ok_or_else(|| anyhow::anyhow!("bad RRULE part {part}"))?;
            match k.to_ascii_uppercase().as_str() {
                "FREQ" => {
                    have_freq = true;
                    r.freq = match v {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        "MONTHLY" => Freq::Monthly,
                        "YEARLY" => Freq::Yearly,
                        _ => anyhow::bail!("unsupported FREQ {v}"),
                    }
                }
                "INTERVAL" => r.interval = v.parse::<u32>()?.max(1),
                "COUNT" => r.count = Some(v.parse()?),
                "UNTIL" => {
                    let (dt, utc) = parse_ical_dt(v).ok_or_else(|| anyhow::anyhow!("bad UNTIL {v}"))?;
                    // a DATE until means "through the end of that day"
                    let dt = if v.len() == 8 { dt + Duration::seconds(86_399) } else { dt };
                    r.until = Some(dt);
                    r.until_is_utc = utc;
                }
                "BYDAY" => {
                    for d in v.split(',') {
                        let d = d.trim();
                        if d.len() < 2 {
                            anyhow::bail!("bad BYDAY {d}");
                        }
                        let (num, code) = d.split_at(d.len() - 2);
                        let wd = parse_wd(code).ok_or_else(|| anyhow::anyhow!("bad BYDAY {d}"))?;
                        let n = if num.is_empty() { None } else { Some(num.trim_start_matches('+').parse::<i32>()?) };
                        r.byday.push((n, wd));
                    }
                }
                "BYMONTHDAY" => r.bymonthday = v.split(',').map(|x| x.trim().parse()).collect::<Result<_, _>>()?,
                "BYMONTH" => r.bymonth = v.split(',').map(|x| x.trim().parse()).collect::<Result<_, _>>()?,
                "BYSETPOS" => r.bysetpos = v.split(',').map(|x| x.trim().parse()).collect::<Result<_, _>>()?,
                "WKST" => r.wkst = parse_wd(v).unwrap_or(Weekday::Mon),
                _ => {} // ignore unknown parts (BYHOUR etc. are not produced by Outlook/Google UI)
            }
        }
        if !have_freq {
            anyhow::bail!("RRULE without FREQ");
        }
        Ok(r)
    }

    pub fn to_string(&self) -> String {
        let mut parts = vec![format!(
            "FREQ={}",
            match self.freq {
                Freq::Daily => "DAILY",
                Freq::Weekly => "WEEKLY",
                Freq::Monthly => "MONTHLY",
                Freq::Yearly => "YEARLY",
            }
        )];
        if self.interval > 1 {
            parts.push(format!("INTERVAL={}", self.interval));
        }
        if !self.byday.is_empty() {
            let v: Vec<String> =
                self.byday.iter().map(|(n, w)| format!("{}{}", n.map(|n| n.to_string()).unwrap_or_default(), wd_code(*w))).collect();
            parts.push(format!("BYDAY={}", v.join(",")));
        }
        if !self.bymonthday.is_empty() {
            parts.push(format!("BYMONTHDAY={}", join(&self.bymonthday)));
        }
        if !self.bymonth.is_empty() {
            parts.push(format!("BYMONTH={}", join(&self.bymonth)));
        }
        if !self.bysetpos.is_empty() {
            parts.push(format!("BYSETPOS={}", join(&self.bysetpos)));
        }
        if let Some(c) = self.count {
            parts.push(format!("COUNT={c}"));
        }
        if let Some(u) = self.until {
            parts.push(format!("UNTIL={}{}", u.format("%Y%m%dT%H%M%S"), if self.until_is_utc { "Z" } else { "" }));
        }
        format!("RRULE:{}", parts.join(";"))
    }

    /// Candidate dates inside one period that starts at `period_start` (a day, week start, month
    /// start or year start depending on FREQ), before BYSETPOS / dtstart filtering.
    fn candidates(&self, period_start: NaiveDate, dtstart: NaiveDate) -> Vec<NaiveDate> {
        let mut out: Vec<NaiveDate> = match self.freq {
            Freq::Daily => {
                let d = period_start;
                let ok_day = self.byday.is_empty() || self.byday.iter().any(|(_, w)| *w == d.weekday());
                let ok_month = self.bymonth.is_empty() || self.bymonth.contains(&d.month());
                let ok_md = self.bymonthday.is_empty() || self.bymonthday.iter().any(|&m| month_day_matches(d, m));
                if ok_day && ok_month && ok_md { vec![d] } else { vec![] }
            }
            Freq::Weekly => {
                let days: Vec<Weekday> =
                    if self.byday.is_empty() { vec![dtstart.weekday()] } else { self.byday.iter().map(|(_, w)| *w).collect() };
                (0..7)
                    .map(|i| period_start + Duration::days(i))
                    .filter(|d| days.contains(&d.weekday()))
                    .filter(|d| self.bymonth.is_empty() || self.bymonth.contains(&d.month()))
                    .collect()
            }
            Freq::Monthly => {
                if !self.bymonth.is_empty() && !self.bymonth.contains(&period_start.month()) {
                    vec![]
                } else {
                    self.month_candidates(period_start.year(), period_start.month(), dtstart)
                }
            }
            Freq::Yearly => {
                let months: Vec<u32> = if self.bymonth.is_empty() { vec![dtstart.month()] } else { self.bymonth.clone() };
                let mut v = vec![];
                for m in months {
                    if self.byday.is_empty() && self.bymonthday.is_empty() {
                        if let Some(d) = NaiveDate::from_ymd_opt(period_start.year(), m, dtstart.day()) {
                            v.push(d);
                        }
                    } else {
                        v.extend(self.month_candidates(period_start.year(), m, dtstart));
                    }
                }
                v
            }
        };
        out.sort();
        out.dedup();
        if !self.bysetpos.is_empty() && !out.is_empty() {
            let n = out.len() as i32;
            let mut picked: Vec<NaiveDate> = self
                .bysetpos
                .iter()
                .filter_map(|&p| {
                    let idx = if p > 0 { p - 1 } else { n + p };
                    (0..n).contains(&idx).then(|| out[idx as usize])
                })
                .collect();
            picked.sort();
            picked.dedup();
            out = picked;
        }
        out
    }

    fn month_candidates(&self, year: i32, month: u32, dtstart: NaiveDate) -> Vec<NaiveDate> {
        let Some(first) = NaiveDate::from_ymd_opt(year, month, 1) else { return vec![] };
        let last = last_day_of_month(year, month);
        let all: Vec<NaiveDate> = (0..last).map(|i| first + Duration::days(i as i64)).collect();
        if !self.bymonthday.is_empty() {
            let mut v: Vec<NaiveDate> = all.iter().copied().filter(|d| self.bymonthday.iter().any(|&m| month_day_matches(*d, m))).collect();
            if !self.byday.is_empty() {
                v.retain(|d| self.byday.iter().any(|(_, w)| *w == d.weekday()));
            }
            return v;
        }
        if !self.byday.is_empty() {
            let mut v = vec![];
            for (n, wd) in &self.byday {
                let matching: Vec<NaiveDate> = all.iter().copied().filter(|d| d.weekday() == *wd).collect();
                match n {
                    None => v.extend(matching),
                    Some(n) if *n > 0 => {
                        if let Some(d) = matching.get((*n - 1) as usize) {
                            v.push(*d)
                        }
                    }
                    Some(n) => {
                        let idx = matching.len() as i32 + n;
                        if idx >= 0
                            && let Some(d) = matching.get(idx as usize)
                        {
                            v.push(*d)
                        }
                    }
                }
            }
            return v;
        }
        NaiveDate::from_ymd_opt(year, month, dtstart.day()).into_iter().collect()
    }

    /// Expand from `dtstart` (wall time). Yields occurrences whose start is in `after..before`,
    /// honouring COUNT and UNTIL (occurrences before `after` still count towards COUNT, but not
    /// towards `max`). `to_utc` converts a wall time to UTC, to compare it with a UTC UNTIL.
    pub fn expand(
        &self,
        dtstart: NaiveDateTime,
        after: NaiveDateTime,
        before: NaiveDateTime,
        to_utc: &dyn Fn(NaiveDateTime) -> NaiveDateTime,
        max: usize,
    ) -> Vec<NaiveDateTime> {
        let time: NaiveTime = dtstart.time();
        let start_date = dtstart.date();
        let mut out = Vec::new();
        let mut produced: u32 = 0;
        let mut period = period_start(self.freq, start_date, self.wkst);
        let mut guard = 0;
        'outer: loop {
            guard += 1;
            if guard > 20_000 {
                break;
            }
            for d in self.candidates(period, start_date) {
                let occ = d.and_time(time);
                if occ < dtstart {
                    continue;
                }
                if let Some(u) = self.until {
                    let cmp = if self.until_is_utc { to_utc(occ) } else { occ };
                    if cmp > u {
                        break 'outer;
                    }
                }
                if let Some(c) = self.count
                    && produced >= c
                {
                    break 'outer;
                }
                if occ >= before {
                    break 'outer;
                }
                produced += 1;
                if occ < after {
                    continue;
                }
                out.push(occ);
                if out.len() >= max {
                    break 'outer;
                }
            }
            period = next_period(self.freq, period, self.interval);
            if period.and_time(NaiveTime::MIN) > before + Duration::days(370) {
                break;
            }
        }
        out
    }
}

fn join<T: ToString>(v: &[T]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")
}

fn month_day_matches(d: NaiveDate, m: i32) -> bool {
    if m > 0 {
        d.day() as i32 == m
    } else {
        let last = last_day_of_month(d.year(), d.month()) as i32;
        d.day() as i32 == last + 1 + m
    }
}

pub fn last_day_of_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    (NaiveDate::from_ymd_opt(ny, nm, 1).unwrap() - Duration::days(1)).day()
}

fn period_start(freq: Freq, d: NaiveDate, wkst: Weekday) -> NaiveDate {
    match freq {
        Freq::Daily => d,
        Freq::Weekly => {
            let back = (d.weekday().num_days_from_monday() + 7 - wkst.num_days_from_monday()) % 7;
            d - Duration::days(back as i64)
        }
        Freq::Monthly => NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap(),
        Freq::Yearly => NaiveDate::from_ymd_opt(d.year(), 1, 1).unwrap(),
    }
}

fn next_period(freq: Freq, p: NaiveDate, interval: u32) -> NaiveDate {
    let i = interval as i64;
    match freq {
        Freq::Daily => p + Duration::days(i),
        Freq::Weekly => p + Duration::days(7 * i),
        Freq::Monthly => {
            let total = p.year() as i64 * 12 + (p.month() as i64 - 1) + i;
            NaiveDate::from_ymd_opt((total / 12) as i32, (total % 12) as u32 + 1, 1).unwrap()
        }
        Freq::Yearly => NaiveDate::from_ymd_opt(p.year() + interval as i32, 1, 1).unwrap(),
    }
}

/// A whole `recurrence` array (RRULE + EXDATE + RDATE lines) applied to a start in zone `tz`.
/// Returns occurrence starts as UTC epoch ms, sorted, those starting before `before_ms`.
pub fn expand_recurrence(lines: &[String], dtstart_ms: i64, all_day: bool, tz: chrono_tz::Tz, before_ms: i64, max: usize) -> Vec<i64> {
    expand_recurrence_from(lines, dtstart_ms, all_day, tz, None, before_ms, max)
}

/// `expand_recurrence` that only collects (and counts towards `max`) rule occurrences from about
/// `from_ms` on, so a long-running series still fills a window far from its first start.
pub fn expand_recurrence_from(
    lines: &[String],
    dtstart_ms: i64,
    all_day: bool,
    tz: chrono_tz::Tz,
    from_ms: Option<i64>,
    before_ms: i64,
    max: usize,
) -> Vec<i64> {
    let wall = |ms: i64| -> NaiveDateTime {
        let utc = chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default();
        if all_day { utc.naive_utc() } else { utc.with_timezone(&tz).naive_local() }
    };
    let to_ms = |n: NaiveDateTime| -> i64 {
        if all_day {
            n.and_utc().timestamp_millis()
        } else {
            match tz.from_local_datetime(&n) {
                chrono::LocalResult::Single(t) => t.timestamp_millis(),
                chrono::LocalResult::Ambiguous(a, _) => a.timestamp_millis(),
                // in a DST gap: move forward an hour
                chrono::LocalResult::None => tz
                    .from_local_datetime(&(n + Duration::hours(1)))
                    .earliest()
                    .map(|t| t.timestamp_millis())
                    .unwrap_or_else(|| n.and_utc().timestamp_millis()),
            }
        }
    };
    let to_utc_naive =
        |n: NaiveDateTime| -> NaiveDateTime { chrono::DateTime::from_timestamp_millis(to_ms(n)).unwrap_or_default().naive_utc() };
    // keep the horizon representable (callers may pass "forever")
    let before_ms = before_ms.min(dtstart_ms.saturating_add(200 * 366 * 86_400_000));
    let start = wall(dtstart_ms);
    let before = wall(before_ms) + Duration::days(1);
    let after = from_ms.map_or(NaiveDateTime::MIN, |f| wall(f) - Duration::days(1));
    let mut occ: Vec<i64> = Vec::new();
    let mut exdates: Vec<NaiveDateTime> = Vec::new();
    let mut have_rule = false;
    for line in lines {
        let up = line.trim();
        if up.starts_with("RRULE") {
            if let Ok(rule) = Rule::parse(up) {
                have_rule = true;
                occ.extend(rule.expand(start, after, before, &to_utc_naive, max).into_iter().map(to_ms));
            }
        } else if let Some(rest) = up.strip_prefix("EXDATE").or_else(|| up.strip_prefix("RDATE")) {
            let is_ex = up.starts_with("EXDATE");
            let vals = rest.rsplit_once(':').map(|(_, v)| v).unwrap_or("");
            for v in vals.split(',') {
                if let Some((dt, utc)) = parse_ical_dt(v) {
                    let wall_dt = if utc && !all_day { wall(dt.and_utc().timestamp_millis()) } else { dt };
                    // date-only EXDATE on a timed event: matches that day at the event's time
                    let wall_dt = if v.trim().len() == 8 && !all_day { wall_dt.date().and_time(start.time()) } else { wall_dt };
                    if is_ex {
                        exdates.push(wall_dt);
                    } else {
                        occ.push(to_ms(wall_dt));
                    }
                }
            }
        }
    }
    if !have_rule {
        occ.push(dtstart_ms);
    }
    let ex_ms: Vec<i64> = exdates.into_iter().map(to_ms).collect();
    occ.retain(|o| !ex_ms.contains(o) && *o < before_ms);
    occ.sort();
    occ.dedup();
    occ
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }
    fn ident(n: NaiveDateTime) -> NaiveDateTime {
        n
    }
    fn exp(rule: &str, start: &str, before: &str) -> Vec<String> {
        Rule::parse(rule)
            .unwrap()
            .expand(dt(start), NaiveDateTime::MIN, dt(before), &ident, 500)
            .iter()
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .collect()
    }

    #[test]
    fn daily_count() {
        assert_eq!(
            exp("RRULE:FREQ=DAILY;COUNT=3", "2026-10-12 09:00", "2027-01-01 00:00"),
            vec!["2026-10-12 09:00", "2026-10-13 09:00", "2026-10-14 09:00"]
        );
    }

    #[test]
    fn daily_interval_until() {
        assert_eq!(
            exp("FREQ=DAILY;INTERVAL=2;UNTIL=20261018", "2026-10-12 09:00", "2027-01-01 00:00"),
            vec!["2026-10-12 09:00", "2026-10-14 09:00", "2026-10-16 09:00", "2026-10-18 09:00"]
        );
    }

    #[test]
    fn weekly_byday() {
        // 2026-10-12 is a Monday
        assert_eq!(
            exp("RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=5", "2026-10-12 10:00", "2027-01-01 00:00"),
            vec!["2026-10-12 10:00", "2026-10-14 10:00", "2026-10-19 10:00", "2026-10-21 10:00", "2026-10-26 10:00"]
        );
    }

    #[test]
    fn weekly_interval_two_starts_midweek() {
        // starts Wednesday; BYDAY includes Monday of the same week (before dtstart → skipped)
        assert_eq!(
            exp("RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=4", "2026-10-14 08:00", "2027-01-01 00:00"),
            vec!["2026-10-14 08:00", "2026-10-26 08:00", "2026-10-28 08:00", "2026-11-09 08:00"]
        );
    }

    #[test]
    fn monthly_nth_weekday_and_last() {
        assert_eq!(
            exp("RRULE:FREQ=MONTHLY;BYDAY=2TU;COUNT=3", "2026-10-13 09:00", "2027-06-01 00:00"),
            vec!["2026-10-13 09:00", "2026-11-10 09:00", "2026-12-08 09:00"]
        );
        assert_eq!(
            exp("RRULE:FREQ=MONTHLY;BYDAY=-1FR;COUNT=2", "2026-10-30 09:00", "2027-06-01 00:00"),
            vec!["2026-10-30 09:00", "2026-11-27 09:00"]
        );
        assert_eq!(
            exp("RRULE:FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1;COUNT=2", "2026-10-30 09:00", "2027-06-01 00:00"),
            vec!["2026-10-30 09:00", "2026-11-30 09:00"]
        );
    }

    #[test]
    fn monthly_by_monthday_skips_short_months() {
        assert_eq!(
            exp("RRULE:FREQ=MONTHLY;COUNT=3", "2027-01-31 09:00", "2028-01-01 00:00"),
            vec!["2027-01-31 09:00", "2027-03-31 09:00", "2027-05-31 09:00"]
        );
    }

    #[test]
    fn yearly() {
        assert_eq!(exp("RRULE:FREQ=YEARLY;COUNT=2", "2026-10-12 00:00", "2030-01-01 00:00"), vec!["2026-10-12 00:00", "2027-10-12 00:00"]);
        assert_eq!(
            exp("RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=4TH;COUNT=2", "2026-11-26 00:00", "2030-01-01 00:00"),
            vec!["2026-11-26 00:00", "2027-11-25 00:00"]
        );
    }

    #[test]
    fn roundtrip_string() {
        let r = Rule::parse("RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,FR;UNTIL=20261231T215959Z").unwrap();
        assert_eq!(r.to_string(), "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,FR;UNTIL=20261231T215959Z");
    }

    /// A daily series from years ago still fills a window today; COUNT still counts from the start.
    #[test]
    fn expansion_from_a_window_far_from_the_start() {
        let utc = chrono_tz::UTC;
        let ms = |s: &str| dt(s).and_utc().timestamp_millis();
        let daily = vec!["RRULE:FREQ=DAILY".to_string()];
        let v =
            expand_recurrence_from(&daily, ms("2020-01-01 09:00"), false, utc, Some(ms("2026-10-01 00:00")), ms("2026-10-11 00:00"), 1000);
        // the window's 10 days plus the day of margin before it
        assert_eq!(v.len(), 11);
        assert_eq!(v[0], ms("2026-09-30 09:00"));
        // without the window start, the 1000 occurrences run out years before it
        let all = expand_recurrence(&daily, ms("2020-01-01 09:00"), false, utc, ms("2026-10-11 00:00"), 1000);
        assert!(all.iter().all(|o| *o < ms("2023-01-01 00:00")));
        let counted = vec!["RRULE:FREQ=DAILY;COUNT=5".to_string()];
        assert!(
            expand_recurrence_from(
                &counted,
                ms("2020-01-01 09:00"),
                false,
                utc,
                Some(ms("2026-10-01 00:00")),
                ms("2026-10-11 00:00"),
                1000
            )
            .is_empty()
        );
    }

    #[test]
    fn recurrence_with_exdate_and_dst() {
        let tz: chrono_tz::Tz = "Asia/Jerusalem".parse().unwrap();
        // Sunday 2026-10-18 10:00 Israel (IDT, UTC+3) weekly; DST ends 2026-10-25 → stays 10:00 local
        let start = tz.with_ymd_and_hms(2026, 10, 18, 10, 0, 0).unwrap().timestamp_millis();
        let lines = vec!["RRULE:FREQ=WEEKLY;BYDAY=SU;COUNT=4".to_string(), "EXDATE;TZID=Asia/Jerusalem:20261101T100000".to_string()];
        let v = expand_recurrence(&lines, start, false, tz, i64::MAX / 2, 100);
        let local: Vec<String> = v
            .iter()
            .map(|ms| chrono::DateTime::from_timestamp_millis(*ms).unwrap().with_timezone(&tz).format("%m-%d %H:%M").to_string())
            .collect();
        assert_eq!(local, vec!["10-18 10:00", "10-25 10:00", "11-08 10:00"]);
    }
}
