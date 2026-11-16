//! Timezone-aware date/time helpers. The app is built for Asia/Kolkata but the
//! timezone is stored per plan and in settings, so all helpers accept an IANA
//! timezone name and fall back to Asia/Kolkata when it cannot be parsed.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Timelike, Weekday};
use chrono_tz::Tz;

pub const DEFAULT_TZ: &str = "Asia/Kolkata";

pub fn parse_tz(name: &str) -> Tz {
    name.parse::<Tz>().unwrap_or(chrono_tz::Asia::Kolkata)
}

pub fn now_in(name: &str) -> DateTime<Tz> {
    let tz = parse_tz(name);
    Utc::now().with_timezone(&tz)
}

use chrono::Utc;

pub fn today_in(name: &str) -> NaiveDate {
    now_in(name).date_naive()
}

pub fn parse_date(s: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|_| format!("'{s}' is not a valid date (expected YYYY-MM-DD)"))
}

pub fn parse_time(s: &str) -> Result<NaiveTime, String> {
    let s = s.trim();
    NaiveTime::parse_from_str(s, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(s, "%H:%M:%S"))
        .map_err(|_| format!("'{s}' is not a valid time (expected HH:MM, 24-hour)"))
}

pub fn minutes_between(start: NaiveTime, end: NaiveTime) -> i64 {
    let secs = end.num_seconds_from_midnight() as i64 - start.num_seconds_from_midnight() as i64;
    if secs >= 0 {
        secs / 60
    } else {
        // A block crossing midnight.
        (24 * 3600 + secs) / 60
    }
}

pub fn fmt_time(t: NaiveTime) -> String {
    format!("{:02}:{:02}", t.hour(), t.minute())
}

pub fn fmt_time_12(t: NaiveTime) -> String {
    t.format("%-I:%M %p").to_string()
}

pub fn fmt_date(d: NaiveDate) -> String {
    d.format("%a, %d %b %Y").to_string()
}

pub fn fmt_date_short(d: NaiveDate) -> String {
    d.format("%d %b").to_string()
}

pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

pub fn weekday_index(d: NaiveDate) -> usize {
    d.weekday().num_days_from_monday() as usize
}

pub fn weekday_name(d: NaiveDate) -> &'static str {
    WEEKDAYS[weekday_index(d)]
}

pub fn start_of_week(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

pub fn days_between(a: NaiveDate, b: NaiveDate) -> i64 {
    b.signed_duration_since(a).num_days()
}

pub fn date_range(start: NaiveDate, end: NaiveDate) -> Vec<NaiveDate> {
    let mut out = Vec::new();
    let mut d = start;
    while d <= end {
        out.push(d);
        d += Duration::days(1);
    }
    out
}

/// Human friendly countdown between two instants, e.g. "23d 4h 12m".
pub fn countdown_text(target: DateTime<Tz>, now: DateTime<Tz>) -> String {
    let delta = target.signed_duration_since(now);
    if delta.num_seconds() <= 0 {
        return "Started".to_string();
    }
    let days = delta.num_days();
    let hours = delta.num_hours() % 24;
    let minutes = delta.num_minutes() % 60;
    let seconds = delta.num_seconds() % 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else {
        format!("{minutes}m {seconds}s")
    }
}

/// Combine a local date + time in a timezone into a concrete instant.
pub fn at_local(tz: &str, date: NaiveDate, time: NaiveTime) -> DateTime<Tz> {
    let tz = parse_tz(tz);
    tz.from_local_datetime(&date.and_time(time))
        .single()
        .unwrap_or_else(|| tz.from_utc_datetime(&date.and_time(time)))
}

pub fn human_duration(seconds: i64) -> String {
    let m = seconds.max(0) / 60;
    if m < 60 {
        format!("{m}m")
    } else {
        format!("{}h {:02}m", m / 60, m % 60)
    }
}

pub fn is_weekend(d: NaiveDate) -> bool {
    matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}
