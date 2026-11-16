//! Sleep logging helpers: merging overlapping intervals so a night sleep and a
//! nap that overlap are never double counted.

use crate::models::*;
use chrono::{DateTime, Duration, FixedOffset, TimeZone, Utc};

pub fn parse_stamp(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M")
                .ok()
                .map(|n| Utc.from_utc_datetime(&n))
        })
}

pub fn interval(entry: &SleepEntry) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let start = parse_stamp(&entry.start_at)?;
    let end = parse_stamp(&entry.end_at)?;
    if end > start {
        Some((start, end))
    } else {
        None
    }
}

/// Total minutes covered by the union of intervals overlapping the window.
pub fn merged_minutes(
    entries: &[SleepEntry],
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> i64 {
    let mut spans: Vec<(DateTime<Utc>, DateTime<Utc>)> = entries
        .iter()
        .filter_map(interval)
        .filter_map(|(a, b)| {
            let start = a.max(window_start);
            let end = b.min(window_end);
            if end > start {
                Some((start, end))
            } else {
                None
            }
        })
        .collect();
    spans.sort();
    let mut total_seconds = 0i64;
    let mut cursor: Option<(DateTime<Utc>, DateTime<Utc>)> = None;
    for (start, end) in spans {
        match cursor {
            None => cursor = Some((start, end)),
            Some((cs, ce)) => {
                if start <= ce {
                    cursor = Some((cs, ce.max(end)));
                } else {
                    total_seconds += (ce - cs).num_seconds();
                    cursor = Some((start, end));
                }
            }
        }
    }
    if let Some((s, e)) = cursor {
        total_seconds += (e - s).num_seconds();
    }
    (total_seconds / 60).max(0)
}

pub fn recorded_minutes_last_24h(entries: &[SleepEntry], now: DateTime<chrono_tz::Tz>) -> i64 {
    let end = now.with_timezone(&Utc);
    merged_minutes(entries, end - Duration::hours(24), end)
}

pub fn local_range_for_date<Tz: TimeZone>(
    date: chrono::NaiveDate,
    tz: &Tz,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let start = tz
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .unwrap_or_else(|| tz.from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap()));
    let end = start.clone() + Duration::hours(24);
    (start.with_timezone(&Utc), end.with_timezone(&Utc))
}

pub fn parse_local_input(date: &str, time: &str) -> Option<DateTime<FixedOffset>> {
    let dt = format!("{date} {time}");
    DateTime::parse_from_str(&format!("{dt} +0530"), "%Y-%m-%d %H:%M %z").ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn entry(start: &str, end: &str) -> SleepEntry {
        SleepEntry {
            id: "x".into(),
            plan_id: "p".into(),
            kind: SleepKind::Night,
            start_at: start.into(),
            end_at: end.into(),
            energy: None,
            notes: String::new(),
            created_at: String::new(),
        }
    }

    #[test]
    fn overlapping_sleep_is_not_double_counted() {
        let entries = vec![
            entry("2026-12-01T23:00:00+05:30", "2026-12-02T06:00:00+05:30"),
            // A nap fully inside that night.
            entry("2026-12-02T05:00:00+05:30", "2026-12-02T05:30:00+05:30"),
        ];
        let start = DateTime::parse_from_rfc3339("2026-12-01T00:00:00+05:30")
            .unwrap()
            .with_timezone(&Utc);
        let end = start + Duration::hours(48);
        assert_eq!(merged_minutes(&entries, start, end), 420);
    }

    #[test]
    fn adjacent_entries_merge() {
        let entries = vec![
            entry("2026-12-01T22:00:00+05:30", "2026-12-02T00:00:00+05:30"),
            entry("2026-12-02T00:00:00+05:30", "2026-12-02T06:00:00+05:30"),
        ];
        let start = DateTime::parse_from_rfc3339("2026-12-01T00:00:00+05:30")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(merged_minutes(&entries, start, start + Duration::hours(48)), 480);
    }

    #[test]
    fn local_date_range_is_24h() {
        let date = NaiveDate::from_ymd_opt(2026, 11, 15).unwrap();
        let (s, e) = local_range_for_date(date, &chrono_tz::Asia::Kolkata);
        assert_eq!((e - s).num_hours(), 24);
    }
}
