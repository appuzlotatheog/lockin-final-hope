//! Versioned schema migrations driven by `PRAGMA user_version`.
//!
//! Every migration runs inside one transaction. The database is never rebuilt
//! destructively: new migrations are appended and applied on launch.

use crate::error::AppResult;
use rusqlite::Connection;

const MIGRATIONS: &[&str] = &[
    // v1 — initial schema
    r#"
CREATE TABLE plans (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    academic_year TEXT NOT NULL DEFAULT '',
    timezone     TEXT NOT NULL DEFAULT 'Asia/Kolkata',
    start_date   TEXT NOT NULL,
    end_date     TEXT NOT NULL,
    archived     INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL
);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE subjects (
    id               TEXT PRIMARY KEY,
    plan_id          TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    name             TEXT NOT NULL,
    color            TEXT NOT NULL DEFAULT '#9B7BFF',
    sort_order       INTEGER NOT NULL DEFAULT 0,
    portions_pending INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE chapters (
    id                 TEXT PRIMARY KEY,
    subject_id         TEXT NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    title              TEXT NOT NULL,
    sort_order         INTEGER NOT NULL DEFAULT 0,
    status             TEXT NOT NULL DEFAULT 'not_started',
    concept_done       INTEGER NOT NULL DEFAULT 0,
    textbook_done      INTEGER NOT NULL DEFAULT 0,
    exercises_done     INTEGER NOT NULL DEFAULT 0,
    selftest_done      INTEGER NOT NULL DEFAULT 0,
    confidence         INTEGER,
    questions_attempted INTEGER NOT NULL DEFAULT 0,
    correct_answers    INTEGER NOT NULL DEFAULT 0,
    last_studied       TEXT,
    next_revision      TEXT,
    study_seconds      INTEGER NOT NULL DEFAULT 0,
    notes              TEXT NOT NULL DEFAULT ''
);

CREATE TABLE exams (
    id               TEXT PRIMARY KEY,
    plan_id          TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    subject_id       TEXT NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    label            TEXT NOT NULL,
    date             TEXT NOT NULL,
    start_time       TEXT NOT NULL,
    session          TEXT NOT NULL DEFAULT '',
    duration_minutes INTEGER NOT NULL DEFAULT 150
);

CREATE TABLE schedule_templates (
    id         TEXT PRIMARY KEY,
    plan_id    TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    day_types  TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE template_slots (
    id          TEXT PRIMARY KEY,
    template_id TEXT NOT NULL REFERENCES schedule_templates(id) ON DELETE CASCADE,
    start_time  TEXT NOT NULL,
    end_time    TEXT NOT NULL,
    kind        TEXT NOT NULL DEFAULT 'study',
    title       TEXT NOT NULL,
    subject_id  TEXT REFERENCES subjects(id) ON DELETE SET NULL,
    chapter_id  TEXT REFERENCES chapters(id) ON DELETE SET NULL,
    sort_order  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE tasks (
    id                 TEXT PRIMARY KEY,
    plan_id            TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    date               TEXT NOT NULL,
    title              TEXT NOT NULL,
    task_type          TEXT NOT NULL DEFAULT 'study',
    subject_id         TEXT REFERENCES subjects(id) ON DELETE SET NULL,
    chapter_id         TEXT REFERENCES chapters(id) ON DELETE SET NULL,
    start_time         TEXT NOT NULL,
    end_time           TEXT NOT NULL,
    estimated_minutes  INTEGER NOT NULL DEFAULT 30,
    priority           INTEGER NOT NULL DEFAULT 2,
    status             TEXT NOT NULL DEFAULT 'planned',
    completed_at       TEXT,
    notes              TEXT NOT NULL DEFAULT '',
    source_template_id TEXT,
    created_at         TEXT NOT NULL
);
CREATE INDEX idx_tasks_plan_date ON tasks(plan_id, date);
CREATE INDEX idx_tasks_date ON tasks(date);

CREATE TABLE day_overrides (
    plan_id  TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    date     TEXT NOT NULL,
    day_type TEXT NOT NULL,
    note     TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (plan_id, date)
);

CREATE TABLE study_sessions (
    id              TEXT PRIMARY KEY,
    plan_id         TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    task_id         TEXT,
    subject_id      TEXT,
    chapter_id      TEXT,
    mode            TEXT NOT NULL DEFAULT 'deep',
    started_at      TEXT NOT NULL,
    on_date         TEXT NOT NULL,
    ended_at        TEXT,
    focused_seconds INTEGER NOT NULL DEFAULT 0,
    break_seconds   INTEGER NOT NULL DEFAULT 0,
    interruptions   INTEGER NOT NULL DEFAULT 0,
    status          TEXT NOT NULL DEFAULT 'completed',
    notes           TEXT NOT NULL DEFAULT ''
);
CREATE INDEX idx_sessions_date ON study_sessions(on_date);
CREATE INDEX idx_sessions_plan ON study_sessions(plan_id, on_date);

CREATE TABLE timer_state (
    id                INTEGER PRIMARY KEY CHECK (id = 1),
    session_id        TEXT,
    plan_id           TEXT,
    task_id           TEXT,
    subject_id        TEXT,
    chapter_id        TEXT,
    mode              TEXT,
    phase             TEXT,
    state             TEXT,
    started_at        TEXT,
    accumulated_ms    INTEGER NOT NULL DEFAULT 0,
    break_accumulated_ms INTEGER NOT NULL DEFAULT 0,
    planned_focus_secs INTEGER NOT NULL DEFAULT 0,
    planned_break_secs INTEGER NOT NULL DEFAULT 0,
    interruptions     INTEGER NOT NULL DEFAULT 0,
    last_checkpoint   TEXT
);

CREATE TABLE revisions (
    id             TEXT PRIMARY KEY,
    plan_id        TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    chapter_id     TEXT NOT NULL REFERENCES chapters(id) ON DELETE CASCADE,
    due_date       TEXT NOT NULL,
    interval_days  INTEGER NOT NULL,
    ease           REAL NOT NULL DEFAULT 2.5,
    state          TEXT NOT NULL DEFAULT 'pending',
    result         TEXT,
    completed_at   TEXT,
    snoozed_until  TEXT,
    created_at     TEXT NOT NULL
);
CREATE INDEX idx_revisions_due ON revisions(plan_id, state, due_date);

CREATE TABLE mock_tests (
    id               TEXT PRIMARY KEY,
    plan_id          TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    subject_id       TEXT NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    name             TEXT NOT NULL,
    date             TEXT NOT NULL,
    score            REAL NOT NULL DEFAULT 0,
    max_marks        REAL NOT NULL DEFAULT 0,
    duration_minutes INTEGER NOT NULL DEFAULT 0,
    weak_topics      TEXT NOT NULL DEFAULT '',
    notes            TEXT NOT NULL DEFAULT '',
    created_at       TEXT NOT NULL
);

CREATE TABLE mistakes (
    id           TEXT PRIMARY KEY,
    plan_id      TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    mock_test_id TEXT REFERENCES mock_tests(id) ON DELETE SET NULL,
    subject_id   TEXT NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    chapter_id   TEXT REFERENCES chapters(id) ON DELETE SET NULL,
    question     TEXT NOT NULL,
    explanation  TEXT NOT NULL DEFAULT '',
    resolved     INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL
);

CREATE TABLE sleep_entries (
    id         TEXT PRIMARY KEY,
    plan_id    TEXT NOT NULL REFERENCES plans(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL DEFAULT 'night',
    start_at   TEXT NOT NULL,
    end_at     TEXT NOT NULL,
    energy     INTEGER,
    notes      TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);
CREATE INDEX idx_sleep_start ON sleep_entries(start_at);
"#,
];

pub fn migrate(conn: &mut Connection) -> AppResult<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let target = MIGRATIONS.len() as i64;
    if version > target {
        return Err(crate::error::AppError::validation(format!(
            "This database was created by a newer version of Lockin (schema v{version}, app supports v{target}). Update the app instead of downgrading."
        )));
    }
    for (idx, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (idx + 1) as i64)?;
        tx.commit()?;
        tracing::info!("applied schema migration v{}", idx + 1);
    }
    Ok(())
}
