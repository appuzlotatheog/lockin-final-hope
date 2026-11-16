//! First-run seed data: the Kerala Plus One Christmas 2026 plan.
//!
//! The seed is inserted exactly once (guarded by an empty `plans` table) and is
//! ordinary user data afterwards — none of it is special-cased in the UI.

use crate::models::time::{DEFAULT_TZ, minutes_between, parse_time, date_range};
use crate::models::*;
use crate::scheduling;
use crate::error::AppResult;
use chrono::{Duration, NaiveDate, Utc};
use rusqlite::{params, Connection};

pub const SEED_PLAN_ID: &str = "kerala-plus-one-christmas-2026";

pub fn seed_if_empty(conn: &mut Connection) -> AppResult<()> {
    let plans: i64 = conn.query_row("SELECT COUNT(*) FROM plans", [], |r| r.get(0))?;
    if plans == 0 {
        let tx = conn.transaction()?;
        seed_kerala_plan(&tx)?;
        tx.commit()?;
        tracing::info!("seeded default Christmas 2026 plan");
    }
    crate::db::repo::ensure_setting_defaults(conn)?;
    Ok(())
}

struct SeedSubject {
    id: &'static str,
    name: &'static str,
    color: &'static str,
    chapters: &'static [&'static str],
    pending: bool,
}

const SUBJECTS: &[SeedSubject] = &[
    SeedSubject {
        id: "sub-physics",
        name: "Physics",
        color: "#9B7BFF",
        chapters: &[
            "Units and Measurements",
            "Motion in a Straight Line",
            "Motion in a Plane",
            "Laws of Motion",
            "Work, Energy and Power",
            "System of Particles and Rotational Motion",
            "Gravitation",
            "Thermodynamics",
            "Mechanical Properties of Solids",
            "Mechanical Properties of Fluids",
            "Thermal Properties of Matter",
        ],
        pending: false,
    },
    SeedSubject {
        id: "sub-chemistry",
        name: "Chemistry",
        color: "#69D6A6",
        chapters: &[
            "Some Basic Concepts of Chemistry",
            "Redox Reactions",
            "Equilibrium",
            "Thermodynamics",
            "Chemical Bonding and Molecular Structure",
            "Classification of Elements and Periodicity in Properties",
            "Structure of Atom",
        ],
        pending: false,
    },
    SeedSubject {
        id: "sub-mathematics",
        name: "Mathematics",
        color: "#EABF73",
        chapters: &[
            "Sets",
            "Relations and Functions",
            "Trigonometric Functions",
            "Principle of Mathematical Induction",
            "Introduction to Three Dimensional Geometry",
            "Conic Sections",
            "Straight Lines",
            "Sequences and Series",
            "Binomial Theorem",
            "Permutations and Combinations",
            "Linear Inequalities",
            "Complex Numbers and Quadratic Equations",
        ],
        pending: false,
    },
    SeedSubject {
        id: "sub-computer-science",
        name: "Computer Science",
        color: "#7FB2E6",
        chapters: &[
            "Introduction to Computing",
            "Data Representation and Boolean Algebra",
            "Computer System Organisation",
            "Getting Started with Python",
            "Python Fundamentals",
            "Control Statements",
            "Functions and Modules",
            "Strings in Python",
            "Lists",
        ],
        pending: false,
    },
    SeedSubject {
        id: "sub-english",
        name: "English",
        color: "#E67F90",
        chapters: &[
            "Fufi",
            "So Much Happiness",
            "An Angel in Disguise",
            "A Draught of Kindness",
            "Ozymandias",
            "A Retrieved Reformation",
            "Classic Cars: Poetry on Wheels",
            "The Window is Blurred",
            "Women's World",
            "What a Wonderful World",
            "Let's Live with Them",
            "Radha, Just Radha",
            "Cat in an Empty Apartment",
        ],
        pending: false,
    },
    SeedSubject {
        id: "sub-hindi",
        name: "Hindi",
        color: "#C6A6FF",
        chapters: &[],
        pending: true,
    },
];

const EXAMS: &[(&str, &str, &str, &str, i64)] = &[
    ("exam-english", "sub-english", "English (Part I)", "2026-12-09", 810),
    ("exam-physics", "sub-physics", "Physics", "2026-12-10", 810),
    ("exam-computer-science", "sub-computer-science", "Computer Science", "2026-12-11", 840),
    ("exam-mathematics", "sub-mathematics", "Mathematics", "2026-12-14", 570),
    ("exam-chemistry", "sub-chemistry", "Chemistry", "2026-12-15", 810),
    ("exam-hindi", "sub-hindi", "Hindi (Part II Language)", "2026-12-17", 810),
];

struct SlotSeed {
    start: &'static str,
    end: &'static str,
    kind: SlotKind,
    title: &'static str,
    subject: Option<&'static str>,
}

const WEEKDAY_SLOTS: &[SlotSeed] = &[
    SlotSeed { start: "05:15", end: "06:15", kind: SlotKind::Study, title: "Mathematics / Physics study", subject: None },
    SlotSeed { start: "19:00", end: "20:15", kind: SlotKind::Study, title: "Main subject: Physics / Chemistry", subject: None },
    SlotSeed { start: "20:15", end: "20:25", kind: SlotKind::Break, title: "Break", subject: None },
    SlotSeed { start: "20:25", end: "21:10", kind: SlotKind::Study, title: "Computer Science", subject: Some("sub-computer-science") },
    SlotSeed { start: "21:10", end: "21:40", kind: SlotKind::Meal, title: "Dinner", subject: None },
    SlotSeed { start: "21:40", end: "22:30", kind: SlotKind::Study, title: "English / Hindi / revision", subject: None },
];

const SATURDAY_SLOTS: &[SlotSeed] = &[
    SlotSeed { start: "07:30", end: "09:00", kind: SlotKind::Study, title: "Mathematics", subject: Some("sub-mathematics") },
    SlotSeed { start: "09:30", end: "11:00", kind: SlotKind::Study, title: "Physics / Chemistry", subject: None },
    SlotSeed { start: "11:30", end: "13:00", kind: SlotKind::Study, title: "Physics / Chemistry", subject: None },
    SlotSeed { start: "14:30", end: "15:30", kind: SlotKind::Study, title: "Computer Science", subject: Some("sub-computer-science") },
    SlotSeed { start: "16:00", end: "16:50", kind: SlotKind::Study, title: "English / Hindi", subject: None },
    SlotSeed { start: "19:00", end: "19:45", kind: SlotKind::Study, title: "Backlog", subject: None },
];

const SUNDAY_SLOTS: &[SlotSeed] = &[
    SlotSeed { start: "07:30", end: "09:00", kind: SlotKind::Study, title: "Weekly test", subject: None },
    SlotSeed { start: "09:30", end: "11:00", kind: SlotKind::Study, title: "Test corrections", subject: None },
    SlotSeed { start: "11:30", end: "13:00", kind: SlotKind::Study, title: "Chemistry / revision", subject: Some("sub-chemistry") },
    SlotSeed { start: "14:30", end: "15:30", kind: SlotKind::Study, title: "Computer Science practice", subject: Some("sub-computer-science") },
    SlotSeed { start: "16:00", end: "16:50", kind: SlotKind::Study, title: "English / Hindi", subject: None },
    SlotSeed { start: "19:00", end: "19:45", kind: SlotKind::Study, title: "Weekly review and planning", subject: None },
];

fn seed_kerala_plan(conn: &Connection) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO plans (id,name,academic_year,timezone,start_date,end_date,archived,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,0,?7)",
        params![
            SEED_PLAN_ID,
            "Plus One Christmas Examinations 2026",
            "2026–27",
            DEFAULT_TZ,
            "2026-11-16",
            "2026-12-17",
            now
        ],
    )?;

    for (s_idx, subject) in SUBJECTS.iter().enumerate() {
        conn.execute(
            "INSERT INTO subjects (id,plan_id,name,color,sort_order,portions_pending)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                subject.id,
                SEED_PLAN_ID,
                subject.name,
                subject.color,
                s_idx as i64,
                subject.pending as i64
            ],
        )?;
        let prefix = subject.id.trim_start_matches("sub-");
        for (c_idx, title) in subject.chapters.iter().enumerate() {
            conn.execute(
                "INSERT INTO chapters (id,subject_id,title,sort_order)
                 VALUES (?1,?2,?3,?4)",
                params![
                    format!("{prefix}-{:02}", c_idx + 1),
                    subject.id,
                    title,
                    c_idx as i64
                ],
            )?;
        }
    }

    for (idx, (id, subject, label, date, minutes)) in EXAMS.iter().enumerate() {
        let start = chrono::NaiveTime::from_num_seconds_from_midnight(*minutes as u32 * 60, 0);
        let session = if start.format("%H").to_string().parse::<u32>().unwrap_or(0) < 12 {
            "Morning"
        } else {
            "Afternoon"
        };
        conn.execute(
            "INSERT INTO exams (id,plan_id,subject_id,label,date,start_time,session,duration_minutes)
             VALUES (?1,?2,?3,?4,?5,?6,?7,150)",
            params![
                id,
                SEED_PLAN_ID,
                subject,
                label,
                date,
                crate::models::time::fmt_time(start),
                session
            ],
        )?;
        tracing::debug!("seeded exam {} at {}", id, idx);
    }

    let templates = [
        ("tpl-weekday", "School day", vec![DayType::Weekday], WEEKDAY_SLOTS),
        ("tpl-saturday", "Saturday", vec![DayType::Saturday], SATURDAY_SLOTS),
        ("tpl-sunday", "Sunday", vec![DayType::Sunday], SUNDAY_SLOTS),
    ];
    for (t_idx, (tpl_id, name, days, slots)) in templates.iter().enumerate() {
        let days_json = serde_json::to_string(days).unwrap_or_else(|_| "[]".into());
        conn.execute(
            "INSERT INTO schedule_templates (id,plan_id,name,day_types,sort_order)
             VALUES (?1,?2,?3,?4,?5)",
            params![tpl_id, SEED_PLAN_ID, name, days_json, t_idx as i64],
        )?;
        for (s_idx, slot) in slots.iter().enumerate() {
            conn.execute(
                "INSERT INTO template_slots (id,template_id,start_time,end_time,kind,title,subject_id,sort_order)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    format!("{tpl_id}-slot-{}", s_idx + 1),
                    tpl_id,
                    slot.start,
                    slot.end,
                    slot.kind.as_str(),
                    slot.title,
                    slot.subject,
                    s_idx as i64
                ],
            )?;
        }
    }

    // Materialise planned tasks for the seeded preparation window so the
    // timetable is immediately useful. These are normal editable tasks; no
    // study history is invented.
    let start = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 12, 17).unwrap();
    let plan = Plan {
        id: SEED_PLAN_ID.into(),
        name: "Plus One Christmas Examinations 2026".into(),
        academic_year: "2026–27".into(),
        timezone: DEFAULT_TZ.into(),
        start_date: start,
        end_date: end,
        archived: false,
        created_at: String::new(),
    };
    let templates = crate::db::repo::templates_for_plan(conn, SEED_PLAN_ID)?;
    let overrides = crate::db::repo::day_overrides_for_plan(conn, SEED_PLAN_ID)?;
    for date in date_range(start, end) {
        let generated = scheduling::planned_tasks_for_date(&plan, date, &templates, &overrides);
        for task in generated {
            crate::db::repo::insert_task(conn, &task)?;
        }
    }

    Ok(())
}

/// Used by tests and by the plan wizard to derive estimated minutes from a
/// slot's start/end times.
pub fn estimate_minutes(slot: &SlotSeed) -> i64 {
    match (parse_time(slot.start), parse_time(slot.end)) {
        (Ok(a), Ok(b)) => minutes_between(a, b),
        _ => 0,
    }
}

/// Seed a brand-new exam season for the wizard (subjects/chapters/templates are
/// supplied by the caller). Kept here so tests can build realistic fixtures.
pub fn template_slots_for(day_type: DayType) -> Vec<(&'static str, &'static str, SlotKind, &'static str)> {
    let src = match day_type {
        DayType::Weekday => WEEKDAY_SLOTS,
        DayType::Saturday => SATURDAY_SLOTS,
        DayType::Sunday => SUNDAY_SLOTS,
        _ => WEEKDAY_SLOTS,
    };
    src.iter()
        .map(|s| (s.start, s.end, s.kind, s.title))
        .collect()
}

/// The default date window used when creating a plan in the wizard.
pub fn default_window() -> (NaiveDate, NaiveDate) {
    (NaiveDate::from_ymd_opt(2026, 11, 16).unwrap(), NaiveDate::from_ymd_opt(2026, 12, 17).unwrap())
}

pub fn season_end_plus_week(start: NaiveDate) -> NaiveDate {
    start + Duration::days(31)
}
