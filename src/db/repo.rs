//! Typed repository functions. All SQL lives here; services compose these.

use crate::error::{AppError, AppResult};
use crate::models::time::{parse_date, parse_time};
use crate::models::*;
use chrono::{NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};

fn b(v: i64) -> bool {
    v != 0
}

fn stamp() -> String {
    Utc::now().to_rfc3339()
}

fn date_opt(row: &Row<'_>, idx: usize) -> rusqlite::Result<Option<NaiveDate>> {
    let s: Option<String> = row.get(idx)?;
    Ok(s.and_then(|s| parse_date(&s).ok()))
}

// ---------------------------------------------------------------- settings

pub const SETTING_KEYS: &[&str] = &[
    "display_name",
    "accent",
    "theme",
    "timezone",
    "daily_goal_minutes",
    "sleep_target_hours",
    "sound_enabled",
    "notifications_enabled",
    "default_import_mode",
    "revision_intervals",
    "auto_generate_revisions",
    "last_plan_id",
];

pub fn ensure_setting_defaults(conn: &Connection) -> AppResult<()> {
    let defaults = Settings::default();
    let vals = [
        ("display_name", defaults.display_name.clone()),
        ("accent", defaults.accent.clone()),
        ("theme", defaults.theme.clone()),
        ("timezone", defaults.timezone.clone()),
        ("daily_goal_minutes", defaults.daily_goal_minutes.to_string()),
        ("sleep_target_hours", defaults.sleep_target_hours.to_string()),
        ("sound_enabled", (defaults.sound_enabled as i64).to_string()),
        ("notifications_enabled", (defaults.notifications_enabled as i64).to_string()),
        ("default_import_mode", defaults.default_import_mode.clone()),
        (
            "revision_intervals",
            serde_json::to_string(&defaults.revision_intervals).unwrap_or_else(|_| "[1,3,7]".into()),
        ),
        ("auto_generate_revisions", (defaults.auto_generate_revisions as i64).to_string()),
    ];
    for (k, v) in vals {
        conn.execute(
            "INSERT OR IGNORE INTO settings (key,value) VALUES (?1,?2)",
            params![k, v],
        )?;
    }
    Ok(())
}

pub fn get_setting(conn: &Connection, key: &str) -> AppResult<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0))
        .optional()?)
}

pub fn put_setting(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO settings (key,value) VALUES (?1,?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn load_settings(conn: &Connection) -> AppResult<Settings> {
    ensure_setting_defaults(conn)?;
    let mut s = Settings::default();
    if let Some(v) = get_setting(conn, "display_name")? {
        s.display_name = v;
    }
    if let Some(v) = get_setting(conn, "accent")? {
        s.accent = v;
    }
    if let Some(v) = get_setting(conn, "theme")? {
        s.theme = v;
    }
    if let Some(v) = get_setting(conn, "timezone")? {
        s.timezone = v;
    }
    if let Some(v) = get_setting(conn, "daily_goal_minutes")? {
        s.daily_goal_minutes = v.parse().unwrap_or(s.daily_goal_minutes);
    }
    if let Some(v) = get_setting(conn, "sleep_target_hours")? {
        s.sleep_target_hours = v.parse().unwrap_or(s.sleep_target_hours);
    }
    if let Some(v) = get_setting(conn, "sound_enabled")? {
        s.sound_enabled = v == "1" || v == "true";
    }
    if let Some(v) = get_setting(conn, "notifications_enabled")? {
        s.notifications_enabled = v == "1" || v == "true";
    }
    if let Some(v) = get_setting(conn, "default_import_mode")? {
        s.default_import_mode = v;
    }
    if let Some(v) = get_setting(conn, "revision_intervals")? {
        if let Ok(list) = serde_json::from_str::<Vec<i64>>(&v) {
            if !list.is_empty() {
                s.revision_intervals = list;
            }
        }
    }
    if let Some(v) = get_setting(conn, "auto_generate_revisions")? {
        s.auto_generate_revisions = v == "1" || v == "true";
    }
    s.last_plan_id = get_setting(conn, "last_plan_id")?;
    Ok(s)
}

pub fn save_settings(conn: &Connection, s: &Settings) -> AppResult<()> {
    put_setting(conn, "display_name", &s.display_name)?;
    put_setting(conn, "accent", &s.accent)?;
    put_setting(conn, "theme", &s.theme)?;
    put_setting(conn, "timezone", &s.timezone)?;
    put_setting(conn, "daily_goal_minutes", &s.daily_goal_minutes.to_string())?;
    put_setting(conn, "sleep_target_hours", &s.sleep_target_hours.to_string())?;
    put_setting(conn, "sound_enabled", if s.sound_enabled { "1" } else { "0" })?;
    put_setting(
        conn,
        "notifications_enabled",
        if s.notifications_enabled { "1" } else { "0" },
    )?;
    put_setting(conn, "default_import_mode", &s.default_import_mode)?;
    put_setting(
        conn,
        "revision_intervals",
        &serde_json::to_string(&s.revision_intervals)?,
    )?;
    put_setting(
        conn,
        "auto_generate_revisions",
        if s.auto_generate_revisions { "1" } else { "0" },
    )?;
    if let Some(id) = &s.last_plan_id {
        put_setting(conn, "last_plan_id", id)?;
    }
    Ok(())
}

// ------------------------------------------------------------------- plans

fn row_to_plan(row: &Row<'_>) -> rusqlite::Result<Plan> {
    Ok(Plan {
        id: row.get(0)?,
        name: row.get(1)?,
        academic_year: row.get(2)?,
        timezone: row.get(3)?,
        start_date: parse_date(&row.get::<_, String>(4)?).unwrap_or_default(),
        end_date: parse_date(&row.get::<_, String>(5)?).unwrap_or_default(),
        archived: b(row.get::<_, i64>(6)?),
        created_at: row.get(7)?,
    })
}

const PLAN_COLS: &str = "id,name,academic_year,timezone,start_date,end_date,archived,created_at";

pub fn list_plans(conn: &Connection, include_archived: bool) -> AppResult<Vec<Plan>> {
    let sql = format!(
        "SELECT {PLAN_COLS} FROM plans {} ORDER BY archived, start_date",
        if include_archived { "" } else { "WHERE archived=0" }
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_plan)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn get_plan(conn: &Connection, id: &str) -> AppResult<Plan> {
    conn.query_row(
        &format!("SELECT {PLAN_COLS} FROM plans WHERE id=?1"),
        [id],
        row_to_plan,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("plan '{id}'")))
}

pub fn insert_plan(conn: &Connection, p: &Plan) -> AppResult<()> {
    conn.execute(
        "INSERT INTO plans (id,name,academic_year,timezone,start_date,end_date,archived,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            p.id,
            p.name,
            p.academic_year,
            p.timezone,
            p.start_date.to_string(),
            p.end_date.to_string(),
            p.archived as i64,
            if p.created_at.is_empty() { stamp() } else { p.created_at.clone() }
        ],
    )?;
    Ok(())
}

pub fn update_plan(conn: &Connection, p: &Plan) -> AppResult<()> {
    conn.execute(
        "UPDATE plans SET name=?2, academic_year=?3, timezone=?4, start_date=?5, end_date=?6, archived=?7
         WHERE id=?1",
        params![
            p.id,
            p.name,
            p.academic_year,
            p.timezone,
            p.start_date.to_string(),
            p.end_date.to_string(),
            p.archived as i64
        ],
    )?;
    Ok(())
}

pub fn set_plan_archived(conn: &Connection, id: &str, archived: bool) -> AppResult<()> {
    conn.execute(
        "UPDATE plans SET archived=?2 WHERE id=?1",
        params![id, archived as i64],
    )?;
    Ok(())
}

pub fn delete_plan(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM plans WHERE id=?1", [id])?;
    Ok(())
}

// -------------------------------------------------------- subjects/chapters

fn row_to_subject(row: &Row<'_>) -> rusqlite::Result<Subject> {
    Ok(Subject {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        name: row.get(2)?,
        color: row.get(3)?,
        sort_order: row.get(4)?,
        portions_pending: b(row.get(5)?),
    })
}

const SUBJECT_COLS: &str = "id,plan_id,name,color,sort_order,portions_pending";

pub fn subjects_for_plan(conn: &Connection, plan_id: &str) -> AppResult<Vec<Subject>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SUBJECT_COLS} FROM subjects WHERE plan_id=?1 ORDER BY sort_order, name"
    ))?;
    let rows = stmt.query_map([plan_id], row_to_subject)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn get_subject(conn: &Connection, id: &str) -> AppResult<Subject> {
    conn.query_row(
        &format!("SELECT {SUBJECT_COLS} FROM subjects WHERE id=?1"),
        [id],
        row_to_subject,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("subject '{id}'")))
}

pub fn insert_subject(conn: &Connection, s: &Subject) -> AppResult<()> {
    conn.execute(
        "INSERT INTO subjects (id,plan_id,name,color,sort_order,portions_pending)
         VALUES (?1,?2,?3,?4,?5,?6)",
        params![s.id, s.plan_id, s.name, s.color, s.sort_order, s.portions_pending as i64],
    )?;
    Ok(())
}

pub fn update_subject(conn: &Connection, s: &Subject) -> AppResult<()> {
    conn.execute(
        "UPDATE subjects SET name=?2,color=?3,sort_order=?4,portions_pending=?5 WHERE id=?1",
        params![s.id, s.name, s.color, s.sort_order, s.portions_pending as i64],
    )?;
    Ok(())
}

pub fn delete_subject(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM subjects WHERE id=?1", [id])?;
    Ok(())
}

fn row_to_chapter(row: &Row<'_>) -> rusqlite::Result<Chapter> {
    Ok(Chapter {
        id: row.get(0)?,
        subject_id: row.get(1)?,
        title: row.get(2)?,
        sort_order: row.get(3)?,
        status: ChapterStatus::parse_or(&row.get::<_, String>(4)?, ChapterStatus::NotStarted),
        concept_done: b(row.get(5)?),
        textbook_done: b(row.get(6)?),
        exercises_done: b(row.get(7)?),
        selftest_done: b(row.get(8)?),
        confidence: row.get(9)?,
        questions_attempted: row.get(10)?,
        correct_answers: row.get(11)?,
        last_studied: date_opt(row, 12)?,
        next_revision: date_opt(row, 13)?,
        study_seconds: row.get(14)?,
        notes: row.get(15)?,
    })
}

const CHAPTER_COLS: &str = "id,subject_id,title,sort_order,status,concept_done,textbook_done,exercises_done,selftest_done,confidence,questions_attempted,correct_answers,last_studied,next_revision,study_seconds,notes";

pub fn chapters_for_subject(conn: &Connection, subject_id: &str) -> AppResult<Vec<Chapter>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {CHAPTER_COLS} FROM chapters WHERE subject_id=?1 ORDER BY sort_order, id"
    ))?;
    let rows = stmt.query_map([subject_id], row_to_chapter)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn chapters_for_plan(conn: &Connection, plan_id: &str) -> AppResult<Vec<Chapter>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT c.id,c.subject_id,c.title,c.sort_order,c.status,c.concept_done,c.textbook_done,c.exercises_done,c.selftest_done,c.confidence,c.questions_attempted,c.correct_answers,c.last_studied,c.next_revision,c.study_seconds,c.notes
         FROM chapters c JOIN subjects s ON s.id=c.subject_id WHERE s.plan_id=?1 ORDER BY s.sort_order, c.sort_order"
    ))?;
    let rows = stmt.query_map([plan_id], row_to_chapter)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn get_chapter(conn: &Connection, id: &str) -> AppResult<Chapter> {
    conn.query_row(
        &format!("SELECT {CHAPTER_COLS} FROM chapters WHERE id=?1"),
        [id],
        row_to_chapter,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("chapter '{id}'")))
}

pub fn insert_chapter(conn: &Connection, c: &Chapter) -> AppResult<()> {
    conn.execute(
        "INSERT INTO chapters (id,subject_id,title,sort_order,status,concept_done,textbook_done,exercises_done,selftest_done,confidence,questions_attempted,correct_answers,last_studied,next_revision,study_seconds,notes)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        params![
            c.id,
            c.subject_id,
            c.title,
            c.sort_order,
            c.status.as_str(),
            c.concept_done as i64,
            c.textbook_done as i64,
            c.exercises_done as i64,
            c.selftest_done as i64,
            c.confidence,
            c.questions_attempted,
            c.correct_answers,
            c.last_studied.map(|d| d.to_string()),
            c.next_revision.map(|d| d.to_string()),
            c.study_seconds,
            c.notes
        ],
    )?;
    Ok(())
}

pub fn update_chapter(conn: &Connection, c: &Chapter) -> AppResult<()> {
    conn.execute(
        "UPDATE chapters SET title=?2,sort_order=?3,status=?4,concept_done=?5,textbook_done=?6,
         exercises_done=?7,selftest_done=?8,confidence=?9,questions_attempted=?10,correct_answers=?11,
         last_studied=?12,next_revision=?13,study_seconds=?14,notes=?15 WHERE id=?1",
        params![
            c.id,
            c.title,
            c.sort_order,
            c.status.as_str(),
            c.concept_done as i64,
            c.textbook_done as i64,
            c.exercises_done as i64,
            c.selftest_done as i64,
            c.confidence,
            c.questions_attempted,
            c.correct_answers,
            c.last_studied.map(|d| d.to_string()),
            c.next_revision.map(|d| d.to_string()),
            c.study_seconds,
            c.notes
        ],
    )?;
    Ok(())
}

pub fn delete_chapter(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM chapters WHERE id=?1", [id])?;
    Ok(())
}

pub fn add_chapter_study_seconds(conn: &Connection, id: &str, secs: i64, date: NaiveDate) -> AppResult<()> {
    conn.execute(
        "UPDATE chapters SET study_seconds=study_seconds+?2, last_studied=?3 WHERE id=?1",
        params![id, secs, date.to_string()],
    )?;
    Ok(())
}

// ------------------------------------------------------------------- exams

fn row_to_exam(row: &Row<'_>) -> rusqlite::Result<Exam> {
    Ok(Exam {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        subject_id: row.get(2)?,
        label: row.get(3)?,
        date: parse_date(&row.get::<_, String>(4)?).unwrap_or_default(),
        start_time: parse_time(&row.get::<_, String>(5)?).unwrap_or_default(),
        session: row.get(6)?,
        duration_minutes: row.get(7)?,
    })
}

const EXAM_COLS: &str = "id,plan_id,subject_id,label,date,start_time,session,duration_minutes";

pub fn exams_for_plan(conn: &Connection, plan_id: &str) -> AppResult<Vec<Exam>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {EXAM_COLS} FROM exams WHERE plan_id=?1 ORDER BY date, start_time"
    ))?;
    let rows = stmt.query_map([plan_id], row_to_exam)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn insert_exam(conn: &Connection, e: &Exam) -> AppResult<()> {
    conn.execute(
        "INSERT INTO exams (id,plan_id,subject_id,label,date,start_time,session,duration_minutes)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            e.id,
            e.plan_id,
            e.subject_id,
            e.label,
            e.date.to_string(),
            e.start_time.format("%H:%M").to_string(),
            e.session,
            e.duration_minutes
        ],
    )?;
    Ok(())
}

pub fn update_exam(conn: &Connection, e: &Exam) -> AppResult<()> {
    conn.execute(
        "UPDATE exams SET subject_id=?2,label=?3,date=?4,start_time=?5,session=?6,duration_minutes=?7 WHERE id=?1",
        params![
            e.id,
            e.subject_id,
            e.label,
            e.date.to_string(),
            e.start_time.format("%H:%M").to_string(),
            e.session,
            e.duration_minutes
        ],
    )?;
    Ok(())
}

pub fn delete_exam(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM exams WHERE id=?1", [id])?;
    Ok(())
}

// --------------------------------------------------------------- templates

fn row_to_slot(row: &Row<'_>) -> rusqlite::Result<TemplateSlot> {
    Ok(TemplateSlot {
        id: row.get(0)?,
        template_id: row.get(1)?,
        start_time: parse_time(&row.get::<_, String>(2)?).unwrap_or_default(),
        end_time: parse_time(&row.get::<_, String>(3)?).unwrap_or_default(),
        kind: SlotKind::parse_or(&row.get::<_, String>(4)?, SlotKind::Study),
        title: row.get(5)?,
        subject_id: row.get(6)?,
        chapter_id: row.get(7)?,
    })
}

pub fn templates_for_plan(conn: &Connection, plan_id: &str) -> AppResult<Vec<ScheduleTemplate>> {
    let mut stmt = conn.prepare(
        "SELECT id,plan_id,name,day_types FROM schedule_templates WHERE plan_id=?1 ORDER BY sort_order, name",
    )?;
    let base: Vec<(String, String, String, String)> = stmt
        .query_map([plan_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::new();
    for (id, plan_id, name, days_json) in base {
        let mut slot_stmt = conn.prepare(
            "SELECT id,template_id,start_time,end_time,kind,title,subject_id,chapter_id
             FROM template_slots WHERE template_id=?1 ORDER BY sort_order, start_time",
        )?;
        let slots = slot_stmt
            .query_map([&id], row_to_slot)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let day_types: Vec<DayType> = serde_json::from_str(&days_json).unwrap_or_default();
        out.push(ScheduleTemplate {
            id,
            plan_id,
            name,
            day_types,
            slots,
        });
    }
    Ok(out)
}

pub fn insert_template(conn: &Connection, t: &ScheduleTemplate, sort: i64) -> AppResult<()> {
    conn.execute(
        "INSERT INTO schedule_templates (id,plan_id,name,day_types,sort_order) VALUES (?1,?2,?3,?4,?5)",
        params![
            t.id,
            t.plan_id,
            t.name,
            serde_json::to_string(&t.day_types)?,
            sort
        ],
    )?;
    Ok(())
}

pub fn update_template(conn: &Connection, t: &ScheduleTemplate) -> AppResult<()> {
    conn.execute(
        "UPDATE schedule_templates SET name=?2, day_types=?3 WHERE id=?1",
        params![t.id, t.name, serde_json::to_string(&t.day_types)?],
    )?;
    Ok(())
}

pub fn delete_template(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM schedule_templates WHERE id=?1", [id])?;
    Ok(())
}

pub fn insert_slot(conn: &Connection, slot: &TemplateSlot, sort: i64) -> AppResult<()> {
    conn.execute(
        "INSERT INTO template_slots (id,template_id,start_time,end_time,kind,title,subject_id,chapter_id,sort_order)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            slot.id,
            slot.template_id,
            slot.start_time.format("%H:%M").to_string(),
            slot.end_time.format("%H:%M").to_string(),
            slot.kind.as_str(),
            slot.title,
            slot.subject_id,
            slot.chapter_id,
            sort
        ],
    )?;
    Ok(())
}

pub fn update_slot(conn: &Connection, slot: &TemplateSlot) -> AppResult<()> {
    conn.execute(
        "UPDATE template_slots SET start_time=?2,end_time=?3,kind=?4,title=?5,subject_id=?6,chapter_id=?7 WHERE id=?1",
        params![
            slot.id,
            slot.start_time.format("%H:%M").to_string(),
            slot.end_time.format("%H:%M").to_string(),
            slot.kind.as_str(),
            slot.title,
            slot.subject_id,
            slot.chapter_id
        ],
    )?;
    Ok(())
}

pub fn delete_slot(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM template_slots WHERE id=?1", [id])?;
    Ok(())
}

// ------------------------------------------------------------------- tasks

fn row_to_task(row: &Row<'_>) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        date: parse_date(&row.get::<_, String>(2)?).unwrap_or_default(),
        title: row.get(3)?,
        task_type: TaskType::parse_or(&row.get::<_, String>(4)?, TaskType::Study),
        subject_id: row.get(5)?,
        chapter_id: row.get(6)?,
        start_time: parse_time(&row.get::<_, String>(7)?).unwrap_or_default(),
        end_time: parse_time(&row.get::<_, String>(8)?).unwrap_or_default(),
        estimated_minutes: row.get(9)?,
        priority: row.get(10)?,
        status: TaskStatus::parse_or(&row.get::<_, String>(11)?, TaskStatus::Planned),
        completed_at: row.get(12)?,
        notes: row.get(13)?,
        source_template_id: row.get(14)?,
        created_at: row.get(15)?,
    })
}

const TASK_COLS: &str = "id,plan_id,date,title,task_type,subject_id,chapter_id,start_time,end_time,estimated_minutes,priority,status,completed_at,notes,source_template_id,created_at";

pub fn tasks_for_date(conn: &Connection, plan_id: &str, date: NaiveDate) -> AppResult<Vec<Task>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TASK_COLS} FROM tasks WHERE plan_id=?1 AND date=?2 ORDER BY start_time, created_at"
    ))?;
    let rows = stmt.query_map(params![plan_id, date.to_string()], row_to_task)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn tasks_for_range(
    conn: &Connection,
    plan_id: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> AppResult<Vec<Task>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TASK_COLS} FROM tasks WHERE plan_id=?1 AND date BETWEEN ?2 AND ?3 ORDER BY date, start_time"
    ))?;
    let rows = stmt.query_map(params![plan_id, start.to_string(), end.to_string()], row_to_task)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn all_tasks_for_plan(conn: &Connection, plan_id: &str) -> AppResult<Vec<Task>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TASK_COLS} FROM tasks WHERE plan_id=?1 ORDER BY date, start_time"
    ))?;
    let rows = stmt.query_map([plan_id], row_to_task)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn get_task(conn: &Connection, id: &str) -> AppResult<Task> {
    conn.query_row(
        &format!("SELECT {TASK_COLS} FROM tasks WHERE id=?1"),
        [id],
        row_to_task,
    )
    .optional()?
    .ok_or_else(|| AppError::NotFound(format!("task '{id}'")))
}

pub fn insert_task(conn: &Connection, t: &Task) -> AppResult<()> {
    conn.execute(
        "INSERT INTO tasks (id,plan_id,date,title,task_type,subject_id,chapter_id,start_time,end_time,estimated_minutes,priority,status,completed_at,notes,source_template_id,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        params![
            t.id,
            t.plan_id,
            t.date.to_string(),
            t.title,
            t.task_type.as_str(),
            t.subject_id,
            t.chapter_id,
            t.start_time.format("%H:%M").to_string(),
            t.end_time.format("%H:%M").to_string(),
            t.estimated_minutes,
            t.priority,
            t.status.as_str(),
            t.completed_at,
            t.notes,
            t.source_template_id,
            if t.created_at.is_empty() { stamp() } else { t.created_at.clone() }
        ],
    )?;
    Ok(())
}

pub fn update_task(conn: &Connection, t: &Task) -> AppResult<()> {
    conn.execute(
        "UPDATE tasks SET date=?2,title=?3,task_type=?4,subject_id=?5,chapter_id=?6,start_time=?7,
         end_time=?8,estimated_minutes=?9,priority=?10,status=?11,completed_at=?12,notes=?13 WHERE id=?1",
        params![
            t.id,
            t.date.to_string(),
            t.title,
            t.task_type.as_str(),
            t.subject_id,
            t.chapter_id,
            t.start_time.format("%H:%M").to_string(),
            t.end_time.format("%H:%M").to_string(),
            t.estimated_minutes,
            t.priority,
            t.status.as_str(),
            t.completed_at,
            t.notes
        ],
    )?;
    Ok(())
}

pub fn delete_task(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM tasks WHERE id=?1", [id])?;
    Ok(())
}

pub fn backlog_tasks(conn: &Connection, plan_id: &str) -> AppResult<Vec<Task>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TASK_COLS} FROM tasks WHERE plan_id=?1 AND task_type='backlog'
         AND status NOT IN ('completed','skipped') ORDER BY date, start_time"
    ))?;
    let rows = stmt.query_map([plan_id], row_to_task)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// ------------------------------------------------------------ day overrides

pub fn day_overrides_for_plan(conn: &Connection, plan_id: &str) -> AppResult<Vec<DayOverride>> {
    let mut stmt = conn.prepare("SELECT plan_id,date,day_type,note FROM day_overrides WHERE plan_id=?1")?;
    let rows = stmt.query_map([plan_id], |row| {
        Ok(DayOverride {
            plan_id: row.get(0)?,
            date: parse_date(&row.get::<_, String>(1)?).unwrap_or_default(),
            day_type: DayType::parse_or(&row.get::<_, String>(2)?, DayType::Weekday),
            note: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn set_day_override(conn: &Connection, o: &DayOverride) -> AppResult<()> {
    conn.execute(
        "INSERT INTO day_overrides (plan_id,date,day_type,note) VALUES (?1,?2,?3,?4)
         ON CONFLICT(plan_id,date) DO UPDATE SET day_type=excluded.day_type, note=excluded.note",
        params![o.plan_id, o.date.to_string(), o.day_type.as_str(), o.note],
    )?;
    Ok(())
}

pub fn delete_day_override(conn: &Connection, plan_id: &str, date: NaiveDate) -> AppResult<()> {
    conn.execute(
        "DELETE FROM day_overrides WHERE plan_id=?1 AND date=?2",
        params![plan_id, date.to_string()],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- sessions

fn row_to_session(row: &Row<'_>) -> rusqlite::Result<FocusSession> {
    Ok(FocusSession {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        task_id: row.get(2)?,
        subject_id: row.get(3)?,
        chapter_id: row.get(4)?,
        mode: row.get(5)?,
        started_at: row.get(6)?,
        on_date: parse_date(&row.get::<_, String>(7)?).unwrap_or_default(),
        ended_at: row.get(8)?,
        focused_seconds: row.get(9)?,
        break_seconds: row.get(10)?,
        interruptions: row.get(11)?,
        status: row.get(12)?,
        notes: row.get(13)?,
    })
}

const SESSION_COLS: &str = "id,plan_id,task_id,subject_id,chapter_id,mode,started_at,on_date,ended_at,focused_seconds,break_seconds,interruptions,status,notes";

pub fn insert_session(conn: &Connection, s: &FocusSession) -> AppResult<()> {
    conn.execute(
        "INSERT INTO study_sessions (id,plan_id,task_id,subject_id,chapter_id,mode,started_at,on_date,ended_at,focused_seconds,break_seconds,interruptions,status,notes)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            s.id,
            s.plan_id,
            s.task_id,
            s.subject_id,
            s.chapter_id,
            s.mode,
            s.started_at,
            s.on_date.to_string(),
            s.ended_at,
            s.focused_seconds,
            s.break_seconds,
            s.interruptions,
            s.status,
            s.notes
        ],
    )?;
    Ok(())
}

pub fn sessions_for_date(conn: &Connection, plan_id: &str, date: NaiveDate) -> AppResult<Vec<FocusSession>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SESSION_COLS} FROM study_sessions WHERE plan_id=?1 AND on_date=?2 ORDER BY started_at DESC"
    ))?;
    let rows = stmt.query_map(params![plan_id, date.to_string()], row_to_session)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn sessions_for_range(
    conn: &Connection,
    plan_id: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> AppResult<Vec<FocusSession>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SESSION_COLS} FROM study_sessions WHERE plan_id=?1 AND on_date BETWEEN ?2 AND ?3 ORDER BY started_at"
    ))?;
    let rows = stmt.query_map(params![plan_id, start.to_string(), end.to_string()], row_to_session)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn all_sessions_for_plan(conn: &Connection, plan_id: &str) -> AppResult<Vec<FocusSession>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SESSION_COLS} FROM study_sessions WHERE plan_id=?1 ORDER BY started_at"
    ))?;
    let rows = stmt.query_map([plan_id], row_to_session)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn recent_sessions(conn: &Connection, plan_id: &str, limit: i64) -> AppResult<Vec<FocusSession>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SESSION_COLS} FROM study_sessions WHERE plan_id=?1 ORDER BY started_at DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![plan_id, limit], row_to_session)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn session_dates(conn: &Connection, plan_id: &str) -> AppResult<Vec<NaiveDate>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT on_date FROM study_sessions WHERE plan_id=?1 AND focused_seconds>0 ORDER BY on_date",
    )?;
    let rows = stmt.query_map([plan_id], |r| r.get::<_, String>(0))?;
    Ok(rows
        .filter_map(|r| r.ok())
        .filter_map(|s| parse_date(&s).ok())
        .collect())
}

// --------------------------------------------------------------- revisions

fn row_to_revision(row: &Row<'_>) -> rusqlite::Result<Revision> {
    Ok(Revision {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        chapter_id: row.get(2)?,
        due_date: parse_date(&row.get::<_, String>(3)?).unwrap_or_default(),
        interval_days: row.get(4)?,
        ease: row.get(5)?,
        state: RevisionState::parse_or(&row.get::<_, String>(6)?, RevisionState::Pending),
        result: row
            .get::<_, Option<String>>(7)?
            .and_then(|s| RevisionResult::from_str(&s)),
        completed_at: row.get(8)?,
        snoozed_until: date_opt(row, 9)?,
        created_at: row.get(10)?,
    })
}

const REVISION_COLS: &str = "id,plan_id,chapter_id,due_date,interval_days,ease,state,result,completed_at,snoozed_until,created_at";

pub fn revisions_for_plan(conn: &Connection, plan_id: &str) -> AppResult<Vec<Revision>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {REVISION_COLS} FROM revisions WHERE plan_id=?1 ORDER BY due_date"
    ))?;
    let rows = stmt.query_map([plan_id], row_to_revision)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn pending_revisions_for_chapter(conn: &Connection, chapter_id: &str) -> AppResult<Vec<Revision>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {REVISION_COLS} FROM revisions WHERE chapter_id=?1 AND state IN ('pending','snoozed') ORDER BY due_date"
    ))?;
    let rows = stmt.query_map([chapter_id], row_to_revision)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn insert_revision(conn: &Connection, r: &Revision) -> AppResult<()> {
    conn.execute(
        "INSERT INTO revisions (id,plan_id,chapter_id,due_date,interval_days,ease,state,result,completed_at,snoozed_until,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            r.id,
            r.plan_id,
            r.chapter_id,
            r.due_date.to_string(),
            r.interval_days,
            r.ease,
            r.state.as_str(),
            r.result.map(|x| x.as_str()),
            r.completed_at,
            r.snoozed_until.map(|d| d.to_string()),
            if r.created_at.is_empty() { stamp() } else { r.created_at.clone() }
        ],
    )?;
    Ok(())
}

pub fn update_revision(conn: &Connection, r: &Revision) -> AppResult<()> {
    conn.execute(
        "UPDATE revisions SET due_date=?2,interval_days=?3,ease=?4,state=?5,result=?6,completed_at=?7,snoozed_until=?8 WHERE id=?1",
        params![
            r.id,
            r.due_date.to_string(),
            r.interval_days,
            r.ease,
            r.state.as_str(),
            r.result.map(|x| x.as_str()),
            r.completed_at,
            r.snoozed_until.map(|d| d.to_string())
        ],
    )?;
    Ok(())
}

// ------------------------------------------------------ mock tests/mistakes

fn row_to_mock(row: &Row<'_>) -> rusqlite::Result<MockTest> {
    Ok(MockTest {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        subject_id: row.get(2)?,
        name: row.get(3)?,
        date: parse_date(&row.get::<_, String>(4)?).unwrap_or_default(),
        score: row.get(5)?,
        max_marks: row.get(6)?,
        duration_minutes: row.get(7)?,
        weak_topics: row.get(8)?,
        notes: row.get(9)?,
        created_at: row.get(10)?,
    })
}

pub fn mock_tests_for_plan(conn: &Connection, plan_id: &str) -> AppResult<Vec<MockTest>> {
    let mut stmt = conn.prepare(
        "SELECT id,plan_id,subject_id,name,date,score,max_marks,duration_minutes,weak_topics,notes,created_at
         FROM mock_tests WHERE plan_id=?1 ORDER BY date DESC",
    )?;
    let rows = stmt.query_map([plan_id], row_to_mock)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn insert_mock_test(conn: &Connection, m: &MockTest) -> AppResult<()> {
    conn.execute(
        "INSERT INTO mock_tests (id,plan_id,subject_id,name,date,score,max_marks,duration_minutes,weak_topics,notes,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            m.id,
            m.plan_id,
            m.subject_id,
            m.name,
            m.date.to_string(),
            m.score,
            m.max_marks,
            m.duration_minutes,
            m.weak_topics,
            m.notes,
            if m.created_at.is_empty() { stamp() } else { m.created_at.clone() }
        ],
    )?;
    Ok(())
}

pub fn delete_mock_test(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM mock_tests WHERE id=?1", [id])?;
    Ok(())
}

fn row_to_mistake(row: &Row<'_>) -> rusqlite::Result<Mistake> {
    Ok(Mistake {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        mock_test_id: row.get(2)?,
        subject_id: row.get(3)?,
        chapter_id: row.get(4)?,
        question: row.get(5)?,
        explanation: row.get(6)?,
        resolved: b(row.get(7)?),
        created_at: row.get(8)?,
    })
}

pub fn mistakes_for_plan(conn: &Connection, plan_id: &str) -> AppResult<Vec<Mistake>> {
    let mut stmt = conn.prepare(
        "SELECT id,plan_id,mock_test_id,subject_id,chapter_id,question,explanation,resolved,created_at
         FROM mistakes WHERE plan_id=?1 ORDER BY resolved, created_at DESC",
    )?;
    let rows = stmt.query_map([plan_id], row_to_mistake)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn insert_mistake(conn: &Connection, m: &Mistake) -> AppResult<()> {
    conn.execute(
        "INSERT INTO mistakes (id,plan_id,mock_test_id,subject_id,chapter_id,question,explanation,resolved,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            m.id,
            m.plan_id,
            m.mock_test_id,
            m.subject_id,
            m.chapter_id,
            m.question,
            m.explanation,
            m.resolved as i64,
            if m.created_at.is_empty() { stamp() } else { m.created_at.clone() }
        ],
    )?;
    Ok(())
}

pub fn set_mistake_resolved(conn: &Connection, id: &str, resolved: bool) -> AppResult<()> {
    conn.execute(
        "UPDATE mistakes SET resolved=?2 WHERE id=?1",
        params![id, resolved as i64],
    )?;
    Ok(())
}

// ------------------------------------------------------------------- sleep

fn row_to_sleep(row: &Row<'_>) -> rusqlite::Result<SleepEntry> {
    Ok(SleepEntry {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        kind: SleepKind::parse_or(&row.get::<_, String>(2)?, SleepKind::Night),
        start_at: row.get(3)?,
        end_at: row.get(4)?,
        energy: row.get(5)?,
        notes: row.get(6)?,
        created_at: row.get(7)?,
    })
}

pub fn sleep_entries_for_plan(conn: &Connection, plan_id: &str, limit: i64) -> AppResult<Vec<SleepEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id,plan_id,kind,start_at,end_at,energy,notes,created_at FROM sleep_entries
         WHERE plan_id=?1 ORDER BY start_at DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![plan_id, limit], row_to_sleep)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn insert_sleep(conn: &Connection, s: &SleepEntry) -> AppResult<()> {
    conn.execute(
        "INSERT INTO sleep_entries (id,plan_id,kind,start_at,end_at,energy,notes,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            s.id,
            s.plan_id,
            s.kind.as_str(),
            s.start_at,
            s.end_at,
            s.energy,
            s.notes,
            if s.created_at.is_empty() { stamp() } else { s.created_at.clone() }
        ],
    )?;
    Ok(())
}

pub fn delete_sleep(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM sleep_entries WHERE id=?1", [id])?;
    Ok(())
}

// -------------------------------------------------------------- timer state

#[derive(Debug, Clone, Default)]
pub struct TimerRow {
    pub session_id: String,
    pub plan_id: String,
    pub task_id: Option<String>,
    pub subject_id: Option<String>,
    pub chapter_id: Option<String>,
    pub mode: String,
    pub phase: String,
    pub state: String,
    pub started_at: String,
    pub accumulated_ms: i64,
    pub break_accumulated_ms: i64,
    pub planned_focus_secs: i64,
    pub planned_break_secs: i64,
    pub interruptions: i64,
    pub last_checkpoint: String,
}

pub fn load_timer(conn: &Connection) -> AppResult<Option<TimerRow>> {
    Ok(conn
        .query_row(
            "SELECT session_id,COALESCE(plan_id,''),task_id,subject_id,chapter_id,COALESCE(mode,''),COALESCE(phase,''),COALESCE(state,''),
                    COALESCE(started_at,''),accumulated_ms,break_accumulated_ms,planned_focus_secs,planned_break_secs,interruptions,
                    COALESCE(last_checkpoint,'')
             FROM timer_state WHERE id=1",
            [],
            |r| {
                Ok(TimerRow {
                    session_id: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    plan_id: r.get(1)?,
                    task_id: r.get(2)?,
                    subject_id: r.get(3)?,
                    chapter_id: r.get(4)?,
                    mode: r.get(5)?,
                    phase: r.get(6)?,
                    state: r.get(7)?,
                    started_at: r.get(8)?,
                    accumulated_ms: r.get(9)?,
                    break_accumulated_ms: r.get(10)?,
                    planned_focus_secs: r.get(11)?,
                    planned_break_secs: r.get(12)?,
                    interruptions: r.get(13)?,
                    last_checkpoint: r.get(14)?,
                })
            },
        )
        .optional()?)
}

pub fn save_timer(conn: &Connection, t: &TimerRow) -> AppResult<()> {
    conn.execute(
        "INSERT INTO timer_state (id,session_id,plan_id,task_id,subject_id,chapter_id,mode,phase,state,started_at,
             accumulated_ms,break_accumulated_ms,planned_focus_secs,planned_break_secs,interruptions,last_checkpoint)
         VALUES (1,?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
         ON CONFLICT(id) DO UPDATE SET session_id=excluded.session_id,plan_id=excluded.plan_id,
             task_id=excluded.task_id,subject_id=excluded.subject_id,chapter_id=excluded.chapter_id,
             mode=excluded.mode,phase=excluded.phase,state=excluded.state,started_at=excluded.started_at,
             accumulated_ms=excluded.accumulated_ms,break_accumulated_ms=excluded.break_accumulated_ms,
             planned_focus_secs=excluded.planned_focus_secs,planned_break_secs=excluded.planned_break_secs,
             interruptions=excluded.interruptions,last_checkpoint=excluded.last_checkpoint",
        params![
            t.session_id,
            t.plan_id,
            t.task_id,
            t.subject_id,
            t.chapter_id,
            t.mode,
            t.phase,
            t.state,
            t.started_at,
            t.accumulated_ms,
            t.break_accumulated_ms,
            t.planned_focus_secs,
            t.planned_break_secs,
            t.interruptions,
            t.last_checkpoint
        ],
    )?;
    Ok(())
}

pub fn clear_timer(conn: &Connection) -> AppResult<()> {
    conn.execute("DELETE FROM timer_state WHERE id=1", [])?;
    Ok(())
}
