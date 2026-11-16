//! Typed Lockin Plan Format + export construction.

use crate::db::repo;
use crate::error::AppResult;
use crate::models::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

pub const FORMAT_ID: &str = "lockin-plan";
pub const SCHEMA_VERSION: i64 = 1;
pub const SCHEMA_JSON: &str = include_str!("../../schemas/lockin-plan.schema.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanHeader {
    pub id: String,
    pub name: String,
    #[serde(default = "default_tz")]
    pub timezone: String,
    pub start_date: String,
    pub end_date: String,
    #[serde(default)]
    pub academic_year: String,
}

fn default_tz() -> String {
    time::DEFAULT_TZ.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterDef {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub order: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ChapterStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubjectDef {
    pub id: String,
    pub name: String,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub sort_order: i64,
    #[serde(default)]
    pub chapters: Vec<ChapterDef>,
}

fn default_color() -> String {
    "#9B7BFF".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExamDef {
    pub id: String,
    pub subject_id: String,
    pub label: String,
    pub date: String,
    pub start_time: String,
    #[serde(default)]
    pub session: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotDef {
    pub id: String,
    pub start_time: String,
    pub end_time: String,
    #[serde(rename = "type")]
    pub kind: SlotKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateDef {
    pub id: String,
    pub name: String,
    pub day_types: Vec<DayType>,
    #[serde(default)]
    pub slots: Vec<SlotDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDef {
    pub id: String,
    pub date: String,
    pub title: String,
    pub task_type: TaskType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<String>,
    #[serde(default = "default_start")]
    pub start_time: String,
    #[serde(default = "default_end")]
    pub end_time: String,
    #[serde(default)]
    pub estimated_minutes: i64,
    #[serde(default = "default_priority")]
    pub priority: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,
}

fn default_start() -> String {
    "19:00".into()
}
fn default_end() -> String {
    "19:45".into()
}
fn default_priority() -> i64 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevisionPolicy {
    #[serde(default = "default_intervals")]
    pub interval_days: Vec<i64>,
    #[serde(default)]
    pub adapt_to_results: bool,
}

fn default_intervals() -> Vec<i64> {
    vec![1, 3, 7]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockinPlanFile {
    pub format: String,
    #[serde(default)]
    pub schema_version: i64,
    pub plan: PlanHeader,
    #[serde(default)]
    pub subjects: Vec<SubjectDef>,
    #[serde(default)]
    pub exams: Vec<ExamDef>,
    #[serde(default)]
    pub schedule_templates: Vec<TemplateDef>,
    #[serde(default)]
    pub tasks: Vec<TaskDef>,
    #[serde(default)]
    pub revision_policy: Option<RevisionPolicy>,
}

impl LockinPlanFile {
    pub fn empty_template() -> Self {
        Self {
            format: FORMAT_ID.into(),
            schema_version: SCHEMA_VERSION,
            plan: PlanHeader {
                id: "my-exam-plan-2027".into(),
                name: "My Exam Plan".into(),
                timezone: time::DEFAULT_TZ.into(),
                start_date: "2026-11-16".into(),
                end_date: "2026-12-17".into(),
                academic_year: "2026–27".into(),
            },
            subjects: Vec::new(),
            exams: Vec::new(),
            schedule_templates: Vec::new(),
            tasks: Vec::new(),
            revision_policy: Some(RevisionPolicy {
                interval_days: vec![1, 3, 7],
                adapt_to_results: true,
            }),
        }
    }

    pub fn all_ids(&self) -> Vec<(String, String)> {
        let mut ids = Vec::new();
        ids.push((format!("plan.id"), self.plan.id.clone()));
        for (i, s) in self.subjects.iter().enumerate() {
            ids.push((format!("subjects[{i}].id"), s.id.clone()));
            for (j, c) in s.chapters.iter().enumerate() {
                ids.push((format!("subjects[{i}].chapters[{j}].id"), c.id.clone()));
            }
        }
        for (i, e) in self.exams.iter().enumerate() {
            ids.push((format!("exams[{i}].id"), e.id.clone()));
        }
        for (i, t) in self.schedule_templates.iter().enumerate() {
            ids.push((format!("schedule_templates[{i}].id"), t.id.clone()));
            for (j, s) in t.slots.iter().enumerate() {
                ids.push((format!("schedule_templates[{i}].slots[{j}].id"), s.id.clone()));
            }
        }
        for (i, t) in self.tasks.iter().enumerate() {
            ids.push((format!("tasks[{i}].id"), t.id.clone()));
        }
        ids
    }

    pub fn duplicate_ids(&self) -> Vec<(String, String)> {
        let mut seen = HashSet::new();
        let mut dups = Vec::new();
        for (path, id) in self.all_ids() {
            if id.trim().is_empty() {
                continue;
            }
            if !seen.insert(id.clone()) {
                dups.push((path, id));
            }
        }
        dups
    }
}

/// Export the selected plan's configuration, plus tasks that are still part of
/// the plan (completed tasks and study history are only in a full backup).
pub fn export_plan(conn: &rusqlite::Connection, plan_id: &str) -> AppResult<LockinPlanFile> {
    let bundle = crate::services::plan::load_bundle(conn, plan_id)?;
    let mut subjects = Vec::new();
    for (subject, chapters) in &bundle.subjects {
        subjects.push(SubjectDef {
            id: subject.id.clone(),
            name: subject.name.clone(),
            color: subject.color.clone(),
            sort_order: subject.sort_order,
            chapters: chapters
                .iter()
                .map(|c| ChapterDef {
                    id: c.id.clone(),
                    title: c.title.clone(),
                    order: c.sort_order,
                    status: Some(c.status),
                })
                .collect(),
        });
    }
    let exams = bundle
        .exams
        .iter()
        .map(|e| ExamDef {
            id: e.id.clone(),
            subject_id: e.subject_id.clone(),
            label: e.label.clone(),
            date: e.date.to_string(),
            start_time: time::fmt_time(e.start_time),
            session: e.session.clone(),
        })
        .collect();
    let schedule_templates = bundle
        .templates
        .iter()
        .map(|t| TemplateDef {
            id: t.id.clone(),
            name: t.name.clone(),
            day_types: t.day_types.clone(),
            slots: t
                .slots
                .iter()
                .map(|s| SlotDef {
                    id: s.id.clone(),
                    start_time: time::fmt_time(s.start_time),
                    end_time: time::fmt_time(s.end_time),
                    kind: s.kind,
                    title: s.title.clone(),
                    subject_id: s.subject_id.clone(),
                    chapter_id: s.chapter_id.clone(),
                })
                .collect(),
        })
        .collect();
    let tasks = repo::all_tasks_for_plan(conn, plan_id)?
        .into_iter()
        .filter(|t| t.status != TaskStatus::Completed)
        .map(|t| TaskDef {
            id: t.id,
            date: t.date.to_string(),
            title: t.title,
            task_type: t.task_type,
            subject_id: t.subject_id,
            chapter_id: t.chapter_id,
            start_time: time::fmt_time(t.start_time),
            end_time: time::fmt_time(t.end_time),
            estimated_minutes: t.estimated_minutes,
            priority: t.priority,
            status: Some(t.status),
        })
        .collect();
    let settings = repo::load_settings(conn)?;
    Ok(LockinPlanFile {
        format: FORMAT_ID.into(),
        schema_version: SCHEMA_VERSION,
        plan: PlanHeader {
            id: bundle.plan.id,
            name: bundle.plan.name,
            timezone: bundle.plan.timezone,
            start_date: bundle.plan.start_date.to_string(),
            end_date: bundle.plan.end_date.to_string(),
            academic_year: bundle.plan.academic_year,
        },
        subjects,
        exams,
        schedule_templates,
        tasks,
        revision_policy: Some(RevisionPolicy {
            interval_days: settings.revision_intervals,
            adapt_to_results: true,
        }),
    })
}

pub fn to_pretty_json(file: &LockinPlanFile) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(file)
}

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_template_is_valid_json() {
        let t = LockinPlanFile::empty_template();
        let s = to_pretty_json(&t).unwrap();
        let parsed: LockinPlanFile = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.format, FORMAT_ID);
        assert_eq!(parsed.schema_version, 1);
        assert!(parsed.duplicate_ids().is_empty());
    }

    #[test]
    fn duplicate_ids_are_found() {
        let mut t = LockinPlanFile::empty_template();
        t.subjects = vec![
            SubjectDef {
                id: "dup".into(),
                name: "A".into(),
                color: "#fff".into(),
                sort_order: 0,
                chapters: vec![],
            },
            SubjectDef {
                id: "dup".into(),
                name: "B".into(),
                color: "#fff".into(),
                sort_order: 1,
                chapters: vec![],
            },
        ];
        let dups = t.duplicate_ids();
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].0, "subjects[1].id");
    }
}
