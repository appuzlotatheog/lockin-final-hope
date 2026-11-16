//! Persisting focus sessions and rolling their real time into chapter stats.

use crate::db::repo;
use crate::error::AppResult;
use crate::models::*;
use chrono::Utc;
use rusqlite::Connection;

pub struct FinishedSession {
    pub id: String,
    pub focused_seconds: i64,
    pub break_seconds: i64,
}

pub fn mark_task_started(conn: &Connection, task_id: &str) -> AppResult<()> {
    if let Ok(mut task) = repo::get_task(conn, task_id) {
        if matches!(task.status, TaskStatus::Planned) {
            task.status = TaskStatus::InProgress;
            repo::update_task(conn, &task)?;
        }
    }
    Ok(())
}

/// Persist a completed focus session. Only focused seconds are credited to the
/// chapter — break time is stored separately and never counts as study.
pub fn save_session(conn: &mut Connection, plan_id: &str, t: &crate::timer::FocusTimer) -> AppResult<FinishedSession> {
    let focused = t.focus_elapsed().as_secs() as i64;
    let brk = t.break_elapsed().as_secs() as i64;
    let on_date = t.on_date;
    let session = FocusSession {
        id: t.session_id.clone(),
        plan_id: plan_id.to_string(),
        task_id: t.task_id.clone(),
        subject_id: t.subject_id.clone(),
        chapter_id: t.chapter_id.clone(),
        mode: t.mode.clone(),
        started_at: t.started_at_local.to_rfc3339(),
        on_date,
        ended_at: Some(Utc::now().to_rfc3339()),
        focused_seconds: focused,
        break_seconds: brk,
        interruptions: t.interruptions,
        status: "completed".into(),
        notes: String::new(),
    };
    let tx = conn.transaction()?;
    repo::insert_session(&tx, &session)?;
    if let Some(chapter_id) = &t.chapter_id {
        repo::add_chapter_study_seconds(&tx, chapter_id, focused, on_date)?;
    }
    if let Some(task_id) = &t.task_id {
        if let Ok(mut task) = repo::get_task(&tx, task_id) {
            if matches!(task.status, TaskStatus::Planned) {
                task.status = TaskStatus::InProgress;
                repo::update_task(&tx, &task)?;
            }
        }
    }
    tx.commit()?;
    Ok(FinishedSession {
        id: session.id,
        focused_seconds: focused,
        break_seconds: brk,
    })
}

pub fn discard_session(conn: &Connection) -> AppResult<()> {
    repo::clear_timer(conn)
}
