//! Pure scheduling rules: template expansion, overlap detection, replanning and
//! the "What should I study now?" recommendation. No database or UI access, so
//! everything here is unit-testable.

use crate::models::*;
use chrono::{Datelike, NaiveDate, NaiveTime, Timelike, Utc};
use uuid::Uuid;

pub fn default_day_type(date: NaiveDate) -> DayType {
    match date.weekday() {
        chrono::Weekday::Sat => DayType::Saturday,
        chrono::Weekday::Sun => DayType::Sunday,
        _ => DayType::Weekday,
    }
}

pub fn effective_day_type(date: NaiveDate, overrides: &[DayOverride]) -> DayType {
    overrides
        .iter()
        .find(|o| o.date == date)
        .map(|o| o.day_type)
        .unwrap_or_else(|| default_day_type(date))
}

pub fn template_for<'a>(
    templates: &'a [ScheduleTemplate],
    day_type: DayType,
) -> Option<&'a ScheduleTemplate> {
    templates
        .iter()
        .find(|t| t.day_types.contains(&day_type))
        .or_else(|| {
            // Custom days fall back to the weekday template only when one exists.
            if day_type == DayType::Custom {
                templates.iter().find(|t| t.day_types.contains(&DayType::Weekday))
            } else {
                None
            }
        })
}

/// Turn a day's study slots into planned tasks. Break and meal slots are shown
/// as fixed periods but never become tasks.
pub fn planned_tasks_for_date(
    plan: &Plan,
    date: NaiveDate,
    templates: &[ScheduleTemplate],
    overrides: &[DayOverride],
) -> Vec<Task> {
    if date < plan.start_date || date > plan.end_date {
        return Vec::new();
    }
    let day_type = effective_day_type(date, overrides);
    let Some(template) = template_for(templates, day_type) else {
        return Vec::new();
    };
    if day_type == DayType::Holiday {
        return Vec::new();
    }
    let now = Utc::now().to_rfc3339();
    let mut tasks: Vec<Task> = template
        .slots
        .iter()
        .filter(|s| s.kind == SlotKind::Study)
        .map(|slot| Task {
            id: Uuid::new_v4().to_string(),
            plan_id: plan.id.clone(),
            date,
            title: slot.title.clone(),
            task_type: TaskType::Study,
            subject_id: slot.subject_id.clone(),
            chapter_id: slot.chapter_id.clone(),
            start_time: slot.start_time,
            end_time: slot.end_time,
            estimated_minutes: time::minutes_between(slot.start_time, slot.end_time),
            priority: 2,
            status: TaskStatus::Planned,
            completed_at: None,
            notes: String::new(),
            source_template_id: Some(template.id.clone()),
            created_at: now.clone(),
        })
        .collect();
    tasks.sort_by_key(|t| t.start_time);
    tasks
}

/// Fixed periods (breaks/meals) displayed alongside tasks.
pub fn fixed_periods_for_date(
    date: NaiveDate,
    templates: &[ScheduleTemplate],
    overrides: &[DayOverride],
) -> Vec<TemplateSlot> {
    let day_type = effective_day_type(date, overrides);
    if day_type == DayType::Holiday {
        return Vec::new();
    }
    template_for(templates, day_type)
        .map(|t| {
            let mut slots: Vec<TemplateSlot> = t
                .slots
                .iter()
                .filter(|s| s.kind != SlotKind::Study)
                .cloned()
                .collect();
            slots.sort_by_key(|s| s.start_time);
            slots
        })
        .unwrap_or_default()
}

pub fn overlaps(a_start: NaiveTime, a_end: NaiveTime, b_start: NaiveTime, b_end: NaiveTime) -> bool {
    let to_min = |t: NaiveTime| t.num_seconds_from_midnight() as i64 / 60;
    let (as_, ae) = (to_min(a_start), to_min(a_end));
    let (bs, be) = (to_min(b_start), to_min(b_end));
    as_ < be && bs < ae
}

/// Pairs of tasks whose scheduled blocks intersect (same date only).
pub fn overlapping_tasks(tasks: &[Task]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (i, a) in tasks.iter().enumerate() {
        for b in tasks.iter().skip(i + 1) {
            if a.date == b.date && overlaps(a.start_time, a.end_time, b.start_time, b.end_time) {
                out.push((a.id.clone(), b.id.clone()));
            }
        }
    }
    out
}

pub fn is_protected(status: TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Completed | TaskStatus::PartiallyCompleted | TaskStatus::InProgress
    )
}

#[derive(Debug, Clone, Default)]
pub struct ReplanOutcome {
    pub added: Vec<Task>,
    pub backlog_moves: Vec<Task>,
    pub protected_conflicts: Vec<String>,
    pub duplicate_titles: Vec<String>,
}

/// Build a replan proposal for a single day.
///
/// - Completed / partially completed / in-progress tasks are never modified.
/// - Planned tasks that a new slot would duplicate are left alone.
/// - Planned tasks that collide with a template study slot and are not already
///   scheduled at the same time are moved to the backlog (not deleted).
pub fn replan_day(
    plan: &Plan,
    date: NaiveDate,
    templates: &[ScheduleTemplate],
    overrides: &[DayOverride],
    existing: &[Task],
) -> ReplanOutcome {
    let mut outcome = ReplanOutcome::default();
    let proposed = planned_tasks_for_date(plan, date, templates, overrides);
    if proposed.is_empty() {
        return outcome;
    }

    for slot_task in proposed {
        let duplicate = existing.iter().any(|t| {
            t.start_time == slot_task.start_time
                && t.end_time == slot_task.end_time
                && (t.status != TaskStatus::Skipped)
        });
        if duplicate {
            outcome.duplicate_titles.push(slot_task.title.clone());
            continue;
        }
        let protected_conflict = existing.iter().find(|t| {
            is_protected(t.status)
                && overlaps(t.start_time, t.end_time, slot_task.start_time, slot_task.end_time)
        });
        if let Some(p) = protected_conflict {
            outcome.protected_conflicts.push(p.title.clone());
            continue;
        }
        let planned_conflict: Vec<&Task> = existing
            .iter()
            .filter(|t| {
                t.status == TaskStatus::Planned
                    && t.task_type != TaskType::Backlog
                    && overlaps(t.start_time, t.end_time, slot_task.start_time, slot_task.end_time)
            })
            .collect();
        for stale in planned_conflict {
            if !outcome.backlog_moves.iter().any(|m| m.id == stale.id) {
                let mut moved = stale.clone();
                moved.task_type = TaskType::Backlog;
                moved.status = TaskStatus::Planned;
                outcome.backlog_moves.push(moved);
            }
        }
        outcome.added.push(slot_task);
    }
    outcome
}

pub fn next_interval(current: i64, result: RevisionResult, policy: &[i64], ease: f64) -> (i64, f64) {
    let base = if policy.is_empty() { vec![1, 3, 7] } else { policy.to_vec() };
    let next_up = base.iter().find(|v| **v > current).copied().unwrap_or_else(|| {
        let last = *base.last().unwrap_or(&7);
        ((current as f64 * ease.max(1.2)) as i64).max(last + 1)
    });
    match result {
        RevisionResult::Easy => {
            let mut ease = (ease + 0.15).min(3.2);
            ease = (ease * 100.0).round() / 100.0;
            (next_up, ease)
        }
        RevisionResult::Good => (next_up, ease),
        RevisionResult::Difficult => {
            // Repeat at a shorter interval and lower ease.
            let shorter = (current / 2).max(1);
            (shorter.min(next_up), ((ease - 0.2).max(1.3) * 100.0).round() / 100.0)
        }
        RevisionResult::Failed => (1, ((ease - 0.35).max(1.3) * 100.0).round() / 100.0),
    }
}

/// Recommend something real to study. The order is deliberate: overdue
/// revision → current/next planned task → overdue task → exam-critical chapter.
pub fn recommend(
    date: NaiveDate,
    now: NaiveTime,
    tasks: &[Task],
    chapters: &[Chapter],
    subjects: &[Subject],
    exams: &[Exam],
    revision_chapter_ids: &[String],
) -> Recommendation {
    let subject_name = |id: &Option<String>| {
        id.as_ref()
            .and_then(|sid| subjects.iter().find(|s| &s.id == sid))
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "Any subject".into())
    };

    if let Some(chapter) = revision_chapter_ids
        .iter()
        .filter_map(|id| chapters.iter().find(|c| &c.id == id))
        .next()
    {
        let subject = subjects
            .iter()
            .find(|s| s.id == chapter.subject_id)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        return Recommendation {
            kind: "revision".into(),
            title: format!("Revise: {}", chapter.title),
            subject,
            reason: "Revision is due today. Spaced recall beats re-reading.".into(),
            task_id: None,
            minutes: 20,
        };
    }

    let mut todays: Vec<&Task> = tasks
        .iter()
        .filter(|t| t.date == date)
        .filter(|t| !matches!(t.status, TaskStatus::Completed | TaskStatus::Skipped))
        .collect();
    todays.sort_by_key(|t| t.start_time);

    let active = todays
        .iter()
        .find(|t| t.start_time <= now && t.end_time > now && t.status != TaskStatus::Skipped);
    if let Some(task) = active {
        return Recommendation {
            kind: "task".into(),
            title: task.title.clone(),
            subject: subject_name(&task.subject_id),
            reason: "This block is scheduled right now.".into(),
            task_id: Some(task.id.clone()),
            minutes: time::minutes_between(now, task.end_time).max(5),
        };
    }

    if let Some(task) = todays
        .iter()
        .find(|t| t.start_time > now && t.status == TaskStatus::Planned)
    {
        return Recommendation {
            kind: "task".into(),
            title: task.title.clone(),
            subject: subject_name(&task.subject_id),
            reason: format!("Next up at {}.", time::fmt_time_12(task.start_time)),
            task_id: Some(task.id.clone()),
            minutes: task.estimated_minutes,
        };
    }

    if let Some(task) = todays.iter().find(|t| t.status == TaskStatus::Planned) {
        return Recommendation {
            kind: "task".into(),
            title: task.title.clone(),
            subject: subject_name(&task.subject_id),
            reason: "Still incomplete from today's plan.".into(),
            task_id: Some(task.id.clone()),
            minutes: task.estimated_minutes,
        };
    }

    // Exam-driven fallback: pick a chapter from the nearest paper that is not
    // exam-ready yet. This is a real chapter, not an invented schedule entry.
    let mut upcoming: Vec<&Exam> = exams.iter().filter(|e| e.date >= date).collect();
    upcoming.sort_by_key(|e| (e.date, e.start_time));
    if let Some(exam) = upcoming.first() {
        if let Some(chapter) = chapters
            .iter()
            .filter(|c| c.subject_id == exam.subject_id && c.status != ChapterStatus::ExamReady)
            .min_by_key(|c| c.status.index())
        {
            let subject = subjects
                .iter()
                .find(|s| s.id == exam.subject_id)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            return Recommendation {
                kind: "chapter".into(),
                title: format!("{}: {}", subject, chapter.title),
                subject,
                reason: format!(
                    "{} is on {} and this chapter is not exam ready.",
                    exam.label,
                    time::fmt_date_short(exam.date)
                ),
                task_id: None,
                minutes: 40,
            };
        }
    }

    Recommendation {
        kind: "free".into(),
        title: "No pending work".into(),
        subject: String::new(),
        reason: "Nothing is due. Rest, or add a study block to the timetable.".into(),
        task_id: None,
        minutes: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn task(id: &str, start: NaiveTime, end: NaiveTime, status: TaskStatus) -> Task {
        Task {
            id: id.into(),
            plan_id: "p".into(),
            date: NaiveDate::from_ymd_opt(2026, 11, 16).unwrap(),
            title: id.into(),
            task_type: TaskType::Study,
            subject_id: None,
            chapter_id: None,
            start_time: start,
            end_time: end,
            estimated_minutes: 60,
            priority: 2,
            status,
            completed_at: None,
            notes: String::new(),
            source_template_id: None,
            created_at: String::new(),
        }
    }

    #[test]
    fn overlap_detection_is_half_open() {
        assert!(overlaps(t(10, 0), t(11, 0), t(10, 30), t(11, 30)));
        assert!(!overlaps(t(10, 0), t(11, 0), t(11, 0), t(12, 0)));
        assert!(!overlaps(t(10, 0), t(11, 0), t(9, 0), t(10, 0)));
    }

    #[test]
    fn protected_tasks_survive_replan() {
        let plan = Plan {
            id: "p".into(),
            name: "P".into(),
            academic_year: "".into(),
            timezone: "Asia/Kolkata".into(),
            start_date: NaiveDate::from_ymd_opt(2026, 11, 16).unwrap(),
            end_date: NaiveDate::from_ymd_opt(2026, 12, 17).unwrap(),
            archived: false,
            created_at: String::new(),
        };
        let templates = vec![ScheduleTemplate {
            id: "t".into(),
            plan_id: "p".into(),
            name: "School".into(),
            day_types: vec![DayType::Weekday],
            slots: vec![TemplateSlot {
                id: "s".into(),
                template_id: "t".into(),
                start_time: t(19, 0),
                end_time: t(20, 0),
                kind: SlotKind::Study,
                title: "Physics".into(),
                subject_id: None,
                chapter_id: None,
            }],
        }];
        let date = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let existing = vec![task("done", t(19, 15), t(19, 45), TaskStatus::Completed)];
        let outcome = replan_day(&plan, date, &templates, &[], &existing);
        assert!(outcome.added.is_empty());
        assert_eq!(outcome.protected_conflicts.len(), 1);
    }

    #[test]
    fn failed_revision_comes_back_tomorrow() {
        let (days, ease) = next_interval(7, RevisionResult::Failed, &[1, 3, 7], 2.5);
        assert_eq!(days, 1);
        assert!(ease < 2.5);
    }

    #[test]
    fn good_revision_advances_policy() {
        let (days, _) = next_interval(1, RevisionResult::Good, &[1, 3, 7], 2.5);
        assert_eq!(days, 3);
        let (days, _) = next_interval(7, RevisionResult::Good, &[1, 3, 7], 2.5);
        assert!(days > 7);
    }
}
