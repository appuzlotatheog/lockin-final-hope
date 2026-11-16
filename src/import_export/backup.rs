//! Full backup/restore of every application table, separate from the editable
//! plan format. Restore is all-or-nothing and refuses unknown future versions.

use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::models::*;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const BACKUP_FORMAT: &str = "lockin-backup";
pub const BACKUP_VERSION: i64 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupFile {
    pub format: String,
    pub schema_version: i64,
    pub app_version: String,
    pub exported_at: String,
    pub plans: Vec<Plan>,
    pub subjects: Vec<Subject>,
    pub chapters: Vec<Chapter>,
    pub exams: Vec<Exam>,
    pub schedule_templates: Vec<ScheduleTemplate>,
    pub tasks: Vec<Task>,
    pub day_overrides: Vec<DayOverride>,
    pub sessions: Vec<FocusSession>,
    pub revisions: Vec<Revision>,
    pub mock_tests: Vec<MockTest>,
    pub mistakes: Vec<Mistake>,
    pub sleep_entries: Vec<SleepEntry>,
    pub settings: Settings,
}

impl BackupFile {
    pub fn counts(&self) -> String {
        format!(
            "{} plans · {} subjects · {} chapters · {} tasks · {} sessions · {} revisions · {} mock tests · {} sleep logs",
            self.plans.len(),
            self.subjects.len(),
            self.chapters.len(),
            self.tasks.len(),
            self.sessions.len(),
            self.revisions.len(),
            self.mock_tests.len(),
            self.sleep_entries.len()
        )
    }
}

pub fn export(conn: &Connection) -> AppResult<BackupFile> {
    let plans = repo::list_plans(conn, true)?;
    let mut subjects = Vec::new();
    let mut chapters = Vec::new();
    let mut exams = Vec::new();
    let mut templates = Vec::new();
    let mut tasks = Vec::new();
    let mut overrides = Vec::new();
    let mut sessions = Vec::new();
    let mut revisions = Vec::new();
    let mut mocks = Vec::new();
    let mut mistakes = Vec::new();
    let mut sleep = Vec::new();
    for plan in &plans {
        subjects.extend(repo::subjects_for_plan(conn, &plan.id)?);
        chapters.extend(repo::chapters_for_plan(conn, &plan.id)?);
        exams.extend(repo::exams_for_plan(conn, &plan.id)?);
        templates.extend(repo::templates_for_plan(conn, &plan.id)?);
        tasks.extend(repo::all_tasks_for_plan(conn, &plan.id)?);
        overrides.extend(repo::day_overrides_for_plan(conn, &plan.id)?);
        sessions.extend(repo::all_sessions_for_plan(conn, &plan.id)?);
        revisions.extend(repo::revisions_for_plan(conn, &plan.id)?);
        mocks.extend(repo::mock_tests_for_plan(conn, &plan.id)?);
        mistakes.extend(repo::mistakes_for_plan(conn, &plan.id)?);
        sleep.extend(repo::sleep_entries_for_plan(conn, &plan.id, 100_000)?);
    }
    Ok(BackupFile {
        format: BACKUP_FORMAT.into(),
        schema_version: BACKUP_VERSION,
        app_version: env!("CARGO_PKG_VERSION").into(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        plans,
        subjects,
        chapters,
        exams,
        schedule_templates: templates,
        tasks,
        day_overrides: overrides,
        sessions,
        revisions,
        mock_tests: mocks,
        mistakes,
        sleep_entries: sleep,
        settings: repo::load_settings(conn)?,
    })
}

pub fn to_json(backup: &BackupFile) -> AppResult<String> {
    Ok(serde_json::to_string_pretty(backup)?)
}

pub fn parse(raw: &str) -> AppResult<BackupFile> {
    if raw.len() > crate::import_export::MAX_BACKUP_BYTES {
        return Err(AppError::validation(
            "Backup file is larger than the 25 MB restore limit.",
        ));
    }
    let value: serde_json::Value = serde_json::from_str(raw)?;
    let format = value.get("format").and_then(|v| v.as_str()).unwrap_or("");
    if format != BACKUP_FORMAT {
        return Err(AppError::validation(format!(
            "This is not a Lockin backup: expected format '{BACKUP_FORMAT}', found '{format}'."
        )));
    }
    let version = value
        .get("schema_version")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if version > BACKUP_VERSION {
        return Err(AppError::validation(format!(
            "Backup schema version {version} is newer than this app supports ({BACKUP_VERSION}). Update the app first."
        )));
    }
    let backup: BackupFile = serde_json::from_value(value)?;
    validate(&backup)?;
    Ok(backup)
}

fn validate(b: &BackupFile) -> AppResult<()> {
    let plan_ids: HashSet<&str> = b.plans.iter().map(|p| p.id.as_str()).collect();
    if plan_ids.len() != b.plans.len() {
        return Err(AppError::validation("Backup contains duplicate plan ids."));
    }
    let subject_owner: std::collections::HashMap<&str, &str> =
        b.subjects.iter().map(|s| (s.id.as_str(), s.plan_id.as_str())).collect();
    if subject_owner.len() != b.subjects.len() {
        return Err(AppError::validation("Backup contains duplicate subject ids."));
    }
    let chapter_owner: std::collections::HashMap<&str, &str> =
        b.chapters.iter().map(|c| (c.id.as_str(), c.subject_id.as_str())).collect();
    if chapter_owner.len() != b.chapters.len() {
        return Err(AppError::validation("Backup contains duplicate chapter ids."));
    }
    for subject in &b.subjects {
        if !plan_ids.contains(subject.plan_id.as_str()) {
            return Err(AppError::validation(format!(
                "Subject '{}' references a missing plan '{}'.",
                subject.id, subject.plan_id
            )));
        }
    }
    for chapter in &b.chapters {
        if !chapter_owner.is_empty() && !subject_owner.contains_key(chapter.subject_id.as_str()) {
            return Err(AppError::validation(format!(
                "Chapter '{}' references a missing subject '{}'.",
                chapter.id, chapter.subject_id
            )));
        }
    }
    for plan in &b.plans {
        if plan.start_date > plan.end_date {
            return Err(AppError::validation(format!(
                "Plan '{}' ends before it starts.",
                plan.name
            )));
        }
        if plan.timezone.parse::<chrono_tz::Tz>().is_err() {
            return Err(AppError::validation(format!(
                "Plan '{}' has an invalid timezone '{}'.",
                plan.name, plan.timezone
            )));
        }
    }
    Ok(())
}

/// Replace the entire local database contents with the backup, atomically.
pub fn restore(conn: &mut Connection, backup: &BackupFile) -> AppResult<()> {
    validate(backup)?;
    let tx = conn.transaction()?;
    tx.execute_batch(
        "DELETE FROM timer_state;
         DELETE FROM revisions;
         DELETE FROM mistakes;
         DELETE FROM mock_tests;
         DELETE FROM sleep_entries;
         DELETE FROM study_sessions;
         DELETE FROM day_overrides;
         DELETE FROM tasks;
         DELETE FROM template_slots;
         DELETE FROM schedule_templates;
         DELETE FROM exams;
         DELETE FROM chapters;
         DELETE FROM subjects;
         DELETE FROM plans;",
    )?;
    for plan in &backup.plans {
        repo::insert_plan(&tx, plan)?;
    }
    for subject in &backup.subjects {
        repo::insert_subject(&tx, subject)?;
    }
    for chapter in &backup.chapters {
        repo::insert_chapter(&tx, chapter)?;
    }
    for exam in &backup.exams {
        repo::insert_exam(&tx, exam)?;
    }
    for (idx, template) in backup.schedule_templates.iter().enumerate() {
        repo::insert_template(&tx, template, idx as i64)?;
        for (s_idx, slot) in template.slots.iter().enumerate() {
            repo::insert_slot(&tx, slot, s_idx as i64)?;
        }
    }
    for task in &backup.tasks {
        repo::insert_task(&tx, task)?;
    }
    for over in &backup.day_overrides {
        repo::set_day_override(&tx, over)?;
    }
    for session in &backup.sessions {
        repo::insert_session(&tx, session)?;
    }
    for revision in &backup.revisions {
        repo::insert_revision(&tx, revision)?;
    }
    for mock in &backup.mock_tests {
        repo::insert_mock_test(&tx, mock)?;
    }
    for mistake in &backup.mistakes {
        repo::insert_mistake(&tx, mistake)?;
    }
    for entry in &backup.sleep_entries {
        repo::insert_sleep(&tx, entry)?;
    }
    let mut settings = backup.settings.clone();
    if let Some(last) = &settings.last_plan_id {
        if !backup.plans.iter().any(|p| &p.id == last) {
            settings.last_plan_id = backup.plans.first().map(|p| p.id.clone());
        }
    } else {
        settings.last_plan_id = backup.plans.first().map(|p| p.id.clone());
    }
    repo::save_settings(&tx, &settings)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_round_trip_preserves_everything() {
        let dir = std::env::temp_dir().join(format!("lockin-backup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = crate::db::Database::open(&dir.join("a.db")).unwrap();
        db.migrate().unwrap();
        db.seed_if_empty().unwrap();

        // Add a completed session and a chapter status so history matters.
        let plan_id = crate::db::seed::SEED_PLAN_ID;
        db.conn
            .execute_batch(&format!(
                "UPDATE chapters SET status='exam_ready' WHERE id='physics-01';
                 INSERT INTO study_sessions (id,plan_id,subject_id,chapter_id,mode,started_at,on_date,focused_seconds,status)
                 VALUES ('sess-test','{plan_id}','sub-physics','physics-01','deep','2026-11-16T20:00:00+05:30','2026-11-16',1500,'completed');"
            ))
            .unwrap();
        let backup = export(&db.conn).unwrap();
        let json = to_json(&backup).unwrap();
        let parsed = parse(&json).unwrap();
        assert!(parsed.chapters.iter().any(|c| c.status == ChapterStatus::ExamReady));
        assert_eq!(parsed.sessions.len(), 1);

        // Restore into a fresh database.
        let mut db2 = crate::db::Database::open(&dir.join("b.db")).unwrap();
        db2.migrate().unwrap();
        restore(&mut db2.conn, &parsed).unwrap();
        let again = export(&db2.conn).unwrap();
        assert_eq!(again.plans.len(), backup.plans.len());
        assert_eq!(again.tasks.len(), backup.tasks.len());
        assert_eq!(again.sessions.len(), 1);
        assert_eq!(again.chapters.len(), 52);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_non_backup_json() {
        let err = parse(r#"{"format":"lockin-plan","schema_version":1,"plan":{}}"#).unwrap_err();
        assert!(err.to_string().contains("not a Lockin backup"));
    }
}
