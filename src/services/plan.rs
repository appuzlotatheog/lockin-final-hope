//! Plan lifecycle: bundles, task generation, archiving and plan switching.

use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::models::time::date_range;
use crate::models::*;
use crate::scheduling;
use chrono::{NaiveDate, Utc};
use rusqlite::Connection;

pub fn load_bundle(conn: &Connection, plan_id: &str) -> AppResult<PlanBundle> {
    let plan = repo::get_plan(conn, plan_id)?;
    let subjects = repo::subjects_for_plan(conn, plan_id)?;
    let mut grouped = Vec::new();
    for subject in subjects {
        let chapters = repo::chapters_for_subject(conn, &subject.id)?;
        grouped.push((subject, chapters));
    }
    Ok(PlanBundle {
        exams: repo::exams_for_plan(conn, plan_id)?,
        templates: repo::templates_for_plan(conn, plan_id)?,
        subjects: grouped,
        plan,
    })
}

pub fn current_plan_id(conn: &Connection, settings: &Settings) -> AppResult<String> {
    let plans = repo::list_plans(conn, false)?;
    if plans.is_empty() {
        return Err(AppError::NotFound("an active plan".into()));
    }
    if let Some(id) = &settings.last_plan_id {
        if plans.iter().any(|p| &p.id == id) {
            return Ok(id.clone());
        }
    }
    Ok(plans[0].id.clone())
}

/// Materialise planned tasks for every date in the range, skipping days that
/// already have tasks. Never deletes or rewrites existing work.
pub fn generate_tasks_for_range(
    conn: &mut Connection,
    plan_id: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> AppResult<usize> {
    let plan = repo::get_plan(conn, plan_id)?;
    let templates = repo::templates_for_plan(conn, plan_id)?;
    let overrides = repo::day_overrides_for_plan(conn, plan_id)?;
    let tx = conn.transaction()?;
    let mut inserted = 0usize;
    for date in date_range(start.max(plan.start_date), end.min(plan.end_date)) {
        let existing = repo::tasks_for_date(&tx, plan_id, date)?;
        if !existing.is_empty() {
            continue;
        }
        for task in scheduling::planned_tasks_for_date(&plan, date, &templates, &overrides) {
            repo::insert_task(&tx, &task)?;
            inserted += 1;
        }
    }
    tx.commit()?;
    Ok(inserted)
}

/// Replan a single day. Returns the proposal; the caller must confirm and then
/// call `apply_replan`.
pub fn replan_proposal(
    conn: &Connection,
    plan_id: &str,
    date: NaiveDate,
) -> AppResult<(scheduling::ReplanOutcome, Plan)> {
    let plan = repo::get_plan(conn, plan_id)?;
    let templates = repo::templates_for_plan(conn, plan_id)?;
    let overrides = repo::day_overrides_for_plan(conn, plan_id)?;
    let existing = repo::tasks_for_date(conn, plan_id, date)?;
    let outcome = scheduling::replan_day(&plan, date, &templates, &overrides, &existing);
    Ok((outcome, plan))
}

pub fn apply_replan(conn: &mut Connection, outcome: &scheduling::ReplanOutcome) -> AppResult<()> {
    let tx = conn.transaction()?;
    for moved in &outcome.backlog_moves {
        repo::update_task(&tx, moved)?;
    }
    for task in &outcome.added {
        repo::insert_task(&tx, task)?;
    }
    tx.commit()?;
    Ok(())
}

/// Copy a plan's configuration into a brand-new season without touching the
/// original. Chapter progress is intentionally reset for the new season.
pub fn clone_plan_for_season(
    conn: &mut Connection,
    source_plan_id: &str,
    new_plan_id: &str,
    name: &str,
    academic_year: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> AppResult<()> {
    let bundle = load_bundle(conn, source_plan_id)?;
    let tx = conn.transaction()?;
    let plan = Plan {
        id: new_plan_id.to_string(),
        name: name.to_string(),
        academic_year: academic_year.to_string(),
        timezone: bundle.plan.timezone.clone(),
        start_date: start,
        end_date: end,
        archived: false,
        created_at: Utc::now().to_rfc3339(),
    };
    repo::insert_plan(&tx, &plan)?;
    let mut subject_map = std::collections::HashMap::new();
    for (s_idx, (subject, chapters)) in bundle.subjects.iter().enumerate() {
        let new_subject = Subject {
            id: format!("{new_plan_id}-{}-{}", subject.id, s_idx),
            plan_id: plan.id.clone(),
            name: subject.name.clone(),
            color: subject.color.clone(),
            sort_order: s_idx as i64,
            portions_pending: subject.portions_pending,
        };
        repo::insert_subject(&tx, &new_subject)?;
        subject_map.insert(subject.id.clone(), new_subject.id.clone());
        for (c_idx, chapter) in chapters.iter().enumerate() {
            repo::insert_chapter(
                &tx,
                &Chapter {
                    id: format!("{}-{}", new_subject.id, c_idx + 1),
                    subject_id: new_subject.id.clone(),
                    title: chapter.title.clone(),
                    sort_order: c_idx as i64,
                    ..chapter.clone()
                },
            )?;
        }
    }
    for (t_idx, template) in bundle.templates.iter().enumerate() {
        let mut new_template = ScheduleTemplate {
            id: format!("{new_plan_id}-tpl-{t_idx}"),
            plan_id: plan.id.clone(),
            name: template.name.clone(),
            day_types: template.day_types.clone(),
            slots: Vec::new(),
        };
        repo::insert_template(&tx, &new_template, t_idx as i64)?;
        for (s_idx, slot) in template.slots.iter().enumerate() {
            repo::insert_slot(
                &tx,
                &TemplateSlot {
                    id: format!("{}-slot-{s_idx}", new_template.id),
                    template_id: new_template.id.clone(),
                    subject_id: slot.subject_id.as_ref().and_then(|id| subject_map.get(id).cloned()),
                    ..slot.clone()
                },
                s_idx as i64,
            )?;
        }
        new_template.slots = template.slots.clone();
    }
    tx.commit()?;

    generate_tasks_for_range(conn, &plan.id, start, end)?;
    Ok(())
}
