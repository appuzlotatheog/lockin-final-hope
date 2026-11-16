//! Spaced revision scheduling. History is never deleted: grading a review
//! completes it and schedules the next one.

use crate::db::repo;
use crate::error::AppResult;
use crate::models::*;
use crate::scheduling::next_interval;
use chrono::{Duration, NaiveDate, Utc};
use rusqlite::Connection;
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct RevisionCounts {
    pub due: i64,
    pub overdue: i64,
    pub upcoming: i64,
    pub completed: i64,
}

pub fn generate_for_chapter(
    conn: &mut Connection,
    plan_id: &str,
    chapter_id: &str,
    intervals: &[i64],
    base: NaiveDate,
) -> AppResult<usize> {
    repo::get_chapter(conn, chapter_id)?;
    let existing = repo::pending_revisions_for_chapter(conn, chapter_id)?;
    let tx = conn.transaction()?;
    let mut created = 0;
    for interval in intervals.iter().filter(|i| **i > 0) {
        if existing.iter().any(|r| r.interval_days == *interval) {
            continue;
        }
        let rev = Revision {
            id: Uuid::new_v4().to_string(),
            plan_id: plan_id.to_string(),
            chapter_id: chapter_id.to_string(),
            due_date: base + Duration::days(*interval),
            interval_days: *interval,
            ease: 2.5,
            state: RevisionState::Pending,
            result: None,
            completed_at: None,
            snoozed_until: None,
            created_at: Utc::now().to_rfc3339(),
        };
        repo::insert_revision(&tx, &rev)?;
        created += 1;
    }
    sync_chapter_next_revision(&tx, chapter_id)?;
    tx.commit()?;
    Ok(created)
}

fn sync_chapter_next_revision(conn: &Connection, chapter_id: &str) -> AppResult<()> {
    let pending = repo::pending_revisions_for_chapter(conn, chapter_id)?;
    let next = pending
        .iter()
        .map(|r| r.snoozed_until.unwrap_or(r.due_date))
        .min();
    let mut chapter = repo::get_chapter(conn, chapter_id)?;
    chapter.next_revision = next;
    repo::update_chapter(conn, &chapter)?;
    Ok(())
}

pub fn grade(
    conn: &mut Connection,
    revision_id: &str,
    result: RevisionResult,
    today: NaiveDate,
    policy: &[i64],
) -> AppResult<()> {
    let all = {
        let mut stmt = conn.prepare("SELECT id FROM revisions WHERE id=?1")?;
        let found: Option<String> = stmt
            .query_row([revision_id], |r| r.get(0))
            .ok();
        found
    };
    if all.is_none() {
        return Err(crate::error::AppError::NotFound(format!(
            "revision '{revision_id}'"
        )));
    }
    let mut revision = load_revision(conn, revision_id)?;
    let (next_days, new_ease) =
        next_interval(revision.interval_days, result, policy, revision.ease);
    revision.state = RevisionState::Completed;
    revision.result = Some(result);
    revision.completed_at = Some(Utc::now().to_rfc3339());
    revision.snoozed_until = None;
    revision.ease = new_ease;

    let tx = conn.transaction()?;
    repo::update_revision(&tx, &revision)?;

    // Adapt the next pending review for this chapter instead of piling up
    // duplicates. If none exists, create the follow-up review.
    let mut upcoming = repo::pending_revisions_for_chapter(&tx, &revision.chapter_id)?;
    upcoming.sort_by_key(|r| r.due_date);
    if let Some(next) = upcoming.first().cloned() {
        let mut next = next;
        next.due_date = today + Duration::days(next_days.max(1));
        next.interval_days = next_days.max(1);
        next.ease = new_ease;
        next.snoozed_until = None;
        next.state = RevisionState::Pending;
        repo::update_revision(&tx, &next)?;
    } else {
        let follow_up = Revision {
            id: Uuid::new_v4().to_string(),
            plan_id: revision.plan_id.clone(),
            chapter_id: revision.chapter_id.clone(),
            due_date: today + Duration::days(next_days.max(1)),
            interval_days: next_days.max(1),
            ease: new_ease,
            state: RevisionState::Pending,
            result: None,
            completed_at: None,
            snoozed_until: None,
            created_at: Utc::now().to_rfc3339(),
        };
        repo::insert_revision(&tx, &follow_up)?;
    }
    sync_chapter_next_revision(&tx, &revision.chapter_id)?;
    tx.commit()?;
    Ok(())
}

fn load_revision(conn: &Connection, id: &str) -> AppResult<Revision> {
    let mut stmt = conn.prepare(
        "SELECT id,plan_id,chapter_id,due_date,interval_days,ease,state,result,completed_at,snoozed_until,created_at
         FROM revisions WHERE id=?1",
    )?;
    let rev = stmt
        .query_row([id], |row| {
            Ok(Revision {
                id: row.get(0)?,
                plan_id: row.get(1)?,
                chapter_id: row.get(2)?,
                due_date: crate::models::time::parse_date(&row.get::<_, String>(3)?).unwrap_or_default(),
                interval_days: row.get(4)?,
                ease: row.get(5)?,
                state: RevisionState::parse_or(&row.get::<_, String>(6)?, RevisionState::Pending),
                result: row
                    .get::<_, Option<String>>(7)?
                    .and_then(|s| RevisionResult::from_str(&s)),
                completed_at: row.get(8)?,
                snoozed_until: row
                    .get::<_, Option<String>>(9)?
                    .and_then(|s| crate::models::time::parse_date(&s).ok()),
                created_at: row.get(10)?,
            })
        })
        .map_err(|_| crate::error::AppError::NotFound(format!("revision '{id}'")))?;
    Ok(rev)
}

pub fn snooze(conn: &mut Connection, revision_id: &str, days: i64, today: NaiveDate) -> AppResult<()> {
    let mut revision = load_revision(conn, revision_id)?;
    revision.state = RevisionState::Snoozed;
    revision.snoozed_until = Some(today + Duration::days(days.max(1)));
    let tx = conn.transaction()?;
    repo::update_revision(&tx, &revision)?;
    sync_chapter_next_revision(&tx, &revision.chapter_id)?;
    tx.commit()?;
    Ok(())
}

pub fn reschedule(conn: &mut Connection, revision_id: &str, date: NaiveDate) -> AppResult<()> {
    let mut revision = load_revision(conn, revision_id)?;
    revision.state = RevisionState::Pending;
    revision.due_date = date;
    revision.snoozed_until = None;
    let tx = conn.transaction()?;
    repo::update_revision(&tx, &revision)?;
    sync_chapter_next_revision(&tx, &revision.chapter_id)?;
    tx.commit()?;
    Ok(())
}

pub fn counts(conn: &Connection, plan_id: &str, today: NaiveDate) -> AppResult<RevisionCounts> {
    let all = repo::revisions_for_plan(conn, plan_id)?;
    let mut c = RevisionCounts::default();
    for rev in all {
        let effective = rev.snoozed_until.unwrap_or(rev.due_date);
        match rev.state {
            RevisionState::Completed => c.completed += 1,
            _ if effective < today => c.overdue += 1,
            _ if effective == today => c.due += 1,
            _ => c.upcoming += 1,
        }
    }
    Ok(c)
}
