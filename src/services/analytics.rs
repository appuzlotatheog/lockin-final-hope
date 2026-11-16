//! Real analytics computed from sessions, tasks, chapters, revisions and mock
//! tests. Planned time is always shown separately from actually studied time.

use crate::db::repo;
use crate::error::AppResult;
use crate::models::*;
use chrono::{Datelike, Duration, NaiveDate};
use rusqlite::Connection;

#[derive(Debug, Clone, Default)]
pub struct DayPoint {
    pub date_text: String,
    pub weekday: String,
    pub focused_minutes: i64,
    pub planned_minutes: i64,
    pub tasks_done: i64,
    pub height_pct: f64,
    pub planned_pct: f64,
    pub is_today: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SubjectSlice {
    pub name: String,
    pub color: String,
    pub minutes: i64,
    pub percent: f64,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct StatusCount {
    pub label: String,
    pub count: i64,
    pub percent: f64,
    pub tone: String,
}

#[derive(Debug, Clone, Default)]
pub struct MockRow {
    pub id: String,
    pub name: String,
    pub subject: String,
    pub date_text: String,
    pub score_text: String,
    pub percent: f64,
    pub accuracy_text: String,
    pub weak_topics: String,
    pub duration_text: String,
}

#[derive(Debug, Clone, Default)]
pub struct HeatCell {
    pub level: i32,
    pub tooltip: String,
    pub blank: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AnalyticsView {
    pub range_days: i64,
    pub focused_minutes: i64,
    pub planned_minutes: i64,
    pub tasks_done: i64,
    pub tasks_total: i64,
    pub completion_pct: f64,
    pub avg_session_minutes: i64,
    pub session_count: i64,
    pub chapters_ready: i64,
    pub chapters_total: i64,
    pub revision_count: i64,
    pub recall_good: i64,
    pub recall_total: i64,
    pub recall_pct: f64,
    pub daily: Vec<DayPoint>,
    pub subjects: Vec<SubjectSlice>,
    pub statuses: Vec<StatusCount>,
    pub mocks: Vec<MockRow>,
    pub heatmap: Vec<HeatCell>,
    pub sleep_avg_hours: f64,
    pub backlog_count: i64,
    pub overall_accuracy: String,
}

pub fn build(conn: &Connection, plan_id: &str, range_days: i64) -> AppResult<AnalyticsView> {
    let plan = repo::get_plan(conn, plan_id)?;
    let today = time::today_in(&plan.timezone);
    let range_days = range_days.clamp(7, 90);
    let start = today - Duration::days(range_days - 1);
    let sessions = repo::sessions_for_range(conn, plan_id, start, today)?;
    let tasks = repo::tasks_for_range(conn, plan_id, start, today)?;
    let chapters = repo::chapters_for_plan(conn, plan_id)?;
    let subjects = repo::subjects_for_plan(conn, plan_id)?;
    let revisions = repo::revisions_for_plan(conn, plan_id)?;
    let mocks = repo::mock_tests_for_plan(conn, plan_id)?;
    let sleep = repo::sleep_entries_for_plan(conn, plan_id, 40)?;

    let mut view = AnalyticsView {
        range_days,
        chapters_total: chapters.len() as i64,
        chapters_ready: chapters.iter().filter(|c| c.status.is_complete()).count() as i64,
        revision_count: revisions.len() as i64,
        tasks_total: tasks.len() as i64,
        tasks_done: tasks.iter().filter(|t| t.status == TaskStatus::Completed).count() as i64,
        session_count: sessions.len() as i64,
        ..Default::default()
    };
    view.focused_minutes = sessions
        .iter()
        .filter(|s| s.status == "completed")
        .map(|s| s.focused_seconds / 60)
        .sum();
    view.planned_minutes = tasks
        .iter()
        .filter(|t| t.status != TaskStatus::Skipped)
        .map(|t| t.estimated_minutes)
        .sum();
    view.completion_pct = if view.tasks_total > 0 {
        view.tasks_done as f64 / view.tasks_total as f64 * 100.0
    } else {
        0.0
    };
    view.avg_session_minutes = if view.session_count > 0 {
        view.focused_minutes / view.session_count
    } else {
        0
    };
    let decided: Vec<&Revision> = revisions.iter().filter(|r| r.result.is_some()).collect();
    view.recall_total = decided.len() as i64;
    view.recall_good = decided
        .iter()
        .filter(|r| matches!(r.result, Some(RevisionResult::Easy) | Some(RevisionResult::Good)))
        .count() as i64;
    view.recall_pct = if view.recall_total > 0 {
        view.recall_good as f64 / view.recall_total as f64 * 100.0
    } else {
        0.0
    };
    view.backlog_count = repo::backlog_tasks(conn, plan_id)?.len() as i64;

    let attempts: i64 = chapters.iter().map(|c| c.questions_attempted).sum();
    let correct: i64 = chapters.iter().map(|c| c.correct_answers).sum();
    if attempts > 0 {
        view.overall_accuracy = format!("{:.0}%", correct as f64 / attempts as f64 * 100.0);
    } else if view.recall_total > 0 {
        view.overall_accuracy = format!("{:.0}%", view.recall_pct);
    } else {
        view.overall_accuracy = "—".into();
    }

    let max_minutes = sessions
        .iter()
        .map(|s| s.focused_seconds / 60)
        .max()
        .unwrap_or(0)
        .max(30);
    for i in 0..range_days {
        let date = start + Duration::days(i);
        let mins: i64 = sessions
            .iter()
            .filter(|s| s.on_date == date && s.status == "completed")
            .map(|s| s.focused_seconds / 60)
            .sum();
        let planned: i64 = tasks
            .iter()
            .filter(|t| t.date == date && t.status != TaskStatus::Skipped)
            .map(|t| t.estimated_minutes)
            .sum();
        let done = tasks
            .iter()
            .filter(|t| t.date == date && t.status == TaskStatus::Completed)
            .count() as i64;
        view.daily.push(DayPoint {
            date_text: time::fmt_date_short(date),
            weekday: time::weekday_name(date).to_string(),
            focused_minutes: mins,
            planned_minutes: planned,
            tasks_done: done,
            height_pct: (mins as f64 / max_minutes as f64 * 100.0).clamp(0.0, 100.0),
            planned_pct: (planned as f64 / max_minutes as f64 * 100.0).clamp(0.0, 100.0),
            is_today: date == today,
        });
    }

    let mut per_subject: Vec<(String, String, i64)> = subjects
        .iter()
        .map(|s| (s.name.clone(), s.color.clone(), 0i64))
        .collect();
    let mut unassigned = 0i64;
    for session in sessions.iter().filter(|s| s.status == "completed") {
        match session
            .subject_id
            .as_ref()
            .and_then(|id| subjects.iter().find(|s| &s.id == id))
        {
            Some(subject) => {
                if let Some(row) = per_subject.iter_mut().find(|r| r.0 == subject.name) {
                    row.2 += session.focused_seconds / 60;
                }
            }
            None => unassigned += session.focused_seconds / 60,
        }
    }
    if unassigned > 0 {
        per_subject.push(("Unassigned".into(), "#6D6580".into(), unassigned));
    }
    per_subject.retain(|r| r.2 > 0);
    per_subject.sort_by_key(|r| -r.2);
    let subject_total: i64 = per_subject.iter().map(|r| r.2).sum();
    for (name, color, minutes) in per_subject {
        let percent = if subject_total > 0 {
            minutes as f64 / subject_total as f64 * 100.0
        } else {
            0.0
        };
        view.subjects.push(SubjectSlice {
            name,
            color,
            minutes,
            percent,
            text: format!("{} · {:.0}%", time::human_duration(minutes * 60), percent),
        });
    }

    for status in ChapterStatus::ALL {
        let count = chapters.iter().filter(|c| c.status == *status).count() as i64;
        view.statuses.push(StatusCount {
            label: status.label().to_string(),
            count,
            percent: if chapters.is_empty() {
                0.0
            } else {
                count as f64 / chapters.len() as f64 * 100.0
            },
            tone: match status {
                ChapterStatus::NotStarted => "muted".into(),
                ChapterStatus::Learning => "accent".into(),
                ChapterStatus::NeedsPractice => "warning".into(),
                ChapterStatus::NeedsRevision => "warning".into(),
                ChapterStatus::ExamReady => "success".into(),
            },
        });
    }

    for mock in mocks.iter().take(8) {
        let subject = subjects
            .iter()
            .find(|s| s.id == mock.subject_id)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        view.mocks.push(MockRow {
            id: mock.id.clone(),
            name: mock.name.clone(),
            subject,
            date_text: time::fmt_date_short(mock.date),
            score_text: format!("{}/{}", fmt_num(mock.score), fmt_num(mock.max_marks)),
            percent: mock.percentage(),
            accuracy_text: format!("{:.0}%", mock.percentage()),
            weak_topics: mock.weak_topics.clone(),
            duration_text: format!("{}m", mock.duration_minutes),
        });
    }

    // 12-week consistency heatmap (oldest first, padded to whole weeks).
    let heat_start = today - Duration::days(83);
    for i in 0..84 {
        let date = heat_start + Duration::days(i);
        let mins: i64 = sessions
            .iter()
            .filter(|s| s.on_date == date && s.status == "completed")
            .map(|s| s.focused_seconds / 60)
            .sum();
        let tasks_done = tasks
            .iter()
            .filter(|t| t.date == date && t.status == TaskStatus::Completed)
            .count();
        let level = match mins {
            0 => 0,
            1..=29 => 1,
            30..=59 => 2,
            60..=119 => 3,
            _ => 4,
        };
        view.heatmap.push(HeatCell {
            level,
            tooltip: if mins == 0 && tasks_done == 0 {
                format!("{} — no recorded study", time::fmt_date_short(date))
            } else {
                format!(
                    "{} — {} studied · {} tasks done",
                    time::fmt_date_short(date),
                    time::human_duration(mins * 60),
                    tasks_done
                )
            },
            blank: false,
        });
    }
    let pad = (heat_start.weekday().num_days_from_monday()) as usize;
    for _ in 0..pad {
        view.heatmap.insert(
            0,
            HeatCell {
                level: 0,
                tooltip: String::new(),
                blank: true,
            },
        );
    }

    let mut sleep_total = 0i64;
    for i in 0..7 {
        let date = today - Duration::days(i);
        let (s, e) = crate::services::sleep::local_range_for_date(date, &plan.tz());
        sleep_total += crate::services::sleep::merged_minutes(&sleep, s, e);
    }
    view.sleep_avg_hours = (sleep_total as f64 / 7.0 / 60.0 * 10.0).round() / 10.0;
    Ok(view)
}

fn fmt_num(v: f64) -> String {
    if (v - v.round()).abs() < 0.01 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analytics_reads_real_rows() {
        let dir = std::env::temp_dir().join(format!("lockin-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = crate::db::Database::open(&dir.join("t.db")).unwrap();
        db.migrate().unwrap();
        db.seed_if_empty().unwrap();
        let view = build(&db.conn, crate::db::seed::SEED_PLAN_ID, 30).unwrap();
        assert_eq!(view.chapters_total, 52);
        assert_eq!(view.chapters_ready, 0);
        assert_eq!(view.focused_minutes, 0);
        assert_eq!(view.daily.len(), 30);
        assert!(view.heatmap.len() >= 84); // padded to week boundary
        let _ = std::fs::remove_dir_all(dir);
    }
}
