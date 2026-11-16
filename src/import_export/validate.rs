//! Validation + preview for Lockin Plan Format imports.
//!
//! Hard problems (bad JSON, wrong format, unsupported schema version, oversized
//! file) are returned as errors. Everything else is collected as issues with a
//! JSON path so the user can fix the file.

use super::format::*;
use super::MAX_IMPORT_BYTES;
use crate::error::{AppError, AppResult};
use crate::models::*;
use crate::scheduling;
use chrono::{NaiveDate, NaiveTime};
use rusqlite::{Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct ImportIssue {
    pub path: String,
    pub message: String,
    pub severity: Severity,
}

impl ImportIssue {
    fn error(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
            severity: Severity::Error,
        }
    }
    fn warning(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
            severity: Severity::Warning,
        }
    }
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
    pub fn display(&self) -> String {
        format!(
            "{} {}: {}",
            if self.is_error() { "Error" } else { "Warning" },
            self.path,
            self.message
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct ImportPreview {
    pub plan_id: String,
    pub plan_name: String,
    pub plan_exists: bool,
    pub existing_plan_name: String,
    pub date_span: String,
    pub timezone: String,
    pub subjects_total: i64,
    pub chapters_total: i64,
    pub exams_total: i64,
    pub templates_total: i64,
    pub slots_total: i64,
    pub tasks_total: i64,
    pub subjects_new: i64,
    pub subjects_updated: i64,
    pub chapters_new: i64,
    pub chapters_updated: i64,
    pub exams_new: i64,
    pub exams_updated: i64,
    pub templates_new: i64,
    pub templates_updated: i64,
    pub slots_new: i64,
    pub slots_updated: i64,
    pub tasks_new: i64,
    pub tasks_updated: i64,
    pub tasks_protected: i64,
    pub errors: i64,
    pub warnings: i64,
    pub summary: String,
}

#[derive(Debug, Clone)]
pub struct PreparedPlan {
    pub file: LockinPlanFile,
    pub issues: Vec<ImportIssue>,
    pub preview: ImportPreview,
}

impl PreparedPlan {
    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|i| i.is_error())
    }
    pub fn error_text(&self) -> String {
        let errors: Vec<String> = self
            .issues
            .iter()
            .filter(|i| i.is_error())
            .map(|i| i.display())
            .collect();
        if errors.is_empty() {
            "The file is valid.".into()
        } else {
            errors.join("\n")
        }
    }
}

fn table_has(conn: &Connection, sql: &str, id: &str) -> AppResult<bool> {
    let count: i64 = conn.query_row(sql, [id], |r| r.get(0))?;
    Ok(count > 0)
}

fn existing_task(
    conn: &Connection,
    id: &str,
) -> AppResult<Option<(String, TaskStatus, NaiveDate)>> {
    Ok(conn
        .query_row(
            "SELECT plan_id, status, date FROM tasks WHERE id=?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    TaskStatus::parse_or(&r.get::<_, String>(1)?, TaskStatus::Planned),
                    crate::models::time::parse_date(&r.get::<_, String>(2)?).unwrap_or_default(),
                ))
            },
        )
        .optional()?)
}

pub fn prepare(conn: &Connection, raw: &str) -> AppResult<PreparedPlan> {
    if raw.trim().is_empty() {
        return Err(AppError::validation("The editor is empty. Paste or open a plan JSON file first."));
    }
    if raw.len() > MAX_IMPORT_BYTES {
        return Err(AppError::validation(format!(
            "Import file is too large ({} KB). The limit is 5 MB.",
            raw.len() / 1024
        )));
    }
    let value: serde_json::Value = serde_json::from_str(raw)?;
    let format = value.get("format").and_then(|v| v.as_str()).unwrap_or("");
    if format != FORMAT_ID {
        return Err(AppError::validation(format!(
            "This is not a Lockin Plan file: expected \"format\": \"lockin-plan\", found \"{format}\". For a full backup use Restore Backup instead."
        )));
    }
    let version = value
        .get("schema_version")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if version > SCHEMA_VERSION {
        return Err(AppError::validation(format!(
            "This file uses schema version {version}, but this app supports up to version {SCHEMA_VERSION}. Update the app or downgrade the file."
        )));
    }
    if version < 1 {
        return Err(AppError::validation(
            "schema_version must be at least 1 (usually 1).",
        ));
    }
    let file: LockinPlanFile = serde_json::from_value(value)?;
    let mut issues = validate_semantics(&file);

    let preview = build_preview(conn, &file, &mut issues)?;
    Ok(PreparedPlan {
        file,
        issues,
        preview,
    })
}

pub fn validate_semantics(file: &LockinPlanFile) -> Vec<ImportIssue> {
    let mut issues = Vec::new();
    let plan = &file.plan;

    if plan.id.trim().is_empty() {
        issues.push(ImportIssue::error("plan.id", "Plan id is required."));
    } else if !plan
        .id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        issues.push(ImportIssue::error(
            "plan.id",
            "Use only letters, digits, dots, dashes and underscores in the plan id.",
        ));
    }
    if plan.name.trim().is_empty() {
        issues.push(ImportIssue::error("plan.name", "Plan name is required."));
    }
    let start = crate::models::time::parse_date(&plan.start_date);
    let end = crate::models::time::parse_date(&plan.end_date);
    if let Err(e) = &start {
        issues.push(ImportIssue::error("plan.start_date", e.clone()));
    }
    if let Err(e) = &end {
        issues.push(ImportIssue::error("plan.end_date", e.clone()));
    }
    if let (Ok(s), Ok(e)) = (&start, &end) {
        if e < s {
            issues.push(ImportIssue::error(
                "plan.end_date",
                "End date must be on or after the start date.",
            ));
        }
        if (*e - *s).num_days() > 730 {
            issues.push(ImportIssue::warning(
                "plan",
                "This plan spans more than two years. Double-check the dates.",
            ));
        }
    }
    if plan.timezone.parse::<chrono_tz::Tz>().is_err() {
        issues.push(ImportIssue::error(
            "plan.timezone",
            format!(
                "'{}' is not a known IANA timezone. Use e.g. Asia/Kolkata.",
                plan.timezone
            ),
        ));
    }

    // Duplicate IDs.
    for (path, id) in file.duplicate_ids() {
        issues.push(ImportIssue::error(
            path,
            format!("Duplicate id '{id}'. Every plan, subject, chapter, exam, template, slot and task needs a unique id."),
        ));
    }

    // Remote references are never fetched. Reject them clearly.
    for (path, value) in string_fields(file) {
        let v = value.trim().to_ascii_lowercase();
        if v.starts_with("http://") || v.starts_with("https://") || v.starts_with("file://") {
            issues.push(ImportIssue::error(
                path,
                "Remote/file references are not supported. Paste the actual text or use a local file.",
            ));
        }
    }

    let mut subject_ids = HashSet::new();
    let mut chapter_parent: HashMap<String, String> = HashMap::new();
    for (i, subject) in file.subjects.iter().enumerate() {
        let path = format!("subjects[{i}]");
        if subject.id.trim().is_empty() {
            issues.push(ImportIssue::error(format!("{path}.id"), "Subject id is required."));
        }
        if subject.name.trim().is_empty() {
            issues.push(ImportIssue::error(format!("{path}.name"), "Subject name is required."));
        }
        if !subject.color.starts_with('#') || subject.color.len() != 7 {
            issues.push(ImportIssue::warning(
                format!("{path}.color"),
                format!("'{}' is not a #RRGGBB colour; a default purple will be used.", subject.color),
            ));
        }
        subject_ids.insert(subject.id.clone());
        for (j, chapter) in subject.chapters.iter().enumerate() {
            let cpath = format!("{path}.chapters[{j}]");
            if chapter.id.trim().is_empty() {
                issues.push(ImportIssue::error(format!("{cpath}.id"), "Chapter id is required."));
            }
            if chapter.title.trim().is_empty() {
                issues.push(ImportIssue::error(format!("{cpath}.title"), "Chapter title is required."));
            }
            if let Some(prev) = chapter_parent.insert(chapter.id.clone(), subject.id.clone()) {
                if prev != subject.id {
                    issues.push(ImportIssue::error(
                        format!("{cpath}.id"),
                        "This chapter id already belongs to another subject.",
                    ));
                }
            }
        }
    }
    if file.subjects.is_empty() {
        issues.push(ImportIssue::warning(
            "subjects",
            "No subjects are included. The plan will have an empty syllabus.",
        ));
    }

    for (i, exam) in file.exams.iter().enumerate() {
        let path = format!("exams[{i}]");
        if exam.label.trim().is_empty() {
            issues.push(ImportIssue::error(format!("{path}.label"), "Exam label is required."));
        }
        if !subject_ids.contains(&exam.subject_id) {
            issues.push(ImportIssue::error(
                format!("{path}.subject_id"),
                format!("Unknown subject id '{}'.", exam.subject_id),
            ));
        }
        if let Err(e) = crate::models::time::parse_date(&exam.date) {
            issues.push(ImportIssue::error(format!("{path}.date"), e));
        }
        if let Err(e) = crate::models::time::parse_time(&exam.start_time) {
            issues.push(ImportIssue::error(format!("{path}.start_time"), e));
        }
    }

    let mut template_day_types = HashSet::new();
    for (i, template) in file.schedule_templates.iter().enumerate() {
        let path = format!("schedule_templates[{i}]");
        if template.name.trim().is_empty() {
            issues.push(ImportIssue::error(format!("{path}.name"), "Template name is required."));
        }
        if template.day_types.is_empty() {
            issues.push(ImportIssue::error(
                format!("{path}.day_types"),
                "A template needs at least one day type.",
            ));
        }
        for day in &template.day_types {
            if !template_day_types.insert(*day) {
                issues.push(ImportIssue::warning(
                    format!("{path}.day_types"),
                    format!("More than one template applies to '{}'. The first one wins.", day.as_str()),
                ));
            }
        }
        for (j, slot) in template.slots.iter().enumerate() {
            let spath = format!("{path}.slots[{j}]");
            match (
                crate::models::time::parse_time(&slot.start_time),
                crate::models::time::parse_time(&slot.end_time),
            ) {
                (Ok(a), Ok(b)) => {
                    if b <= a {
                        issues.push(ImportIssue::error(
                            format!("{spath}.end_time"),
                            "End time must be after the start time.",
                        ));
                    }
                }
                (Err(e), _) => issues.push(ImportIssue::error(format!("{spath}.start_time"), e)),
                (_, Err(e)) => issues.push(ImportIssue::error(format!("{spath}.end_time"), e)),
            }
            if let Some(sid) = &slot.subject_id {
                if !subject_ids.contains(sid) {
                    issues.push(ImportIssue::error(
                        format!("{spath}.subject_id"),
                        format!("Unknown subject id '{sid}'."),
                    ));
                }
            }
            if let Some(cid) = &slot.chapter_id {
                if !chapter_parent.contains_key(cid) {
                    issues.push(ImportIssue::error(
                        format!("{spath}.chapter_id"),
                        format!("Unknown chapter id '{cid}'."),
                    ));
                }
            }
        }
        // Overlap detection within this template.
        let mut sorted: Vec<(usize, NaiveTime, NaiveTime)> = template
            .slots
            .iter()
            .enumerate()
            .filter_map(|(j, slot)| {
                Some((
                    j,
                    crate::models::time::parse_time(&slot.start_time).ok()?,
                    crate::models::time::parse_time(&slot.end_time).ok()?,
                ))
            })
            .collect();
        sorted.sort_by_key(|(_, s, _)| *s);
        for pair in sorted.windows(2) {
            if scheduling::overlaps(pair[0].1, pair[0].2, pair[1].1, pair[1].2) {
                issues.push(ImportIssue::warning(
                    format!("{path}.slots[{}]", pair[1].0),
                    format!(
                        "'{}' overlaps '{}'; the timetable will warn about this.",
                        template.slots[pair[1].0].title, template.slots[pair[0].0].title
                    ),
                ));
            }
        }
    }

    for (i, task) in file.tasks.iter().enumerate() {
        let path = format!("tasks[{i}]");
        if task.title.trim().is_empty() {
            issues.push(ImportIssue::error(format!("{path}.title"), "Task title is required."));
        }
        if let Err(e) = crate::models::time::parse_date(&task.date) {
            issues.push(ImportIssue::error(format!("{path}.date"), e));
        }
        match (
            crate::models::time::parse_time(&task.start_time),
            crate::models::time::parse_time(&task.end_time),
        ) {
            (Ok(a), Ok(b)) => {
                if b <= a {
                    issues.push(ImportIssue::error(
                        format!("{path}.end_time"),
                        "End time must be after the start time.",
                    ));
                } else if task.estimated_minutes > 0 {
                    let real = crate::models::time::minutes_between(a, b);
                    if (real - task.estimated_minutes).abs() > 15 {
                        issues.push(ImportIssue::warning(
                            format!("{path}.estimated_minutes"),
                            format!("Estimated {real} minutes fit the time block better than {}.", task.estimated_minutes),
                        ));
                    }
                }
            }
            (Err(e), _) => issues.push(ImportIssue::error(format!("{path}.start_time"), e)),
            (_, Err(e)) => issues.push(ImportIssue::error(format!("{path}.end_time"), e)),
        }
        if task.estimated_minutes <= 0 {
            issues.push(ImportIssue::error(
                format!("{path}.estimated_minutes"),
                "Estimated minutes must be greater than zero.",
            ));
        }
        if !(1..=3).contains(&task.priority) {
            issues.push(ImportIssue::warning(
                format!("{path}.priority"),
                "Priority is normally 1 (low), 2 (normal) or 3 (high).",
            ));
        }
        if let Some(sid) = &task.subject_id {
            if !subject_ids.contains(sid) {
                issues.push(ImportIssue::error(
                    format!("{path}.subject_id"),
                    format!("Unknown subject id '{sid}'."),
                ));
            }
        }
        if let Some(cid) = &task.chapter_id {
            if !chapter_parent.contains_key(cid) {
                issues.push(ImportIssue::error(
                    format!("{path}.chapter_id"),
                    format!("Unknown chapter id '{cid}'."),
                ));
            } else if task.subject_id.is_some() && task.subject_id.as_ref() != chapter_parent.get(cid) {
                issues.push(ImportIssue::warning(
                    format!("{path}.chapter_id"),
                    "This chapter belongs to a different subject than the task's subject.",
                ));
            }
        }
        if let (Ok(task_date), Ok(plan_start), Ok(plan_end)) = (
            crate::models::time::parse_date(&task.date),
            crate::models::time::parse_date(&plan.start_date),
            crate::models::time::parse_date(&plan.end_date),
        ) {
            if task_date < plan_start || task_date > plan_end {
                issues.push(ImportIssue::warning(
                    format!("{path}.date"),
                    "Task falls outside the plan's start/end dates.",
                ));
            }
        }
    }

    if let Some(policy) = &file.revision_policy {
        if policy.interval_days.iter().any(|d| *d <= 0) {
            issues.push(ImportIssue::error(
                "revision_policy.interval_days",
                "Revision intervals must be positive day counts.",
            ));
        }
        if policy.interval_days.len() > 10 {
            issues.push(ImportIssue::warning(
                "revision_policy.interval_days",
                "More than ten revision intervals is unusual; the first ten will be used.",
            ));
        }
    }

    issues
}

fn string_fields(file: &LockinPlanFile) -> Vec<(String, String)> {
    let mut out = vec![
        ("plan.id".into(), file.plan.id.clone()),
        ("plan.name".into(), file.plan.name.clone()),
        ("plan.timezone".into(), file.plan.timezone.clone()),
    ];
    for (i, s) in file.subjects.iter().enumerate() {
        out.push((format!("subjects[{i}].name"), s.name.clone()));
        for (j, c) in s.chapters.iter().enumerate() {
            out.push((format!("subjects[{i}].chapters[{j}].title"), c.title.clone()));
        }
    }
    for (i, e) in file.exams.iter().enumerate() {
        out.push((format!("exams[{i}].label"), e.label.clone()));
    }
    for (i, t) in file.schedule_templates.iter().enumerate() {
        out.push((format!("schedule_templates[{i}].name"), t.name.clone()));
        for (j, s) in t.slots.iter().enumerate() {
            out.push((format!("schedule_templates[{i}].slots[{j}].title"), s.title.clone()));
        }
    }
    for (i, t) in file.tasks.iter().enumerate() {
        out.push((format!("tasks[{i}].title"), t.title.clone()));
    }
    out
}

fn build_preview(
    conn: &Connection,
    file: &LockinPlanFile,
    issues: &mut Vec<ImportIssue>,
) -> AppResult<ImportPreview> {
    let mut p = ImportPreview {
        plan_id: file.plan.id.clone(),
        plan_name: file.plan.name.clone(),
        timezone: file.plan.timezone.clone(),
        date_span: format!("{} → {}", file.plan.start_date, file.plan.end_date),
        ..Default::default()
    };
    if let Ok(name) = conn.query_row(
        "SELECT name FROM plans WHERE id=?1",
        [&file.plan.id],
        |r| r.get::<_, String>(0),
    ) {
        p.plan_exists = true;
        p.existing_plan_name = name;
    }

    for subject in &file.subjects {
        p.subjects_total += 1;
        let exists: Option<String> = conn
            .query_row("SELECT plan_id FROM subjects WHERE id=?1", [&subject.id], |r| r.get(0))
            .optional()?;
        match exists {
            Some(plan_id) if plan_id != file.plan.id => {
                issues.push(ImportIssue::error(
                    format!("subjects.id '{}'", subject.id),
                    format!("This id already belongs to a subject in plan '{plan_id}'."),
                ));
            }
            Some(_) => p.subjects_updated += 1,
            None => p.subjects_new += 1,
        }
        for chapter in &subject.chapters {
            p.chapters_total += 1;
            let exists: Option<String> = conn
                .query_row("SELECT subject_id FROM chapters WHERE id=?1", [&chapter.id], |r| r.get(0))
                .optional()?;
            match exists {
                Some(subject_id) if subject_id != subject.id => {
                    issues.push(ImportIssue::error(
                        format!("chapters.id '{}'", chapter.id),
                        "Chapter id already exists under a different subject.",
                    ));
                }
                Some(_) => p.chapters_updated += 1,
                None => p.chapters_new += 1,
            }
        }
    }
    for exam in &file.exams {
        p.exams_total += 1;
        if table_has(conn, "SELECT COUNT(*) FROM exams WHERE id=?1", &exam.id)? {
            p.exams_updated += 1;
        } else {
            p.exams_new += 1;
        }
    }
    for template in &file.schedule_templates {
        p.templates_total += 1;
        if table_has(conn, "SELECT COUNT(*) FROM schedule_templates WHERE id=?1", &template.id)? {
            p.templates_updated += 1;
        } else {
            p.templates_new += 1;
        }
        for slot in &template.slots {
            p.slots_total += 1;
            if table_has(conn, "SELECT COUNT(*) FROM template_slots WHERE id=?1", &slot.id)? {
                p.slots_updated += 1;
            } else {
                p.slots_new += 1;
            }
        }
    }
    for task in &file.tasks {
        p.tasks_total += 1;
        if let Some((_, status, _)) = existing_task(conn, &task.id)? {
            if scheduling::is_protected(status) {
                p.tasks_protected += 1;
            } else {
                p.tasks_updated += 1;
            }
        } else {
            p.tasks_new += 1;
        }
    }

    p.errors = issues.iter().filter(|i| i.is_error()).count() as i64;
    p.warnings = issues.iter().filter(|i| !i.is_error()).count() as i64;
    p.summary = format!(
        "{} subjects ({} new, {} updated) · {} chapters ({} new, {} updated) · {} exams · {} templates / {} slots · {} tasks ({} new, {} updated, {} protected)",
        p.subjects_total,
        p.subjects_new,
        p.subjects_updated,
        p.chapters_total,
        p.chapters_new,
        p.chapters_updated,
        p.exams_total,
        p.templates_total,
        p.slots_total,
        p.tasks_total,
        p.tasks_new,
        p.tasks_updated,
        p.tasks_protected
    );
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> LockinPlanFile {
        LockinPlanFile::empty_template()
    }

    #[test]
    fn future_schema_is_rejected() {
        let dir = std::env::temp_dir().join(format!("lockin-validate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::Database::open(&dir.join("t.db")).unwrap();
        let mut file = base();
        file.schema_version = 2;
        let raw = serde_json::to_string(&file).unwrap();
        let err = prepare(&db.conn, &raw).unwrap_err();
        assert!(err.to_string().contains("schema version 2"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn semantic_errors_have_paths() {
        let mut file = base();
        file.plan.start_date = "not-a-date".into();
        file.subjects.push(SubjectDef {
            id: "s1".into(),
            name: "Physics".into(),
            color: "#9B7BFF".into(),
            sort_order: 0,
            chapters: vec![ChapterDef {
                id: "c1".into(),
                title: "Units".into(),
                order: 0,
                status: None,
            }],
        });
        file.exams.push(ExamDef {
            id: "e1".into(),
            subject_id: "missing".into(),
            label: "Physics".into(),
            date: "2026-12-10".into(),
            start_time: "13:30".into(),
            session: "Afternoon".into(),
        });
        let issues = validate_semantics(&file);
        assert!(issues.iter().any(|i| i.path == "plan.start_date"));
        assert!(issues.iter().any(|i| i.path == "exams[0].subject_id"));
    }

    #[test]
    fn remote_references_are_rejected() {
        let mut file = base();
        file.subjects.push(SubjectDef {
            id: "s1".into(),
            name: "https://example.com/syllabus".into(),
            color: "#9B7BFF".into(),
            sort_order: 0,
            chapters: vec![],
        });
        let issues = validate_semantics(&file);
        assert!(issues.iter().any(|i| i.message.contains("Remote")));
    }

    #[test]
    fn overlapping_slots_warn() {
        let mut file = base();
        file.schedule_templates.push(TemplateDef {
            id: "t1".into(),
            name: "Day".into(),
            day_types: vec![DayType::Weekday],
            slots: vec![
                SlotDef {
                    id: "sl1".into(),
                    start_time: "19:00".into(),
                    end_time: "20:00".into(),
                    kind: SlotKind::Study,
                    title: "Physics".into(),
                    subject_id: None,
                    chapter_id: None,
                },
                SlotDef {
                    id: "sl2".into(),
                    start_time: "19:30".into(),
                    end_time: "20:30".into(),
                    kind: SlotKind::Study,
                    title: "Chemistry".into(),
                    subject_id: None,
                    chapter_id: None,
                },
            ],
        });
        let issues = validate_semantics(&file);
        assert!(issues.iter().any(|i| i.message.contains("overlaps")));
    }
}
