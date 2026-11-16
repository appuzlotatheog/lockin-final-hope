//! Plan creation wizard draft + transactional commit.
//!
//! The draft is plain editable strings so the UI can bind text fields directly.
//! Validation never mutates the database; `commit` is a single transaction and
//! then materialises planned tasks for the date range.

use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::models::*;
use chrono::NaiveDate;
use rusqlite::Connection;
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct WizardChapter {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct WizardSubject {
    pub id: String,
    pub name: String,
    pub color: String,
    pub chapters: Vec<WizardChapter>,
}

#[derive(Debug, Clone)]
pub struct WizardExam {
    pub id: String,
    pub subject_id: String,
    pub label: String,
    pub date: String,
    pub start_time: String,
    pub session: String,
}

#[derive(Debug, Clone)]
pub struct WizardSlot {
    pub id: String,
    pub start_time: String,
    pub end_time: String,
    pub kind: SlotKind,
    pub title: String,
    pub subject_id: String,
}

#[derive(Debug, Clone)]
pub struct WizardTemplate {
    pub id: String,
    pub name: String,
    pub day_types: Vec<DayType>,
    pub slots: Vec<WizardSlot>,
}

#[derive(Debug, Clone)]
pub struct WizardDraft {
    pub name: String,
    pub academic_year: String,
    pub timezone: String,
    pub start_date: String,
    pub end_date: String,
    pub subjects: Vec<WizardSubject>,
    pub exams: Vec<WizardExam>,
    pub templates: Vec<WizardTemplate>,
    pub generate_tasks: bool,
}

pub const SUBJECT_COLORS: &[&str] = &[
    "#9B7BFF", "#69D6A6", "#EABF73", "#7FB2E6", "#E67F90", "#C6A6FF", "#F0A6BE", "#8FD3C1",
];

impl WizardDraft {
    pub fn from_bundle(bundle: &PlanBundle) -> Self {
        let subjects = bundle
            .subjects
            .iter()
            .map(|(s, chapters)| WizardSubject {
                id: s.id.clone(),
                name: s.name.clone(),
                color: s.color.clone(),
                chapters: chapters
                    .iter()
                    .map(|c| WizardChapter {
                        id: c.id.clone(),
                        title: c.title.clone(),
                    })
                    .collect(),
            })
            .collect();
        let exams = bundle
            .exams
            .iter()
            .map(|e| WizardExam {
                id: e.id.clone(),
                subject_id: e.subject_id.clone(),
                label: e.label.clone(),
                date: e.date.to_string(),
                start_time: time::fmt_time(e.start_time),
                session: e.session.clone(),
            })
            .collect();
        let templates = bundle
            .templates
            .iter()
            .map(|t| WizardTemplate {
                id: t.id.clone(),
                name: t.name.clone(),
                day_types: t.day_types.clone(),
                slots: t
                    .slots
                    .iter()
                    .map(|s| WizardSlot {
                        id: s.id.clone(),
                        start_time: time::fmt_time(s.start_time),
                        end_time: time::fmt_time(s.end_time),
                        kind: s.kind,
                        title: s.title.clone(),
                        subject_id: s.subject_id.clone().unwrap_or_default(),
                    })
                    .collect(),
            })
            .collect();
        Self {
            name: format!("{} (new season)", bundle.plan.name),
            academic_year: bundle.plan.academic_year.clone(),
            timezone: bundle.plan.timezone.clone(),
            start_date: bundle.plan.start_date.to_string(),
            end_date: bundle.plan.end_date.to_string(),
            subjects,
            exams,
            templates,
            generate_tasks: true,
        }
    }

    pub fn blank() -> Self {
        let (start, end) = crate::db::seed::default_window();
        Self {
            name: String::new(),
            academic_year: String::new(),
            timezone: time::DEFAULT_TZ.to_string(),
            start_date: start.to_string(),
            end_date: end.to_string(),
            subjects: Vec::new(),
            exams: Vec::new(),
            templates: vec![WizardTemplate {
                id: Uuid::new_v4().to_string(),
                name: "School day".into(),
                day_types: vec![DayType::Weekday],
                slots: Vec::new(),
            }],
            generate_tasks: true,
        }
    }

    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.name.trim().is_empty() {
            errors.push("Plan name is required.".into());
        }
        let start = time::parse_date(&self.start_date);
        let end = time::parse_date(&self.end_date);
        match (&start, &end) {
            (Ok(s), Ok(e)) if e < s => errors.push("End date must be on or after the start date.".into()),
            (Err(e), _) => errors.push(format!("Start date: {e}")),
            (_, Err(e)) => errors.push(format!("End date: {e}")),
            _ => {}
        }
        if time::parse_tz(&self.timezone).name() != self.timezone.trim()
            && self.timezone.trim() != time::DEFAULT_TZ
        {
            // An invalid IANA name silently falls back; make that visible.
            if self.timezone.trim().parse::<chrono_tz::Tz>().is_err() {
                errors.push(format!(
                    "Timezone '{}' is not a known IANA timezone (for example Asia/Kolkata).",
                    self.timezone
                ));
            }
        }
        if self.subjects.is_empty() {
            errors.push("Add at least one subject.".into());
        }
        let mut subject_names = HashSet::new();
        for subject in &self.subjects {
            if subject.name.trim().is_empty() {
                errors.push("Every subject needs a name.".into());
            } else if !subject_names.insert(subject.name.trim().to_lowercase()) {
                errors.push(format!("Duplicate subject name: {}.", subject.name));
            }
            for chapter in &subject.chapters {
                if chapter.title.trim().is_empty() {
                    errors.push(format!("{} has a chapter without a title.", subject.name));
                }
            }
        }
        for exam in &self.exams {
            if exam.label.trim().is_empty() {
                errors.push("Every exam needs a label.".into());
            }
            if !self.subjects.iter().any(|s| s.id == exam.subject_id) {
                errors.push(format!("Exam '{}' is not linked to a subject.", exam.label));
            }
            if time::parse_date(&exam.date).is_err() {
                errors.push(format!("Exam '{}' has an invalid date.", exam.label));
            }
            if time::parse_time(&exam.start_time).is_err() {
                errors.push(format!("Exam '{}' has an invalid start time.", exam.label));
            }
        }
        for template in &self.templates {
            if template.day_types.is_empty() {
                errors.push(format!("Template '{}' needs at least one day type.", template.name));
            }
            for slot in &template.slots {
                match (
                    time::parse_time(&slot.start_time),
                    time::parse_time(&slot.end_time),
                ) {
                    (Ok(a), Ok(b)) => {
                        if b <= a {
                            errors.push(format!(
                                "'{}' in {} ends before it starts ({} – {}).",
                                slot.title, template.name, slot.start_time, slot.end_time
                            ));
                        }
                    }
                    _ => errors.push(format!(
                        "'{}' in {} has an invalid time.",
                        slot.title, template.name
                    )),
                }
                if slot.kind != SlotKind::Break
                    && slot.kind != SlotKind::Meal
                    && slot.title.trim().is_empty()
                {
                    errors.push(format!("A study slot in {} needs a title.", template.name));
                }
            }
        }
        errors
    }

    pub fn plan_id(&self) -> String {
        let mut slug: String = self
            .name
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        while slug.contains("--") {
            slug = slug.replace("--", "-");
        }
        let slug = slug.trim_matches('-');
        let slug = if slug.is_empty() { "exam-plan" } else { slug };
        format!("{slug}-{}", &Uuid::new_v4().to_string()[..8])
    }

    /// Validated typed entities, ready for insertion.
    pub fn build(&self, plan_id: &str) -> AppResult<(Plan, Vec<(Subject, Vec<Chapter>)>, Vec<Exam>, Vec<ScheduleTemplate>)> {
        let errors = self.validate();
        if !errors.is_empty() {
            return Err(AppError::Validation(errors.join("\n")));
        }
        let plan = Plan {
            id: plan_id.to_string(),
            name: self.name.trim().to_string(),
            academic_year: self.academic_year.trim().to_string(),
            timezone: self.timezone.trim().to_string(),
            start_date: time::parse_date(&self.start_date).map_err(AppError::Validation)?,
            end_date: time::parse_date(&self.end_date).map_err(AppError::Validation)?,
            archived: false,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let mut subject_map = std::collections::HashMap::new();
        let mut subjects = Vec::new();
        for (idx, s) in self.subjects.iter().enumerate() {
            let new_id = Uuid::new_v4().to_string();
            subject_map.insert(s.id.clone(), new_id.clone());
            let subject = Subject {
                id: new_id,
                plan_id: plan.id.clone(),
                name: s.name.trim().to_string(),
                color: s.color.clone(),
                sort_order: idx as i64,
                portions_pending: s.chapters.is_empty(),
            };
            let mut chapters = Vec::new();
            for (c_idx, c) in s.chapters.iter().enumerate() {
                chapters.push(Chapter {
                    id: Uuid::new_v4().to_string(),
                    subject_id: subject.id.clone(),
                    title: c.title.trim().to_string(),
                    sort_order: c_idx as i64,
                    status: ChapterStatus::NotStarted,
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
                });
            }
            subjects.push((subject, chapters));
        }
        let exams = self
            .exams
            .iter()
            .filter_map(|e| {
                Some(Exam {
                    id: Uuid::new_v4().to_string(),
                    plan_id: plan.id.clone(),
                    subject_id: subject_map.get(&e.subject_id)?.clone(),
                    label: e.label.trim().to_string(),
                    date: time::parse_date(&e.date).ok()?,
                    start_time: time::parse_time(&e.start_time).ok()?,
                    session: e.session.clone(),
                    duration_minutes: 150,
                })
            })
            .collect();
        let templates = self
            .templates
            .iter()
            .enumerate()
            .map(|(t_idx, t)| {
                let template_id = Uuid::new_v4().to_string();
                let slots = t
                    .slots
                    .iter()
                    .filter_map(|s| {
                        Some(TemplateSlot {
                            id: Uuid::new_v4().to_string(),
                            template_id: template_id.clone(),
                            start_time: time::parse_time(&s.start_time).ok()?,
                            end_time: time::parse_time(&s.end_time).ok()?,
                            kind: s.kind,
                            title: if s.title.trim().is_empty() {
                                s.kind.label().to_string()
                            } else {
                                s.title.trim().to_string()
                            },
                            subject_id: subject_map.get(&s.subject_id).cloned(),
                            chapter_id: None,
                        })
                    })
                    .collect();
                let _ = t_idx;
                ScheduleTemplate {
                    id: template_id,
                    plan_id: plan.id.clone(),
                    name: t.name.clone(),
                    day_types: t.day_types.clone(),
                    slots,
                }
            })
            .collect();
        Ok((plan, subjects, exams, templates))
    }

    /// Insert everything inside one transaction, then generate planned tasks.
    pub fn commit(&self, conn: &mut Connection) -> AppResult<String> {
        let plan_id = self.plan_id();
        let (plan, subjects, exams, templates) = self.build(&plan_id)?;
        let tx = conn.transaction()?;
        repo::insert_plan(&tx, &plan)?;
        for (subject, chapters) in &subjects {
            repo::insert_subject(&tx, subject)?;
            for chapter in chapters {
                repo::insert_chapter(&tx, chapter)?;
            }
        }
        for exam in &exams {
            repo::insert_exam(&tx, exam)?;
        }
        for (idx, template) in templates.iter().enumerate() {
            repo::insert_template(&tx, template, idx as i64)?;
            for (s_idx, slot) in template.slots.iter().enumerate() {
                repo::insert_slot(&tx, slot, s_idx as i64)?;
            }
        }
        tx.commit()?;
        if self.generate_tasks {
            crate::services::plan::generate_tasks_for_range(
                conn,
                &plan.id,
                plan.start_date,
                plan.end_date,
            )?;
        }
        Ok(plan.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_draft() {
        let draft = WizardDraft {
            name: "".into(),
            academic_year: "".into(),
            timezone: "Mars/Olympus".into(),
            start_date: "2026-12-20".into(),
            end_date: "2026-12-01".into(),
            subjects: vec![],
            exams: vec![],
            templates: vec![],
            generate_tasks: false,
        };
        let errors = draft.validate();
        assert!(errors.iter().any(|e| e.contains("name")));
        assert!(errors.iter().any(|e| e.contains("End date")));
        assert!(errors.iter().any(|e| e.contains("timezone")));
    }

    #[test]
    fn creates_a_working_plan() {
        let dir = std::env::temp_dir().join(format!("lockin-wizard-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = crate::db::Database::open(&dir.join("t.db")).unwrap();
        db.migrate().unwrap();
        let draft = WizardDraft {
            name: "Midterm Plan".into(),
            academic_year: "2026".into(),
            timezone: "Asia/Kolkata".into(),
            start_date: "2026-11-16".into(),
            end_date: "2026-11-22".into(),
            subjects: vec![WizardSubject {
                id: "s1".into(),
                name: "Physics".into(),
                color: "#9B7BFF".into(),
                chapters: vec![WizardChapter {
                    id: "c1".into(),
                    title: "Units".into(),
                }],
            }],
            exams: vec![],
            templates: vec![WizardTemplate {
                id: "t1".into(),
                name: "School day".into(),
                day_types: vec![DayType::Weekday],
                slots: vec![WizardSlot {
                    id: "sl1".into(),
                    start_time: "19:00".into(),
                    end_time: "20:00".into(),
                    kind: SlotKind::Study,
                    title: "Physics".into(),
                    subject_id: "s1".into(),
                }],
            }],
            generate_tasks: true,
        };
        let id = draft.commit(&mut db.conn).unwrap();
        let tasks = repo::tasks_for_range(
            &db.conn,
            &id,
            NaiveDate::from_ymd_opt(2026, 11, 16).unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 22).unwrap(),
        )
        .unwrap();
        assert_eq!(tasks.len(), 5); // Monday–Friday, one block each
        let _ = std::fs::remove_dir_all(dir);
    }
}
