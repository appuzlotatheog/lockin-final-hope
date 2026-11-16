//! Dashboard aggregation. Every value is read from stored data — nothing here
//! fabricates statistics.

use crate::db::repo;
use crate::error::AppResult;
use crate::models::time;
use crate::models::*;
use crate::scheduling;
use chrono::{Datelike, Duration, NaiveDate, Timelike};
use rusqlite::Connection;

#[derive(Debug, Clone, Default)]
pub struct AgendaRow {
    pub id: String,
    pub title: String,
    pub subject: String,
    pub color: String,
    pub start_text: String,
    pub end_text: String,
    pub status: String,
    pub status_index: i32,
    pub is_fixed: bool,
    pub is_break: bool,
    pub task_type: String,
    pub priority: i64,
    pub minutes: i64,
    pub chapter_id: String,
    pub notes: String,
}

#[derive(Debug, Clone, Default)]
pub struct RevisionRow {
    pub id: String,
    pub title: String,
    pub subject: String,
    pub due_text: String,
    pub overdue: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ExamRow {
    pub id: String,
    pub label: String,
    pub subject: String,
    pub date_text: String,
    pub time_text: String,
    pub countdown: String,
    pub days_left: i64,
    pub color: String,
    pub session: String,
    pub progress: i64,
    pub chapters_ready: i64,
    pub chapters_total: i64,
}

#[derive(Debug, Clone, Default)]
pub struct DashboardView {
    pub greeting: String,
    pub date_line: String,
    pub plan_name: String,
    pub plan_span: String,
    pub next_exam_label: String,
    pub next_exam_countdown: String,
    pub next_exam_when: String,
    pub agenda: Vec<AgendaRow>,
    pub planned_minutes: i64,
    pub focused_minutes: i64,
    pub goal_minutes: i64,
    pub goal_progress: f64,
    pub chapters_done: i64,
    pub chapters_total: i64,
    pub chapter_progress: f64,
    pub revisions_due: i64,
    pub revisions_overdue: i64,
    pub revisions: Vec<RevisionRow>,
    pub overdue_tasks: i64,
    pub streak: i64,
    pub next_block: String,
    pub next_block_time: String,
    pub recommendation_title: String,
    pub recommendation_subject: String,
    pub recommendation_reason: String,
    pub recommendation_minutes: i64,
    pub recommendation_task_id: String,
    pub recommendation_chapter_id: String,
    pub recommendation_kind: String,
    pub sleep_hours: String,
    pub exam_mode: bool,
    pub tasks_done_today: i64,
    pub tasks_total_today: i64,
}

pub fn greeting(name: &str, hour: u32) -> String {
    let part = if hour < 12 {
        "Good morning"
    } else if hour < 17 {
        "Good afternoon"
    } else {
        "Good evening"
    };
    if name.trim().is_empty() {
        part.to_string()
    } else {
        format!("{part}, {}", name.trim())
    }
}

pub fn streak_from_dates(mut dates: Vec<NaiveDate>, today: NaiveDate) -> i64 {
    dates.sort();
    dates.dedup();
    if dates.is_empty() {
        return 0;
    }
    let mut anchor = if dates.contains(&today) {
        today
    } else if dates.contains(&(today - Duration::days(1))) {
        today - Duration::days(1)
    } else {
        return 0;
    };
    let mut streak = 0;
    while dates.contains(&anchor) {
        streak += 1;
        anchor -= Duration::days(1);
    }
    streak
}

pub fn build(conn: &Connection, plan_id: &str, settings: &Settings) -> AppResult<DashboardView> {
    let bundle = crate::services::plan::load_bundle(conn, plan_id)?;
    let now = time::now_in(&bundle.plan.timezone);
    let today = now.date_naive();
    let tasks = repo::tasks_for_date(conn, plan_id, today)?;
    let overrides = repo::day_overrides_for_plan(conn, plan_id)?;
    let sessions = repo::sessions_for_date(conn, plan_id, today)?;
    let revisions = repo::revisions_for_plan(conn, plan_id)?;
    let all_chapters = repo::chapters_for_plan(conn, plan_id)?;
    let mut view = DashboardView::default();

    view.greeting = greeting(&settings.display_name, now.hour());
    view.date_line = format!(
        "{}, {} {}",
        time::fmt_date(today),
        now.format("%H:%M"),
        bundle.plan.timezone
    );
    view.plan_name = bundle.plan.name.clone();
    view.plan_span = format!(
        "{} – {}",
        time::fmt_date_short(bundle.plan.start_date),
        time::fmt_date_short(bundle.plan.end_date)
    );

    let subject_of = |id: &Option<String>| -> Option<&Subject> {
        id.as_ref().and_then(|sid| bundle.subjects.iter().find(|(s, _)| &s.id == sid).map(|(s, _)| s))
    };

    // Next exam (precise per-exam start time).
    let mut next: Option<(Exam, chrono::DateTime<chrono_tz::Tz>)> = None;
    for exam in &bundle.exams {
        let at = time::at_local(&bundle.plan.timezone, exam.date, exam.start_time);
        if at >= now && next.as_ref().map(|(_, n)| at < *n).unwrap_or(true) {
            next = Some((exam.clone(), at));
        }
    }
    if let Some((exam, at)) = next {
        let days_left = (exam.date - today).num_days();
        view.next_exam_label = exam.label.clone();
        view.next_exam_countdown = time::countdown_text(at, now);
        view.next_exam_when = format!(
            "{} · {} IST · {}",
            time::fmt_date(exam.date),
            time::fmt_time_12(exam.start_time),
            exam.session
        );
        view.exam_mode = days_left <= 3 && days_left >= 0;
    }

    // Agenda merges tasks with the template's fixed breaks/meals.
    let mut agenda: Vec<AgendaRow> = Vec::new();
    for task in &tasks {
        let subj = subject_of(&task.subject_id);
        let color = subj.map(|s| s.color.clone()).unwrap_or_else(|| "#6D6580".into());
        agenda.push(AgendaRow {
            id: task.id.clone(),
            title: task.title.clone(),
            subject: subj.map(|s| s.name.clone()).unwrap_or_else(|| "General".into()),
            color,
            start_text: time::fmt_time_12(task.start_time),
            end_text: time::fmt_time_12(task.end_time),
            status: task.status.label().into(),
            status_index: match task.status {
                TaskStatus::Planned => 0,
                TaskStatus::InProgress => 1,
                TaskStatus::Completed => 2,
                TaskStatus::PartiallyCompleted => 3,
                TaskStatus::Skipped => 4,
            },
            is_fixed: false,
            is_break: false,
            task_type: task.task_type.label().into(),
            priority: task.priority,
            minutes: task.estimated_minutes,
            chapter_id: task.chapter_id.clone().unwrap_or_default(),
            notes: task.notes.clone(),
        });
    }
    for slot in scheduling::fixed_periods_for_date(today, &bundle.templates, &overrides) {
        agenda.push(AgendaRow {
            id: slot.id.clone(),
            title: slot.title.clone(),
            subject: slot.kind.label().into(),
            color: "#4A4258".into(),
            start_text: time::fmt_time_12(slot.start_time),
            end_text: time::fmt_time_12(slot.end_time),
            status: slot.kind.label().into(),
            status_index: 5,
            is_fixed: true,
            is_break: true,
            task_type: "Fixed".into(),
            priority: 0,
            minutes: time::minutes_between(slot.start_time, slot.end_time),
            chapter_id: String::new(),
            notes: String::new(),
        });
    }
    agenda.sort_by_key(|r| r.start_text.trim_start_matches('1').len());
    agenda.sort_by(|a, b| a.start_text.cmp(&b.start_text));
    // Sort by actual time, not text.
    let time_key = |r: &AgendaRow| -> usize {
        let raw = r.start_text.split(':').next().unwrap_or("0").parse::<usize>().unwrap_or(0);
        let is_pm = r.start_text.contains("PM");
        let mut h = raw % 12;
        if is_pm {
            h += 12;
        }
        h * 60 + r.start_text.split(':').nth(1).and_then(|s| s.split_whitespace().next()).and_then(|m| m.parse().ok()).unwrap_or(0)
    };
    agenda.sort_by_key(time_key);

    view.planned_minutes = tasks
        .iter()
        .filter(|t| t.status != TaskStatus::Skipped)
        .map(|t| t.estimated_minutes)
        .sum();
    view.focused_minutes = sessions
        .iter()
        .filter(|s| s.status == "completed")
        .map(|s| s.focused_seconds / 60)
        .sum();
    view.goal_minutes = settings.daily_goal_minutes;
    view.goal_progress = if settings.daily_goal_minutes > 0 {
        (view.focused_minutes as f64 / settings.daily_goal_minutes as f64 * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    view.tasks_done_today = tasks.iter().filter(|t| t.status == TaskStatus::Completed).count() as i64;
    view.tasks_total_today = tasks.len() as i64;

    view.chapters_total = all_chapters.len() as i64;
    view.chapters_done = all_chapters.iter().filter(|c| c.status.is_complete()).count() as i64;
    view.chapter_progress = if all_chapters.is_empty() {
        0.0
    } else {
        view.chapters_done as f64 / all_chapters.len() as f64 * 100.0
    };

    view.revisions_due = revisions
        .iter()
        .filter(|r| matches!(r.state, RevisionState::Pending | RevisionState::Snoozed))
        .filter(|r| r.snoozed_until.unwrap_or(r.due_date) == today)
        .count() as i64;
    view.revisions_overdue = revisions
        .iter()
        .filter(|r| matches!(r.state, RevisionState::Pending | RevisionState::Snoozed))
        .filter(|r| r.snoozed_until.unwrap_or(r.due_date) < today)
        .count() as i64;
    for rev in revisions
        .iter()
        .filter(|r| matches!(r.state, RevisionState::Pending | RevisionState::Snoozed))
        .filter(|r| r.snoozed_until.unwrap_or(r.due_date) <= today)
        .take(4)
    {
        if let Some(chapter) = all_chapters.iter().find(|c| c.id == rev.chapter_id) {
            let subject = bundle
                .subjects
                .iter()
                .find(|(s, _)| s.id == chapter.subject_id)
                .map(|(s, _)| s.name.clone())
                .unwrap_or_default();
            view.revisions.push(RevisionRow {
                id: rev.id.clone(),
                title: chapter.title.clone(),
                subject,
                due_text: if rev.due_date < today {
                    format!("{} days overdue", (today - rev.due_date).num_days())
                } else {
                    "Due today".into()
                },
                overdue: rev.due_date < today,
            });
        }
    }

    view.overdue_tasks = repo::all_tasks_for_plan(conn, plan_id)?
        .into_iter()
        .filter(|t| t.date < today && matches!(t.status, TaskStatus::Planned | TaskStatus::InProgress))
        .count() as i64;

    let session_dates = repo::session_dates(conn, plan_id)?;
    view.streak = streak_from_dates(session_dates, today);

    if let Some(next_task) = tasks
        .iter()
        .filter(|t| matches!(t.status, TaskStatus::Planned | TaskStatus::InProgress))
        .filter(|t| t.end_time > now.time())
        .min_by_key(|t| t.start_time)
    {
        view.next_block = next_task.title.clone();
        view.next_block_time = format!(
            "{} – {}",
            time::fmt_time_12(next_task.start_time),
            time::fmt_time_12(next_task.end_time)
        );
    } else {
        view.next_block = "No study block left today".into();
        view.next_block_time = String::new();
    }

    let due_revision_chapters: Vec<String> = revisions
        .iter()
        .filter(|r| matches!(r.state, RevisionState::Pending | RevisionState::Snoozed))
        .filter(|r| r.snoozed_until.unwrap_or(r.due_date) <= today)
        .map(|r| r.chapter_id.clone())
        .collect();
    let all_chapters_flat = all_chapters.clone();
    let subjects: Vec<Subject> = bundle.subjects.iter().map(|(s, _)| s.clone()).collect();
    let rec = scheduling::recommend(
        today,
        now.time(),
        &tasks,
        &all_chapters_flat,
        &subjects,
        &bundle.exams,
        &due_revision_chapters,
    );
    view.recommendation_title = rec.title;
    view.recommendation_subject = rec.subject;
    view.recommendation_reason = rec.reason;
    view.recommendation_minutes = rec.minutes;
    view.recommendation_task_id = rec.task_id.unwrap_or_default();
    view.recommendation_kind = rec.kind;
    view.recommendation_chapter_id = all_chapters
        .iter()
        .find(|c| view.recommendation_title.contains(&c.title))
        .map(|c| c.id.clone())
        .unwrap_or_default();

    // Sleep recorded in the trailing 24 hours (merged, no double counting).
    let sleep_entries = repo::sleep_entries_for_plan(conn, plan_id, 20)?;
    let total = crate::services::sleep::recorded_minutes_last_24h(&sleep_entries, now);
    view.sleep_hours = format!("{:.1}h", total as f64 / 60.0);

    view.agenda = agenda;
    Ok(view)
}

pub fn exam_rows(conn: &Connection, plan_id: &str) -> AppResult<Vec<ExamRow>> {
    let bundle = crate::services::plan::load_bundle(conn, plan_id)?;
    let now = time::now_in(&bundle.plan.timezone);
    let chapters = repo::chapters_for_plan(conn, plan_id)?;
    let mut rows = Vec::new();
    for exam in &bundle.exams {
        let at = time::at_local(&bundle.plan.timezone, exam.date, exam.start_time);
        let subject_chapters: Vec<&Chapter> = chapters
            .iter()
            .filter(|c| c.subject_id == exam.subject_id)
            .collect();
        let ready = subject_chapters.iter().filter(|c| c.status.is_complete()).count() as i64;
        let total = subject_chapters.len() as i64;
        let subject = bundle
            .subjects
            .iter()
            .find(|(s, _)| s.id == exam.subject_id)
            .map(|(s, _)| s.clone());
        rows.push(ExamRow {
            id: exam.id.clone(),
            label: exam.label.clone(),
            subject: subject.as_ref().map(|s| s.name.clone()).unwrap_or_default(),
            date_text: time::fmt_date(exam.date),
            time_text: time::fmt_time_12(exam.start_time),
            countdown: if at < now {
                "Completed".into()
            } else {
                time::countdown_text(at, now)
            },
            days_left: (exam.date - now.date_naive()).num_days(),
            color: subject.map(|s| s.color).unwrap_or_else(|| "#9B7BFF".into()),
            session: exam.session.clone(),
            progress: if total > 0 { (ready as f64 / total as f64 * 100.0).round() as i64 } else { 0 },
            chapters_ready: ready,
            chapters_total: total,
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streak_stops_at_a_gap() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 15).unwrap();
        let dates = vec![
            today,
            today - Duration::days(1),
            today - Duration::days(2),
            today - Duration::days(4),
        ];
        assert_eq!(streak_from_dates(dates, today), 3);
    }

    #[test]
    fn streak_survives_unstudied_morning() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 15).unwrap();
        let dates = vec![today - Duration::days(1), today - Duration::days(2)];
        assert_eq!(streak_from_dates(dates, today), 2);
    }

    #[test]
    fn greeting_is_dynamic() {
        assert_eq!(greeting("", 8), "Good morning");
        assert_eq!(greeting("Mira", 20), "Good evening, Mira");
    }
}
