//! Transactional application of a validated Lockin Plan import.
//!
//! Merge: upserts configuration and preserves completed work.
//! Replace: replaces only the selected plan's configuration and future planned
//! tasks. Study sessions, mock tests, mistakes, sleep logs and revision history
//! are never deleted.

use super::format::*;
use super::validate::{validate_semantics, PreparedPlan};
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::models::*;
use crate::scheduling;
use chrono::{NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    Merge,
    Replace,
}

impl ImportMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Replace => "replace",
        }
    }
    pub fn from_index(i: i32) -> Self {
        if i == 1 {
            Self::Replace
        } else {
            Self::Merge
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ApplySummary {
    pub plan_id: String,
    pub mode: String,
    pub subjects: i64,
    pub chapters: i64,
    pub exams: i64,
    pub templates: i64,
    pub slots: i64,
    pub tasks_added: i64,
    pub tasks_updated: i64,
    pub tasks_preserved: i64,
    pub tasks_deleted: i64,
}

impl ApplySummary {
    pub fn message(&self) -> String {
        format!(
            "Imported {} into '{}': {} subjects, {} chapters, {} exams, {} tasks added{}.",
            self.mode,
            self.plan_id,
            self.subjects,
            self.chapters,
            self.exams,
            self.tasks_added,
            if self.tasks_preserved > 0 {
                format!(", {} completed tasks preserved", self.tasks_preserved)
            } else {
                String::new()
            }
        )
    }
}

pub fn apply(
    conn: &mut Connection,
    prepared: &PreparedPlan,
    mode: ImportMode,
) -> AppResult<ApplySummary> {
    let issues = validate_semantics(&prepared.file);
    let errors: Vec<String> = issues
        .iter()
        .filter(|i| i.is_error())
        .map(|i| i.display())
        .collect();
    if !errors.is_empty() {
        return Err(AppError::Validation(format!(
            "Import blocked — fix these errors first:\n{}",
            errors.join("\n")
        )));
    }
    let file = &prepared.file;
    let plan_exists: Option<String> = conn
        .query_row("SELECT id FROM plans WHERE id=?1", [&file.plan.id], |r| r.get(0))
        .optional()?;
    let today = time::today_in(&file.plan.timezone);
    let mut summary = ApplySummary {
        plan_id: file.plan.id.clone(),
        mode: mode.as_str().into(),
        ..Default::default()
    };

    let tx = conn.transaction()?;
    if plan_exists.is_some() {
        let mut plan = repo::get_plan(&tx, &file.plan.id)?;
        plan.name = file.plan.name.trim().to_string();
        plan.timezone = file.plan.timezone.clone();
        plan.start_date = time::parse_date(&file.plan.start_date).map_err(AppError::Validation)?;
        plan.end_date = time::parse_date(&file.plan.end_date).map_err(AppError::Validation)?;
        plan.academic_year = file.plan.academic_year.clone();
        repo::update_plan(&tx, &plan)?;
    } else {
        repo::insert_plan(
            &tx,
            &Plan {
                id: file.plan.id.clone(),
                name: file.plan.name.trim().to_string(),
                academic_year: file.plan.academic_year.clone(),
                timezone: file.plan.timezone.clone(),
                start_date: time::parse_date(&file.plan.start_date).map_err(AppError::Validation)?,
                end_date: time::parse_date(&file.plan.end_date).map_err(AppError::Validation)?,
                archived: false,
                created_at: Utc::now().to_rfc3339(),
            },
        )?;
    }

    if mode == ImportMode::Replace {
        summary.tasks_deleted = delete_future_planned_tasks(&tx, &file.plan.id, today)?;
        tx.execute("DELETE FROM exams WHERE plan_id=?1", [&file.plan.id])?;
        tx.execute(
            "DELETE FROM schedule_templates WHERE plan_id=?1",
            [&file.plan.id],
        )?;
        remove_unlisted_configuration(&tx, &file)?;
    }

    // Subjects + chapters. Progress on existing chapters is always preserved.
    for (s_idx, subject_def) in file.subjects.iter().enumerate() {
        let existing = tx
            .query_row(
                "SELECT plan_id FROM subjects WHERE id=?1",
                [&subject_def.id],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        if let Some(owner) = existing {
            if owner != file.plan.id {
                return Err(AppError::Validation(format!(
                    "Subject id '{}' is already used by another plan.",
                    subject_def.id
                )));
            }
            let mut subject = repo::get_subject(&tx, &subject_def.id)?;
            subject.name = subject_def.name.trim().to_string();
            subject.color = normalize_color(&subject_def.color);
            subject.sort_order = if subject_def.sort_order != 0 {
                subject_def.sort_order
            } else {
                s_idx as i64
            };
            subject.portions_pending = subject_def.chapters.is_empty();
            repo::update_subject(&tx, &subject)?;
        } else {
            repo::insert_subject(
                &tx,
                &Subject {
                    id: subject_def.id.clone(),
                    plan_id: file.plan.id.clone(),
                    name: subject_def.name.trim().to_string(),
                    color: normalize_color(&subject_def.color),
                    sort_order: if subject_def.sort_order != 0 {
                        subject_def.sort_order
                    } else {
                        s_idx as i64
                    },
                    portions_pending: subject_def.chapters.is_empty(),
                },
            )?;
        }
        summary.subjects += 1;
        for (c_idx, chapter_def) in subject_def.chapters.iter().enumerate() {
            let existing = tx
                .query_row(
                    "SELECT subject_id FROM chapters WHERE id=?1",
                    [&chapter_def.id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?;
            if let Some(owner) = existing {
                if owner != subject_def.id {
                    return Err(AppError::Validation(format!(
                        "Chapter id '{}' is already used by another subject.",
                        chapter_def.id
                    )));
                }
                let mut chapter = repo::get_chapter(&tx, &chapter_def.id)?;
                chapter.title = chapter_def.title.trim().to_string();
                chapter.sort_order = chapter_def.order;
                repo::update_chapter(&tx, &chapter)?;
            } else {
                repo::insert_chapter(
                    &tx,
                    &Chapter {
                        id: chapter_def.id.clone(),
                        subject_id: subject_def.id.clone(),
                        title: chapter_def.title.trim().to_string(),
                        sort_order: chapter_def.order,
                        status: chapter_def.status.unwrap_or(ChapterStatus::NotStarted),
                        concept_done: false,
                        textbook_done: false,
                        exercises_done: false,
                        selftest_done: false,
                        confidence: None,
                        questions_attempted: 0,
                        correct_answers: 0,
                        last_studied: None,
                        next_revision: None,
                        study_seconds: 0,
                        notes: String::new(),
                    },
                )?;
            }
            summary.chapters += 1;
        }
    }

    for exam_def in &file.exams {
        let exam = Exam {
            id: exam_def.id.clone(),
            plan_id: file.plan.id.clone(),
            subject_id: exam_def.subject_id.clone(),
            label: exam_def.label.trim().to_string(),
            date: time::parse_date(&exam_def.date).map_err(AppError::Validation)?,
            start_time: time::parse_time(&exam_def.start_time).map_err(AppError::Validation)?,
            session: exam_def.session.clone(),
            duration_minutes: 150,
        };
        let exists: Option<String> = tx
            .query_row("SELECT plan_id FROM exams WHERE id=?1", [&exam.id], |r| r.get(0))
            .optional()?;
        match exists {
            Some(owner) if owner != file.plan.id => {
                return Err(AppError::Validation(format!(
                    "Exam id '{}' is already used by another plan.",
                    exam.id
                )));
            }
            Some(_) => repo::update_exam(&tx, &exam)?,
            None => repo::insert_exam(&tx, &exam)?,
        }
        summary.exams += 1;
    }

    for (t_idx, template_def) in file.schedule_templates.iter().enumerate() {
        let existing: Option<String> = tx
            .query_row(
                "SELECT plan_id FROM schedule_templates WHERE id=?1",
                [&template_def.id],
                |r| r.get(0),
            )
            .optional()?;
        let template = ScheduleTemplate {
            id: template_def.id.clone(),
            plan_id: file.plan.id.clone(),
            name: template_def.name.trim().to_string(),
            day_types: template_def.day_types.clone(),
            slots: Vec::new(),
        };
        match existing {
            Some(owner) if owner != file.plan.id => {
                return Err(AppError::Validation(format!(
                    "Template id '{}' is already used by another plan.",
                    template.id
                )));
            }
            Some(_) => repo::update_template(&tx, &template)?,
            None => repo::insert_template(&tx, &template, t_idx as i64)?,
        }
        summary.templates += 1;
        for (sl_idx, slot_def) in template_def.slots.iter().enumerate() {
            let slot = TemplateSlot {
                id: slot_def.id.clone(),
                template_id: template.id.clone(),
                start_time: time::parse_time(&slot_def.start_time).map_err(AppError::Validation)?,
                end_time: time::parse_time(&slot_def.end_time).map_err(AppError::Validation)?,
                kind: slot_def.kind,
                title: if slot_def.title.trim().is_empty() {
                    slot_def.kind.label().to_string()
                } else {
                    slot_def.title.trim().to_string()
                },
                subject_id: slot_def.subject_id.clone(),
                chapter_id: slot_def.chapter_id.clone(),
            };
            let exists: Option<String> = tx
                .query_row(
                    "SELECT template_id FROM template_slots WHERE id=?1",
                    [&slot.id],
                    |r| r.get(0),
                )
                .optional()?;
            match exists {
                Some(owner) if owner != template.id => {
                    return Err(AppError::Validation(format!(
                        "Slot id '{}' belongs to a different template.",
                        slot.id
                    )));
                }
                Some(_) => repo::update_slot(&tx, &slot)?,
                None => repo::insert_slot(&tx, &slot, sl_idx as i64)?,
            }
            summary.slots += 1;
        }
    }

    for task_def in &file.tasks {
        match upsert_task(&tx, file, task_def)? {
            TaskUpsert::Inserted => summary.tasks_added += 1,
            TaskUpsert::Updated => summary.tasks_updated += 1,
            TaskUpsert::Preserved => summary.tasks_preserved += 1,
        }
    }

    if let Some(policy) = &file.revision_policy {
        if !policy.interval_days.is_empty() {
            let mut intervals: Vec<i64> = policy.interval_days.clone();
            intervals.retain(|v| *v > 0);
            intervals.sort_unstable();
            intervals.dedup();
            repo::put_setting(
                &tx,
                "revision_intervals",
                &serde_json::to_string(&intervals)?,
            )?;
        }
    }

    tx.commit()?;
    Ok(summary)
}

enum TaskUpsert {
    Inserted,
    Updated,
    Preserved,
}

fn upsert_task(conn: &Connection, file: &LockinPlanFile, def: &TaskDef) -> AppResult<TaskUpsert> {
    let date = time::parse_date(&def.date).map_err(AppError::Validation)?;
    let start = time::parse_time(&def.start_time).map_err(AppError::Validation)?;
    let end = time::parse_time(&def.end_time).map_err(AppError::Validation)?;
    let make = |id: String| Task {
        id,
        plan_id: file.plan.id.clone(),
        date,
        title: def.title.trim().to_string(),
        task_type: def.task_type,
        subject_id: def.subject_id.clone(),
        chapter_id: def.chapter_id.clone(),
        start_time: start,
        end_time: end,
        estimated_minutes: def.estimated_minutes,
        priority: def.priority.clamp(1, 3),
        status: def.status.unwrap_or(TaskStatus::Planned),
        completed_at: None,
        notes: String::new(),
        source_template_id: None,
        created_at: Utc::now().to_rfc3339(),
    };

    // Primary match: stable id.
    if let Some(existing) = conn
        .query_row(
            "SELECT id,plan_id,status FROM tasks WHERE id=?1",
            [&def.id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?
    {
        if existing.1 != file.plan.id {
            return Err(AppError::Validation(format!(
                "Task id '{}' is already used by another plan.",
                def.id
            )));
        }
        let status = TaskStatus::parse_or(&existing.2, TaskStatus::Planned);
        if scheduling::is_protected(status) {
            return Ok(TaskUpsert::Preserved);
        }
        let mut task = make(existing.0);
        let current = repo::get_task(conn, &task.id)?;
        task.created_at = current.created_at;
        repo::update_task(conn, &task)?;
        return Ok(TaskUpsert::Updated);
    }

    // Secondary match prevents duplicate planned rows when a generator produced
    // fresh ids for an identical block.
    let duplicate: Option<(String, String)> = conn
        .query_row(
            "SELECT id,status FROM tasks WHERE plan_id=?1 AND date=?2 AND start_time=?3 AND title=?4 AND status!='skipped' LIMIT 1",
            params![
                file.plan.id,
                date.to_string(),
                time::fmt_time(start),
                def.title.trim()
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((id, status_text)) = duplicate {
        let status = TaskStatus::parse_or(&status_text, TaskStatus::Planned);
        if scheduling::is_protected(status) {
            return Ok(TaskUpsert::Preserved);
        }
        let mut task = make(id.clone());
        let current = repo::get_task(conn, &id)?;
        task.created_at = current.created_at;
        repo::update_task(conn, &task)?;
        return Ok(TaskUpsert::Updated);
    }

    repo::insert_task(conn, &make(def.id.clone()))?;
    Ok(TaskUpsert::Inserted)
}

fn normalize_color(color: &str) -> String {
    if color.starts_with('#') && color.len() == 7 {
        color.to_string()
    } else {
        "#9B7BFF".into()
    }
}

fn delete_future_planned_tasks(conn: &Connection, plan_id: &str, today: NaiveDate) -> AppResult<i64> {
    let deleted = conn.execute(
        "DELETE FROM tasks WHERE plan_id=?1 AND date>=?2 AND status='planned'",
        params![plan_id, today.to_string()],
    )?;
    Ok(deleted as i64)
}

/// Delete configuration entries that are no longer present, but only when they
/// have no study history attached.
fn remove_unlisted_configuration(conn: &Connection, file: &LockinPlanFile) -> AppResult<()> {
    let imported_subjects: std::collections::HashSet<&str> =
        file.subjects.iter().map(|s| s.id.as_str()).collect();
    let imported_chapters: std::collections::HashSet<&str> = file
        .subjects
        .iter()
        .flat_map(|s| s.chapters.iter().map(|c| c.id.as_str()))
        .collect();

    let existing_chapters = repo::chapters_for_plan(conn, &file.plan.id)?;
    for chapter in existing_chapters {
        if imported_chapters.contains(chapter.id.as_str()) {
            continue;
        }
        let history: i64 = conn.query_row(
            "SELECT (SELECT COUNT(*) FROM study_sessions WHERE chapter_id=?1)
                  + (SELECT COUNT(*) FROM revisions WHERE chapter_id=?1)
                  + (SELECT COUNT(*) FROM mistakes WHERE chapter_id=?1)
                  + (SELECT COUNT(*) FROM tasks WHERE chapter_id=?1)",
            [&chapter.id],
            |r| r.get(0),
        )?;
        if history == 0 {
            repo::delete_chapter(conn, &chapter.id)?;
        }
    }

    let existing_subjects = repo::subjects_for_plan(conn, &file.plan.id)?;
    for subject in existing_subjects {
        if imported_subjects.contains(subject.id.as_str()) {
            continue;
        }
        let history: i64 = conn.query_row(
            "SELECT (SELECT COUNT(*) FROM study_sessions WHERE subject_id=?1)
                  + (SELECT COUNT(*) FROM mock_tests WHERE subject_id=?1)
                  + (SELECT COUNT(*) FROM mistakes WHERE subject_id=?1)
                  + (SELECT COUNT(*) FROM chapters WHERE subject_id=?1)",
            [&subject.id],
            |r| r.get(0),
        )?;
        if history == 0 {
            repo::delete_subject(conn, &subject.id)?;
        }
    }
    Ok(())
}
