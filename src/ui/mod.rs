//! Slint bridge. UI callbacks forward ids and values into services; all
//! business logic stays out of the declarative layer.

use crate::db::{repo, Database};
use crate::error::{AppError, AppResult};
use crate::models::time;
use crate::models::*;
use crate::services;
use crate::timer::{FocusTimer, Preset};
use crate::AppWindow;
use chrono::{Datelike, NaiveDate, NaiveTime, TimeZone, Utc};
use slint::{ComponentHandle, Model, ModelRc, VecModel, Weak};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};
use crate::import_export::apply::ImportMode;
use crate::import_export::backup::BackupFile;
use crate::import_export::validate::PreparedPlan;
use crate::services::wizard::WizardDraft;

pub struct AppState {
    pub db: Database,
    pub settings: Settings,
    pub plan_id: String,
    pub timer: FocusTimer,
    pub recovered: bool,
    pub ui: Weak<AppWindow>,
    toast_seq: Rc<Cell<u64>>,
    /// Selected date on the timetable (and the page the next refresh uses).
    pub selected_date: NaiveDate,
    pub selected_subject: Option<String>,
    pub selected_chapter: Option<String>,
    pub focus_subject: Option<String>,
    pub focus_chapter: Option<String>,
    pub editing_template: Option<String>,
    pub editor_subjects: Vec<Subject>,
    pub editor_chapters: Vec<Chapter>,
    pub focus_subjects: Vec<Subject>,
    pub focus_chapters: Vec<Chapter>,
    pub mistake_subjects: Vec<Subject>,
    pub mistake_chapters: Vec<Chapter>,
    pub import_preview: Option<PreparedPlan>,
    pub import_file: Option<PathBuf>,
    pub backup_pending: Option<BackupFile>,
    pub replan: Option<crate::scheduling::ReplanOutcome>,
    pub wizard: WizardDraft,
    pub last_autosave: Instant,
    pub exam_mode_override: bool,
}

const PAGE_TITLES: [&str; 9] = [
    "Overview",
    "Timetable & plan",
    "Syllabus",
    "Focus timer",
    "Revision queue",
    "Exams",
    "Analytics",
    "Import / Export",
    "Settings",
];

fn page_context(page: i32, plan_name: &str, tz: &str) -> String {
    let tail = format!("{plan_name} · {tz} · local only");
    match page {
        0 => format!("Your day at a glance · {tail}"),
        1 => format!("Tasks, week grid and templates · {tail}"),
        2 => format!("Chapter checklists, confidence and revision hooks · {tail}"),
        3 => format!("Monotonic timer · pauses never count · {tail}"),
        4 => format!("1 / 3 / 7-day policy with adaptive intervals · {tail}"),
        5 => format!("Exam countdown, mock tests and mistakes · {tail}"),
        6 => format!("Real focus sessions and completion · {tail}"),
        7 => format!("Lockin Plan Format · validated, versioned, offline · {tail}"),
        8 => format!("Preferences, plans and backups · {tail}"),
        _ => tail,
    }
}

fn plan_bundle(state: &AppState) -> AppResult<PlanBundle> {
    services::plan::load_bundle(&state.db.conn, &state.plan_id)
}

fn color(hex: &str) -> slint::Color {
    let s = hex.trim().trim_start_matches('#');
    if s.len() >= 6 {
        let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(130);
        let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(125);
        let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(160);
        slint::Color::from_rgb_u8(r, g, b)
    } else {
        slint::Color::from_rgb_u8(155, 123, 255)
    }
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

fn show_toast(ui: &AppWindow, state: &AppState, message: impl Into<String>, tone: i32) {
    let shell = ui.global::<crate::Shell>();
    shell.set_toast_text(message.into().into());
    shell.set_toast_tone(tone);
    shell.set_toast_open(true);
    let seq = state.toast_seq.get() + 1;
    state.toast_seq.set(seq);
    let shared = state.toast_seq.clone();
    let weak = ui.as_weak();
    slint::Timer::single_shot(Duration::from_secs(4), move || {
        if shared.get() == seq {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::Shell>().set_toast_open(false);
            }
        }
    });
}

fn report(ui: &AppWindow, state: &AppState, result: AppResult<()>) {
    if let Err(err) = result {
        show_toast(ui, state, err.to_string(), 2);
    }
}

fn to_slint_agenda(rows: &[services::dashboard::AgendaRow], date: NaiveDate) -> Vec<crate::AgendaItem> {
    rows.iter()
        .map(|r| crate::AgendaItem {
            id: r.id.clone().into(),
            title: r.title.clone().into(),
            subject: r.subject.clone().into(),
            tint: color(&r.color),
            start: r.start_text.clone().into(),
            end: r.end_text.clone().into(),
            status: r.status.clone().into(),
            status_index: r.status_index,
            is_fixed: r.is_fixed,
            is_break: r.is_break,
            kind: r.task_type.clone().into(),
            minutes: r.minutes as i32,
            chapter_id: r.chapter_id.clone().into(),
            notes: r.notes.clone().into(),
            date: date.to_string().into(),
        })
        .collect()
}

fn refresh_shell(ui: &AppWindow, state: &mut AppState) -> AppResult<()> {
    let bundle = plan_bundle(state)?;
    let shell = ui.global::<crate::Shell>();
    shell.set_plan_name(bundle.plan.name.clone().into());
    shell.set_plan_id(bundle.plan.id.clone().into());
    shell.set_plan_span(
        format!(
            "{} – {}",
            time::fmt_date_short(bundle.plan.start_date),
            time::fmt_date_short(bundle.plan.end_date)
        )
        .into(),
    );
    let plans = repo::list_plans(&state.db.conn, false)?;
    shell.set_plan_names(model(plans.iter().map(|p| p.name.clone().into()).collect()));
    if let Some(idx) = plans.iter().position(|p| p.id == state.plan_id) {
        shell.set_plan_index(idx as i32);
    }
    let today = time::today_in(&bundle.plan.timezone);
    shell.set_today_text(
        format!("{} · {}", time::fmt_date(today), bundle.plan.timezone).into(),
    );
    shell.set_version(env!("CARGO_PKG_VERSION").into());
    shell.set_status_text(
        format!("Local · SQLite · {}", bundle.plan.timezone).into(),
    );
    let now = time::now_in(&bundle.plan.timezone);
    let at = bundle
        .exams
        .iter()
        .map(|e| time::at_local(&bundle.plan.timezone, e.date, e.start_time))
        .filter(|at| *at >= now)
        .min();
    shell.set_exam_mode(at.map(|at| (at - now).num_days() <= 3).unwrap_or(false));
    let counts = services::revision::counts(&state.db.conn, &state.plan_id, today)?;
    shell.set_revision_badge((counts.due + counts.overdue) as i32);
    Ok(())
}

fn refresh_overview(ui: &AppWindow, state: &mut AppState) -> AppResult<()> {
    let bundle = plan_bundle(state)?;
    let view = services::dashboard::build(&state.db.conn, &state.plan_id, &state.settings)?;
    let data = ui.global::<crate::OverviewData>();
    data.set_greeting(view.greeting.clone().into());
    data.set_date_line(view.date_line.clone().into());
    data.set_plan_name(view.plan_name.clone().into());
    data.set_plan_span(view.plan_span.clone().into());
    data.set_next_exam_label(view.next_exam_label.clone().into());
    data.set_next_exam_countdown(view.next_exam_countdown.clone().into());
    data.set_next_exam_when(view.next_exam_when.clone().into());
    data.set_exam_mode(view.exam_mode);
    let today = time::today_in(&bundle.plan.timezone);
    data.set_agenda(model(to_slint_agenda(&view.agenda, today)));
    data.set_planned_min(view.planned_minutes as i32);
    data.set_focused_min(view.focused_minutes as i32);
    data.set_goal_min(view.goal_minutes as i32);
    data.set_goal_pct(view.goal_progress as f32);
    data.set_chapters_done(view.chapters_done as i32);
    data.set_chapters_total(view.chapters_total as i32);
    data.set_chapter_pct(view.chapter_progress as f32);
    data.set_revisions_due(view.revisions_due as i32);
    data.set_revisions_overdue(view.revisions_overdue as i32);
    data.set_overdue_tasks(view.overdue_tasks as i32);
    data.set_streak(view.streak as i32);
    data.set_tasks_done(view.tasks_done_today as i32);
    data.set_tasks_total(view.tasks_total_today as i32);
    data.set_next_block(view.next_block.clone().into());
    data.set_next_block_time(view.next_block_time.clone().into());
    data.set_rec_title(view.recommendation_title.clone().into());
    data.set_rec_subject(view.recommendation_subject.clone().into());
    data.set_rec_reason(view.recommendation_reason.clone().into());
    data.set_rec_minutes(view.recommendation_minutes as i32);
    data.set_rec_task_id(view.recommendation_task_id.clone().into());
    data.set_rec_chapter_id(view.recommendation_chapter_id.clone().into());
    data.set_rec_kind(view.recommendation_kind.clone().into());
    data.set_revision_rows(model(
        view.revisions
            .iter()
            .map(|r| crate::RevisionItem {
                id: r.id.clone().into(),
                title: r.title.clone().into(),
                subject: r.subject.clone().into(),
                due: r.due_text.clone().into(),
                overdue: r.overdue,
                chapter_id: "".into(),
                interval_text: "".into(),
                result: "".into(),
                state: "".into(),
                ease_text: "".into(),
            })
            .collect(),
    ));

    let sleep = repo::sleep_entries_for_plan(&state.db.conn, &state.plan_id, 12)?;
    let recorded = services::sleep::recorded_minutes_last_24h(&sleep, time::now_in(&bundle.plan.timezone));
    let target_min = (state.settings.sleep_target_hours * 60.0).max(1.0);
    data.set_sleep_hours(format!("{:.1}h", recorded as f64 / 60.0).into());
    data.set_sleep_pct(((recorded as f64 / target_min) * 100.0).clamp(0.0, 100.0) as f32);
    data.set_sleep_target(format!("{:.1}h", state.settings.sleep_target_hours).into());
    data.set_sleep_recorded(format!("{} logged", format!("{:.1}h", recorded as f64 / 60.0)).into());
    data.set_sleep_rows(model(
        sleep
            .iter()
            .take(4)
            .map(|s| {
                let start = services::sleep::parse_stamp(&s.start_at);
                let end = services::sleep::parse_stamp(&s.end_at);
                let range = match (&start, &end) {
                    (Some(a), Some(b)) => format!(
                        "{} – {}",
                        a.with_timezone(&bundle.plan.tz()).format("%H:%M"),
                        b.with_timezone(&bundle.plan.tz()).format("%H:%M")
                    ),
                    _ => "—".into(),
                };
                let mins = match (&start, &end) {
                    (Some(a), Some(b)) => (*b - *a).num_minutes().max(0),
                    _ => 0,
                };
                crate::SleepItem {
                    id: s.id.clone().into(),
                    kind: s.kind.label().into(),
                    range: range.into(),
                    energy: s.energy.map(|e| format!("energy {e}/5")).unwrap_or_default().into(),
                    duration: time::human_duration(mins * 60).into(),
                }
            })
            .collect(),
    ));
    Ok(())
}


fn task_status_index(status: TaskStatus) -> i32 {
    match status {
        TaskStatus::Planned => 0,
        TaskStatus::InProgress => 1,
        TaskStatus::Completed => 2,
        TaskStatus::PartiallyCompleted => 3,
        TaskStatus::Skipped => 4,
    }
}

fn chapter_status_index(status: ChapterStatus) -> i32 {
    status.index()
}

fn agenda_items_for_date(state: &AppState, date: NaiveDate) -> AppResult<(Vec<crate::AgendaItem>, i64, i64)> {
    let bundle = plan_bundle(state)?;
    let tasks = repo::tasks_for_date(&state.db.conn, &state.plan_id, date)?;
    let overrides = repo::day_overrides_for_plan(&state.db.conn, &state.plan_id)?;
    let mut pairs: Vec<(NaiveTime, crate::AgendaItem)> = Vec::new();
    for task in &tasks {
        let subject = task
            .subject_id
            .as_ref()
            .and_then(|id| bundle.subjects.iter().find(|(s, _)| &s.id == id.as_str()).map(|(s, _)| s));
        let color_hex = subject.map(|s| s.color.clone()).unwrap_or_else(|| "#6D6580".into());
        let chapter_title = task
            .chapter_id
            .as_ref()
            .and_then(|cid| repo::get_chapter(&state.db.conn, cid).ok())
            .map(|c| c.title)
            .unwrap_or_default();
        let seconds = task.estimated_minutes;
        pairs.push((
            task.start_time,
            crate::AgendaItem {
                id: task.id.clone().into(),
                title: task.title.clone().into(),
                subject: subject
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| chapter_title)
                    .into(),
                tint: color(&color_hex),
                start: time::fmt_time_12(task.start_time).into(),
                end: time::fmt_time_12(task.end_time).into(),
                status: task.status.label().into(),
                status_index: task_status_index(task.status),
                is_fixed: false,
                is_break: false,
                kind: task.task_type.label().into(),
                minutes: seconds as i32,
                chapter_id: task.chapter_id.clone().unwrap_or_default().into(),
                notes: task.notes.clone().into(),
                date: date.to_string().into(),
            },
        ));
    }
    for slot in crate::scheduling::fixed_periods_for_date(date, &bundle.templates, &overrides) {
        pairs.push((
            slot.start_time,
            crate::AgendaItem {
                id: slot.id.clone().into(),
                title: slot.title.clone().into(),
                subject: "Fixed".into(),
                tint: color("#4A4258"),
                start: time::fmt_time_12(slot.start_time).into(),
                end: time::fmt_time_12(slot.end_time).into(),
                status: slot.kind.label().into(),
                status_index: 5,
                is_fixed: true,
                is_break: true,
                kind: "Fixed".into(),
                minutes: time::minutes_between(slot.start_time, slot.end_time) as i32,
                chapter_id: "".into(),
                notes: "".into(),
                date: date.to_string().into(),
            },
        ));
    }
    pairs.sort_by_key(|(t, _)| *t);
    let planned: i64 = tasks
        .iter()
        .filter(|t| t.status != TaskStatus::Skipped)
        .map(|t| t.estimated_minutes)
        .sum();
    let done = tasks.iter().filter(|t| t.status == TaskStatus::Completed).count() as i64;
    Ok((pairs.into_iter().map(|(_, i)| i).collect(), planned, done))
}

fn refresh_timetable(ui: &AppWindow, state: &mut AppState) -> AppResult<()> {
    let bundle = plan_bundle(state)?;
    let data = ui.global::<crate::TimetableData>();
    let date = state.selected_date;
    let overrides = repo::day_overrides_for_plan(&state.db.conn, &state.plan_id)?;
    let override_row = overrides.iter().find(|o| o.date == date);
    let effective = crate::scheduling::effective_day_type(date, &overrides);
    data.set_selected_date(date.to_string().into());
    data.set_date_label(format!("{} {}", time::weekday_name(date), time::fmt_date(date)).into());
    data.set_weekday_label(format!("{} · {}", time::weekday_name(date), date.format("%d %b %Y")).into());
    data.set_day_type_label(effective.label().into());
    data.set_day_type_index(match override_row {
        None => 0,
        Some(o) => match o.day_type {
            DayType::Weekday => 1,
            DayType::Saturday => 2,
            DayType::Sunday => 3,
            DayType::Holiday => 4,
            DayType::Custom => 1,
        },
    });
    data.set_is_custom_day(override_row.is_some());
    let (rows, planned, _done) = agenda_items_for_date(state, date)?;
    let tasks = repo::tasks_for_date(&state.db.conn, &state.plan_id, date)?;
    let done = tasks.iter().filter(|t| t.status == TaskStatus::Completed).count();
    data.set_rows(model(rows));
    data.set_day_planned(planned as i32);
    let sessions = repo::sessions_for_date(&state.db.conn, &state.plan_id, date)?;
    data.set_day_focused(
        (sessions.iter().filter(|s| s.status == "completed").map(|s| s.focused_seconds).sum::<i64>() / 60) as i32,
    );
    data.set_day_summary(format!("{} of {} tasks complete", done, tasks.len()).into());

    // Week grid.
    let week_start = time::start_of_week(date);
    let mut week_days = Vec::new();
    let mut week_models: Vec<Vec<crate::AgendaItem>> = Vec::new();
    for i in 0..7 {
        let d = week_start + chrono::Duration::days(i);
        let day_tasks = repo::tasks_for_date(&state.db.conn, &state.plan_id, d)?;
        let items = agenda_items_for_date(state, d)?.0;
        week_days.push(crate::WeekDay {
            date: d.to_string().into(),
            label: time::fmt_date_short(d).into(),
            weekday: time::weekday_name(d).into(),
            minutes: day_tasks.iter().map(|t| t.estimated_minutes).sum::<i64>() as i32,
            task_count: day_tasks.len() as i32,
            done_count: day_tasks.iter().filter(|t| t.status == TaskStatus::Completed).count() as i32,
            is_today: d == time::today_in(&bundle.plan.timezone),
        });
        week_models.push(items.into_iter().filter(|i| !i.is_fixed).collect());
    }
    data.set_week_days(model(week_days));
    let mut it = week_models.into_iter();
    data.set_week_mon(model(it.next().unwrap_or_default()));
    data.set_week_tue(model(it.next().unwrap_or_default()));
    data.set_week_wed(model(it.next().unwrap_or_default()));
    data.set_week_thu(model(it.next().unwrap_or_default()));
    data.set_week_fri(model(it.next().unwrap_or_default()));
    data.set_week_sat(model(it.next().unwrap_or_default()));
    data.set_week_sun(model(it.next().unwrap_or_default()));

    let backlog = repo::backlog_tasks(&state.db.conn, &state.plan_id)?;
    data.set_backlog(model(
        backlog
            .iter()
            .map(|t| crate::AgendaItem {
                id: t.id.clone().into(),
                title: t.title.clone().into(),
                subject: t.task_type.label().into(),
                tint: color("#EABF73"),
                start: time::fmt_date_short(t.date).into(),
                end: "".into(),
                status: t.status.label().into(),
                status_index: task_status_index(t.status),
                is_fixed: false,
                is_break: false,
                kind: t.task_type.label().into(),
                minutes: t.estimated_minutes as i32,
                chapter_id: t.chapter_id.clone().unwrap_or_default().into(),
                notes: "".into(),
                date: t.date.to_string().into(),
            })
            .collect(),
    ));

    let templates = &bundle.templates;
    data.set_templates(model(
        templates
            .iter()
            .map(|t| crate::TemplateRow {
                id: t.id.clone().into(),
                name: t.name.clone().into(),
                day_types: t.day_types.iter().map(|d| d.label()).collect::<Vec<_>>().join(", ").into(),
                slot_count: t.slots.len() as i32,
            })
            .collect(),
    ));
    let tpl_index = state
        .editing_template
        .as_ref()
        .and_then(|id| templates.iter().position(|t| &t.id == id))
        .unwrap_or_else(|| data.get_template_index().max(0) as usize)
        .min(templates.len().saturating_sub(1));
    if !templates.is_empty() {
        data.set_template_index(tpl_index as i32);
        if state.editing_template.is_none() {
            state.editing_template = Some(templates[tpl_index].id.clone());
        }
        let slots = &templates[tpl_index].slots;
        data.set_template_slots(model(
            slots
                .iter()
                .map(|slot| {
                    let subject = slot
                        .subject_id
                        .as_ref()
                        .and_then(|id| bundle.subjects.iter().find(|(s, _)| &s.id == id.as_str()).map(|(s, _)| s));
                    crate::SlotRow {
                        id: slot.id.clone().into(),
                        template_id: slot.template_id.clone().into(),
                        title: slot.title.clone().into(),
                        kind: slot.kind.label().into(),
                        kind_index: match slot.kind {
                            SlotKind::Study => 0,
                            SlotKind::Break => 1,
                            SlotKind::Meal => 2,
                        },
                        start: time::fmt_time(slot.start_time).into(),
                        end: time::fmt_time(slot.end_time).into(),
                        subject: subject.map(|s| s.name.clone()).unwrap_or_default().into(),
                        tint: color(subject.map(|s| s.color.as_str()).unwrap_or("#4A4258")),
                    }
                })
                .collect(),
        ));
    }
    Ok(())
}

fn refresh_editor_options(ui: &AppWindow, state: &mut AppState) -> AppResult<()> {
    let bundle = plan_bundle(state)?;
    let data = ui.global::<crate::TimetableData>();
    state.editor_subjects = bundle.subjects.iter().map(|(s, _)| s.clone()).collect();
    data.set_subject_names(model(
        state.editor_subjects.iter().map(|s| s.name.clone().into()).collect(),
    ));
    let idx = data.get_editor_subject().max(0) as usize;
    state.editor_chapters = state
        .editor_subjects
        .get(idx)
        .map(|s| repo::chapters_for_subject(&state.db.conn, &s.id))
        .transpose()?
        .unwrap_or_default();
    data.set_chapter_names(model(
        state.editor_chapters.iter().map(|c| c.title.clone().into()).collect(),
    ));
    Ok(())
}

fn refresh_syllabus(ui: &AppWindow, state: &mut AppState) -> AppResult<()> {
    let bundle = plan_bundle(state)?;
    let data = ui.global::<crate::SyllabusData>();
    let subjects: Vec<(Subject, Vec<Chapter>)> = bundle.subjects.clone();
    let mut all = 0i64;
    let mut ready = 0i64;
    for (_, chapters) in &subjects {
        all += chapters.len() as i64;
        ready += chapters.iter().filter(|c| c.status.is_complete()).count() as i64;
    }
    data.set_all_count(all as i32);
    data.set_ready_count(ready as i32);
    data.set_subjects(model(
        subjects
            .iter()
            .map(|(s, chapters)| crate::SubjectRow {
                id: s.id.clone().into(),
                name: s.name.clone().into(),
                tint: color(&s.color),
                chapter_count: chapters.len() as i32,
                ready_count: chapters.iter().filter(|c| c.status.is_complete()).count() as i32,
                progress: if chapters.is_empty() {
                    0
                } else {
                    (chapters.iter().filter(|c| c.status.is_complete()).count() as f64 / chapters.len() as f64 * 100.0) as i32
                },
            })
            .collect(),
    ));
    let idx = data.get_subject_index().max(0) as usize;
    let (subject, chapters) = match subjects.get(idx) {
        Some(pair) => pair,
        None => {
            data.set_chapters(model(Vec::new()));
            data.set_subject_id("".into());
            return Ok(());
        }
    };
    data.set_subject_id(subject.id.clone().into());
    state.selected_subject = Some(subject.id.clone());
    let search = data.get_search().to_string().to_lowercase();
    let filter = data.get_filter();
    let visible: Vec<&Chapter> = chapters
        .iter()
        .filter(|c| search.is_empty() || c.title.to_lowercase().contains(&search))
        .filter(|c| match filter {
            0 => true,
            1 => c.status == ChapterStatus::NotStarted,
            2 => c.status == ChapterStatus::Learning,
            3 => c.status == ChapterStatus::NeedsPractice,
            4 => c.status == ChapterStatus::NeedsRevision,
            5 => c.status == ChapterStatus::ExamReady,
            _ => true,
        })
        .collect();
    let today = time::today_in(&bundle.plan.timezone);
    data.set_chapters(model(
        visible
            .iter()
            .map(|c| {
                let chapter_subject = subjects
                    .iter()
                    .find(|(s, _)| s.id == c.subject_id)
                    .map(|(s, _)| s)
                    .unwrap_or(subject);
                crate::ChapterRow {
                    id: c.id.clone().into(),
                    subject_id: c.subject_id.clone().into(),
                    title: c.title.clone().into(),
                    status: c.status.label().into(),
                    status_index: chapter_status_index(c.status),
                    checklist_done: c.checklist_done() as i32,
                    concept: c.concept_done,
                    textbook: c.textbook_done,
                    exercises: c.exercises_done,
                    selftest: c.selftest_done,
                    confidence: c.confidence.unwrap_or(0) as i32,
                    questions: c.questions_attempted as i32,
                    correct: c.correct_answers as i32,
                    accuracy_text: if c.questions_attempted > 0 {
                        format!("{:.0}%", c.correct_answers as f64 / c.questions_attempted as f64 * 100.0).into()
                    } else {
                        "—".into()
                    },
                    study_text: time::human_duration(c.study_seconds).into(),
                    last_studied: c.last_studied.map(time::fmt_date_short).unwrap_or_else(|| "—".into()).into(),
                    next_revision: c.next_revision.map(time::fmt_date_short).unwrap_or_else(|| "—".into()).into(),
                    tint: color(&chapter_subject.color),
                    subject: chapter_subject.name.clone().into(),
                    notes: c.notes.clone().into(),
                }
            })
            .collect(),
    ));
    let _ = today;
    Ok(())
}

fn refresh_focus(ui: &AppWindow, state: &mut AppState) -> AppResult<()> {
    let bundle = plan_bundle(state)?;
    let data = ui.global::<crate::FocusData>();
    state.focus_subjects = bundle.subjects.iter().map(|(s, _)| s.clone()).collect();
    data.set_subject_names(model(
        state.focus_subjects.iter().map(|s| s.name.clone().into()).collect(),
    ));
    let sidx = data.get_subject_index().max(0) as usize;
    state.focus_chapters = state
        .focus_subjects
        .get(sidx)
        .map(|s| repo::chapters_for_subject(&state.db.conn, &s.id))
        .transpose()?
        .unwrap_or_default();
    data.set_chapter_names(model(
        state.focus_chapters.iter().map(|c| c.title.clone().into()).collect(),
    ));
    let t = &state.timer;
    data.set_display(t.display().into());
    data.set_running(t.state == crate::timer::TimerState::Running);
    data.set_paused(t.state == crate::timer::TimerState::Paused);
    data.set_active(t.state != crate::timer::TimerState::Idle);
    data.set_is_break(t.phase == crate::timer::TimerPhase::Break);
    data.set_phase_label(match t.phase {
        crate::timer::TimerPhase::Focus => "Deep focus",
        crate::timer::TimerPhase::Break => "Break",
    }.into());
    data.set_state_label(match t.state {
        crate::timer::TimerState::Idle => "Ready when you are",
        crate::timer::TimerState::Running => "Running",
        crate::timer::TimerState::Paused => "Paused",
    }.into());
    let planned = if t.phase == crate::timer::TimerPhase::Focus { t.planned_focus } else { t.planned_break };
    data.set_progress(if planned.is_zero() { 0.0 } else { (t.elapsed_phase().as_secs_f64() / planned.as_secs_f64() * 100.0).clamp(0.0, 100.0) as f32 });
    data.set_focused_today(time::human_duration(t.focus_elapsed().as_secs() as i64).into());
    data.set_break_today(time::human_duration(t.break_elapsed().as_secs() as i64).into());
    let linked = t
        .task_id
        .as_ref()
        .and_then(|id| repo::get_task(&state.db.conn, id).ok())
        .map(|task| task.title)
        .unwrap_or_default();
    data.set_linked_task(linked.into());
    data.set_linked_chapter(
        t.chapter_id
            .as_ref()
            .and_then(|id| repo::get_chapter(&state.db.conn, id).ok())
            .map(|c| c.title)
            .unwrap_or_default()
            .into(),
    );
    data.set_preset(match t.preset {
        Preset::Deep => 0,
        Preset::Short => 1,
        Preset::Extended => 2,
        Preset::Custom => 3,
        Preset::Stopwatch => 4,
    });
    if let Some(id) = &t.subject_id {
        if let Some(idx) = state.focus_subjects.iter().position(|s| &s.id == id.as_str()) {
            data.set_subject_index(idx as i32);
        }
    }
    if let Some(id) = &t.chapter_id {
        if let Some(idx) = state.focus_chapters.iter().position(|c| &c.id == id) {
            data.set_chapter_index(idx as i32);
        }
    }
    let sessions = repo::recent_sessions(&state.db.conn, &state.plan_id, 8)?;
    data.set_sessions(model(
        sessions
            .iter()
            .map(|s| {
                let subject = s
                    .subject_id
                    .as_ref()
                    .and_then(|id| repo::get_subject(&state.db.conn, id).ok())
                    .map(|s| s.name)
                    .unwrap_or_else(|| "Unassigned".into());
                let chapter = s
                    .chapter_id
                    .as_ref()
                    .and_then(|id| repo::get_chapter(&state.db.conn, id).ok())
                    .map(|c| c.title)
                    .unwrap_or_default();
                crate::SessionRow {
                    id: s.id.clone().into(),
                    subject: subject.into(),
                    chapter: chapter.into(),
                    mode: s.mode.clone().into(),
                    start: chrono::DateTime::parse_from_rfc3339(&s.started_at)
                        .map(|d| d.with_timezone(&bundle.plan.tz()).format("%d %b · %H:%M").to_string())
                        .unwrap_or_default()
                        .into(),
                    focused: time::human_duration(s.focused_seconds).into(),
                    status: s.status.clone().into(),
                }
            })
            .collect(),
    ));
    data.set_session_count(sessions.len() as i32);
    let today = time::today_in(&bundle.plan.timezone);
    let todays = repo::sessions_for_date(&state.db.conn, &state.plan_id, today)?;
    let total: i64 = todays.iter().filter(|s| s.status == "completed").map(|s| s.focused_seconds).sum();
    data.set_today_focus_total(time::human_duration(total).into());
    data.set_focused_today(time::human_duration(total).into());
    Ok(())
}

fn refresh_revision(ui: &AppWindow, state: &mut AppState) -> AppResult<()> {
    let bundle = plan_bundle(state)?;
    let data = ui.global::<crate::RevisionData>();
    let today = time::today_in(&bundle.plan.timezone);
    let all = repo::revisions_for_plan(&state.db.conn, &state.plan_id)?;
    let counts = services::revision::counts(&state.db.conn, &state.plan_id, today)?;
    data.set_due_count(counts.due as i32);
    data.set_overdue_count(counts.overdue as i32);
    data.set_upcoming_count(counts.upcoming as i32);
    data.set_completed_count(counts.completed as i32);
    let tab = data.get_tab();
    let mut rows: Vec<&Revision> = all
        .iter()
        .filter(|r| match tab {
            0 => !matches!(r.state, RevisionState::Completed) && r.snoozed_until.unwrap_or(r.due_date) == today,
            1 => !matches!(r.state, RevisionState::Completed) && r.snoozed_until.unwrap_or(r.due_date) < today,
            2 => !matches!(r.state, RevisionState::Completed) && r.snoozed_until.unwrap_or(r.due_date) > today,
            _ => matches!(r.state, RevisionState::Completed),
        })
        .collect();
    rows.sort_by_key(|r| r.due_date);
    if tab == 3 {
        rows.reverse();
    }
    let mut out = Vec::new();
    for rev in rows {
        let (title, subject, tint) = match repo::get_chapter(&state.db.conn, &rev.chapter_id) {
            Ok(chapter) => {
                let subject = repo::get_subject(&state.db.conn, &chapter.subject_id).ok();
                (
                    chapter.title,
                    subject.as_ref().map(|s| s.name.clone()).unwrap_or_default(),
                    color(subject.as_ref().map(|s| s.color.as_str()).unwrap_or("#9B7BFF")),
                )
            }
            Err(_) => ("Unknown chapter".into(), String::new(), color("#9B7BFF")),
        };
        let _ = tint;
        out.push(crate::RevisionItem {
            id: rev.id.clone().into(),
            title: title.into(),
            subject: subject.into(),
            due: if rev.due_date < today && !matches!(rev.state, RevisionState::Completed) {
                format!("{}d overdue", (today - rev.due_date).num_days()).into()
            } else {
                time::fmt_date_short(rev.due_date).into()
            },
            overdue: rev.due_date < today && !matches!(rev.state, RevisionState::Completed),
            chapter_id: rev.chapter_id.clone().into(),
            interval_text: format!("{}d interval", rev.interval_days).into(),
            result: rev.result.map(|r| r.label().to_string()).unwrap_or_else(|| "—".into()).into(),
            state: rev.state.as_str().into(),
            ease_text: format!("ease {:.1}", rev.ease).into(),
        });
    }
    data.set_rows(model(out));
    Ok(())
}

fn refresh_exams(ui: &AppWindow, state: &mut AppState) -> AppResult<()> {
    let bundle = plan_bundle(state)?;
    let data = ui.global::<crate::ExamsData>();
    let today = time::today_in(&bundle.plan.timezone);
    let now = time::now_in(&bundle.plan.timezone);
    let exams = services::dashboard::exam_rows(&state.db.conn, &state.plan_id)?;
    data.set_exams(model(
        exams
            .iter()
            .map(|e| crate::ExamCard {
                id: e.id.clone().into(),
                label: e.label.clone().into(),
                subject: e.subject.clone().into(),
                subject_id: bundle
                    .exams
                    .iter().find(|x| x.id == e.id).map(|x| x.subject_id.clone()).unwrap_or_default()
                    .into(),
                date_text: e.date_text.clone().into(),
                time_text: e.time_text.clone().into(),
                countdown: e.countdown.clone().into(),
                days_left: e.days_left as i32,
                tint: color(&e.color),
                session: e.session.clone().into(),
                progress: e.progress as i32,
                chapters_ready: e.chapters_ready as i32,
                chapters_total: e.chapters_total as i32,
                is_past: e.days_left < 0,
            })
            .collect(),
    ));
    let mut next: Option<&Exam> = None;
    let mut next_at = None;
    for exam in &bundle.exams {
        let at = time::at_local(&bundle.plan.timezone, exam.date, exam.start_time);
        if at >= now && next_at.map(|n| at < n).unwrap_or(true) {
            next = Some(exam);
            next_at = Some(at);
        }
    }
    data.set_next_paper(next.map(|e| e.label.clone()).unwrap_or_default().into());
    data.set_next_paper_subject_id(next.map(|e| e.subject_id.clone()).unwrap_or_default().into());
    data.set_next_paper_when(next.map(|e| format!(
        "{} · {} · {}",
        time::fmt_date(e.date),
        time::fmt_time_12(e.start_time),
        e.session
    )).unwrap_or_default().into());
    data.set_subject_names(model(bundle.subjects.iter().map(|(s, _)| s.name.clone().into()).collect()));
    state.mistake_subjects = bundle.subjects.iter().map(|(s, _)| s.clone()).collect();
    data.set_mock_subject(
        data.get_mock_subject().clamp(0, (bundle.subjects.len() as i32 - 1).max(0)),
    );
    data.set_mock_date(today.to_string().into());
    let mocks = repo::mock_tests_for_plan(&state.db.conn, &state.plan_id)?;
    data.set_mock_count(mocks.len() as i32);
    data.set_mock_avg(
        if mocks.is_empty() {
            "—".into()
        } else {
            format!("{:.0}%", mocks.iter().map(|m| m.percentage()).sum::<f64>() / mocks.len() as f64).into()
        },
    );
    data.set_mocks(model(
        mocks
            .iter()
            .map(|m| {
                let subject = bundle.subjects.iter().find(|(s, _)| s.id == m.subject_id).map(|(s, _)| s.name.clone()).unwrap_or_default();
                crate::MockRowData {
                    id: m.id.clone().into(),
                    name: m.name.clone().into(),
                    subject: subject.into(),
                    date: time::fmt_date_short(m.date).into(),
                    score: format!("{:.0}/{:.0}", m.score, m.max_marks).into(),
                    percent: m.percentage().round() as i32,
                    accuracy: format!("{:.0}%", m.percentage()).into(),
                    weak: m.weak_topics.clone().into(),
                    duration: format!("{}m", m.duration_minutes).into(),
                }
            })
            .collect(),
    ));
    let mistakes = repo::mistakes_for_plan(&state.db.conn, &state.plan_id)?;
    data.set_open_mistakes(mistakes.iter().filter(|m| !m.resolved).count() as i32);
    data.set_mistakes(model(
        mistakes
            .iter()
            .map(|m| {
                let subject = bundle.subjects.iter().find(|(s, _)| s.id == m.subject_id).map(|(s, _)| s.name.clone()).unwrap_or_default();
                let chapter = m.chapter_id.as_ref().and_then(|id| repo::get_chapter(&state.db.conn, id).ok()).map(|c| c.title).unwrap_or_default();
                crate::MistakeRowData {
                    id: m.id.clone().into(),
                    subject: subject.into(),
                    chapter: chapter.into(),
                    question: m.question.clone().into(),
                    explanation: m.explanation.clone().into(),
                    resolved: m.resolved,
                }
            })
            .collect(),
    ));
    data.set_exam_mode(state.exam_mode_override || services::dashboard::build(&state.db.conn, &state.plan_id, &state.settings)?.exam_mode);
    Ok(())
}

fn refresh_analytics(ui: &AppWindow, state: &mut AppState) -> AppResult<()> {
    let data = ui.global::<crate::AnalyticsData>();
    let days = match data.get_range() { 0 => 7, 1 => 30, _ => 90 };
    let view = services::analytics::build(&state.db.conn, &state.plan_id, days)?;
    data.set_focused_text(time::human_duration(view.focused_minutes * 60).into());
    data.set_planned_text(time::human_duration(view.planned_minutes * 60).into());
    data.set_tasks_done(view.tasks_done as i32);
    data.set_tasks_total(view.tasks_total as i32);
    data.set_completion_pct(view.completion_pct.round() as i32);
    data.set_avg_session(view.avg_session_minutes as i32);
    data.set_session_count(view.session_count as i32);
    data.set_chapters_ready(view.chapters_ready as i32);
    data.set_chapters_total(view.chapters_total as i32);
    data.set_backlog_count(view.backlog_count as i32);
    data.set_recall_pct(view.recall_pct.round() as i32);
    data.set_recall_good(view.recall_good as i32);
    data.set_recall_total(view.recall_total as i32);
    data.set_accuracy(view.overall_accuracy.clone().into());
    data.set_sleep_avg(view.sleep_avg_hours as f32);
    data.set_daily(model(
        view.daily.iter().map(|d| crate::BarPoint {
            label: d.date_text.clone().into(),
            weekday: d.weekday.clone().into(),
            focused: d.focused_minutes as i32,
            planned: d.planned_minutes as i32,
            height: d.height_pct as f32,
            planned_height: d.planned_pct as f32,
            is_today: d.is_today,
        }).collect(),
    ));
    data.set_subjects(model(
        view.subjects.iter().map(|s| crate::SubjectSliceData {
            name: s.name.clone().into(),
            tint: color(&s.color),
            minutes: s.minutes as i32,
            percent: s.percent.round() as i32,
            text: s.text.clone().into(),
        }).collect(),
    ));
    data.set_statuses(model(
        view.statuses.iter().map(|s| crate::StatusSlice {
            label: s.label.clone().into(),
            count: s.count as i32,
            percent: s.percent.round() as i32,
            tone: s.tone.clone().into(),
        }).collect(),
    ));
    data.set_heatmap(model(
        view.heatmap.iter().map(|h| crate::HeatCell {
            level: h.level,
            tooltip: h.tooltip.clone().into(),
            blank: h.blank,
        }).collect(),
    ));
    data.set_mocks(model(
        view.mocks.iter().map(|m| crate::MockRowData {
            id: m.id.clone().into(),
            name: m.name.clone().into(),
            subject: m.subject.clone().into(),
            date: m.date_text.clone().into(),
            score: m.score_text.clone().into(),
            percent: m.percent.round() as i32,
            accuracy: m.accuracy_text.clone().into(),
            weak: m.weak_topics.clone().into(),
            duration: m.duration_text.clone().into(),
        }).collect(),
    ));
    Ok(())
}


fn refresh_io(ui: &AppWindow, state: &mut AppState) -> AppResult<()> {
    let data = ui.global::<crate::IoData>();
    if data.get_editor_text().is_empty() && state.import_preview.is_none() {
        if let Ok(file) = crate::import_export::format::export_plan(&state.db.conn, &state.plan_id) {
            if let Ok(text) = crate::import_export::format::to_pretty_json(&file) {
                data.set_editor_text(text.into());
            }
        }
    }
    if data.get_builder_name().is_empty() {
        if let Ok(file) = crate::import_export::format::export_plan(&state.db.conn, &state.plan_id) {
            data.set_builder_name(file.plan.name.clone().into());
            data.set_builder_id(file.plan.id.clone().into());
            data.set_builder_year(file.plan.academic_year.clone().into());
            data.set_builder_start(file.plan.start_date.clone().into());
            data.set_builder_end(file.plan.end_date.clone().into());
            data.set_builder_timezone(file.plan.timezone.clone().into());
            data.set_builder_subjects(model(
                file.subjects.iter().map(|s| crate::BuilderSubjectRow {
                    index: 0,
                    name: s.name.clone().into(),
                    tint: color(&s.color),
                    chapters: s.chapters.iter().map(|c| c.title.clone()).collect::<Vec<_>>().join(", ").into(),
                }).collect(),
            ));
            data.set_builder_exams(model(
                file.exams.iter().map(|e| crate::BuilderExamRow {
                    index: 0,
                    label: e.label.clone().into(),
                    subject: file.subjects.iter().find(|s| s.id == e.subject_id).map(|s| s.name.clone()).unwrap_or_default().into(),
                    date: e.date.clone().into(),
                    time: e.start_time.clone().into(),
                }).collect(),
            ));
            data.set_builder_subject_names(model(
                file.subjects.iter().map(|s| s.name.clone().into()).collect(),
            ));
        }
    }
    if let Some(prepared) = &state.import_preview {
        let p = &prepared.preview;
        let warnings: Vec<String> = prepared.issues.iter().filter(|i| !i.is_error()).map(|i| i.display()).collect();
        let errors: Vec<String> = prepared.issues.iter().filter(|i| i.is_error()).map(|i| i.display()).collect();
        data.set_preview(crate::PreviewData {
            plan_name: p.plan_name.clone().into(),
            plan_id: p.plan_id.clone().into(),
            exists: p.plan_exists,
            exists_text: if p.plan_exists {
                format!("Updates the existing plan '{}'", p.existing_plan_name)
            } else {
                "Creates a new local plan".into()
            }.into(),
            span: format!("{} · {}", p.date_span, p.timezone).into(),
            summary: p.summary.clone().into(),
            subjects: p.subjects_total.to_string().into(),
            chapters: p.chapters_total.to_string().into(),
            exams: p.exams_total.to_string().into(),
            templates: p.templates_total.to_string().into(),
            tasks: p.tasks_total.to_string().into(),
            warnings: warnings.join("\n").into(),
            errors: errors.join("\n").into(),
            can_import: !prepared.has_errors(),
        });
        data.set_preview_open(true);
    }
    Ok(())
}

fn refresh_settings(ui: &AppWindow, state: &mut AppState) -> AppResult<()> {
    let data = ui.global::<crate::SettingsData>();
    data.set_display_name(state.settings.display_name.clone().into());
    data.set_timezone(state.settings.timezone.clone().into());
    data.set_goal_minutes(state.settings.daily_goal_minutes.to_string().into());
    data.set_sleep_target(format!("{:.1}", state.settings.sleep_target_hours).into());
    data.set_sound(state.settings.sound_enabled);
    data.set_notifications(state.settings.notifications_enabled);
    data.set_auto_revisions(state.settings.auto_generate_revisions);
    data.set_import_mode(if state.settings.default_import_mode == "replace" { 1 } else { 0 });
    data.set_intervals_text(
        state.settings.revision_intervals.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ").into(),
    );
    data.set_db_path(state.db.path.display().to_string().into());
    data.set_version(env!("CARGO_PKG_VERSION").into());
    let accents = ["#9B7BFF", "#69D6A6", "#EABF73", "#7FB2E6", "#E67F90", "#C6A6FF"];
    data.set_accent_index(
        accents.iter().position(|c| c.eq_ignore_ascii_case(&state.settings.accent)).unwrap_or(0) as i32,
    );
    data.set_theme_index(if state.settings.theme == "slate" { 1 } else { 0 });
    let plans = repo::list_plans(&state.db.conn, true)?;
    data.set_plans(model(
        plans.iter().map(|p| crate::PlanRowData {
            id: p.id.clone().into(),
            name: p.name.clone().into(),
            span: format!("{} – {}", time::fmt_date_short(p.start_date), time::fmt_date_short(p.end_date)).into(),
            archived: p.archived,
            current: p.id == state.plan_id,
        }).collect(),
    ));
    Ok(())
}

fn wizard_chapters(state: &AppState, index: i32) -> Vec<crate::WizardChapterRow> {
    state.wizard.subjects.get(index.max(0) as usize)
        .map(|s| s.chapters.iter().enumerate().map(|(i, c)| crate::WizardChapterRow {
            index: i as i32,
            title: c.title.clone().into(),
        }).collect())
        .unwrap_or_default()
}

fn publish_wizard(ui: &AppWindow, state: &AppState) {
    let data = ui.global::<crate::WizardData>();
    data.set_name(state.wizard.name.clone().into());
    data.set_year(state.wizard.academic_year.clone().into());
    data.set_start(state.wizard.start_date.clone().into());
    data.set_end(state.wizard.end_date.clone().into());
    data.set_timezone(state.wizard.timezone.clone().into());
    data.set_subjects(model(
        state.wizard.subjects.iter().enumerate().map(|(i, s)| crate::WizardSubjectRow {
            index: i as i32,
            name: s.name.clone().into(),
            tint: color(&s.color),
            chapter_count: s.chapters.len() as i32,
        }).collect(),
    ));
    data.set_subject_names(model(
        state.wizard.subjects.iter().map(|s| s.name.clone().into()).collect(),
    ));
    data.set_chapters(model(wizard_chapters(state, data.get_subject_index())));
    data.set_selected_subject_name(
        state.wizard.subjects.get(data.get_subject_index().max(0) as usize)
            .map(|s| s.name.clone()).unwrap_or_default().into(),
    );
    data.set_exams(model(
        state.wizard.exams.iter().enumerate().map(|(i, e)| crate::WizardExamRow {
            index: i as i32,
            label: e.label.clone().into(),
            subject: state.wizard.subjects.iter().find(|s| s.id == e.subject_id)
                .map(|s| s.name.clone()).unwrap_or_default().into(),
            date: e.date.clone().into(),
            time: e.start_time.clone().into(),
        }).collect(),
    ));
    data.set_template_names(model(
        state.wizard.templates.iter().map(|t| t.name.clone().into()).collect(),
    ));
    data.set_template_index(data.get_template_index().clamp(0, (state.wizard.templates.len() as i32 - 1).max(0)));
    data.set_slots(model(
        state.wizard.templates.get(data.get_template_index().max(0) as usize)
            .map(|t| t.slots.iter().enumerate().map(|(i, s)| crate::WizardSlotRow {
                index: i as i32,
                title: s.title.clone().into(),
                time: format!("{} – {}", s.start_time, s.end_time).into(),
                kind: s.kind.label().into(),
                subject: state.wizard.subjects.iter().find(|sub| sub.id == s.subject_id)
                    .map(|sub| sub.name.clone()).unwrap_or_default().into(),
            }).collect())
            .unwrap_or_default(),
    ));
    data.set_generate_tasks(state.wizard.generate_tasks);
    data.set_errors(slint::ModelRc::new(slint::VecModel::from(
        state.wizard.validate().iter().map(|e| e.clone().into()).collect::<Vec<slint::SharedString>>()
    )));
    data.set_summary(format!(
        "{} · {} subjects · {} chapters · {} exams · {} templates",
        state.wizard.name,
        state.wizard.subjects.len(),
        state.wizard.subjects.iter().map(|s| s.chapters.len()).sum::<usize>(),
        state.wizard.exams.len(),
        state.wizard.templates.len()
    ).into());
}

fn refresh_page(ui: &AppWindow, state: &mut AppState, page: i32) -> AppResult<()> {
    match page {
        0 => refresh_overview(ui, state),
        1 => { refresh_timetable(ui, state)?; refresh_editor_options(ui, state) }
        2 => refresh_syllabus(ui, state),
        3 => refresh_focus(ui, state),
        4 => refresh_revision(ui, state),
        5 => refresh_exams(ui, state),
        6 => refresh_analytics(ui, state),
        7 => refresh_io(ui, state),
        8 => refresh_settings(ui, state),
        _ => Ok(()),
    }
}

fn refresh_all(ui: &AppWindow, state: &mut AppState) {
    if let Err(err) = refresh_shell(ui, state) {
        show_toast(ui, state, err.to_string(), 2);
        return;
    }
    let page = ui.global::<crate::Shell>().get_current_page();
    if let Err(err) = refresh_page(ui, state, page) {
        show_toast(ui, state, err.to_string(), 2);
    }
}

fn navigate(ui: &AppWindow, state: &mut AppState, page: i32) {
    let page = page.clamp(0, 8);
    let shell = ui.global::<crate::Shell>();
    shell.set_current_page(page);
    shell.set_page_title(PAGE_TITLES[page as usize].into());
    let name = shell.get_plan_name().to_string();
    let tz = repo::get_plan(&state.db.conn, &state.plan_id)
        .map(|p| p.timezone)
        .unwrap_or_else(|_| DEFAULT_TZ.to_string());
    shell.set_page_context(page_context(page, &name, &tz).into());
    if let Err(err) = refresh_page(ui, state, page) {
        show_toast(ui, state, err.to_string(), 2);
    }
}

fn start_for_task(ui: &AppWindow, state: &mut AppState, task_id: &str) -> AppResult<()> {
    let task = repo::get_task(&state.db.conn, task_id)?;
    let bundle = plan_bundle(state)?;
    let subject = task
        .subject_id
        .clone()
        .or_else(|| {
            task.chapter_id
                .as_ref()
                .and_then(|cid| repo::get_chapter(&state.db.conn, cid).ok())
                .map(|c| c.subject_id)
        });
    let mut timer = FocusTimer::new(
        Preset::Deep,
        "deep",
        Duration::from_secs(45 * 60),
        Duration::from_secs(10 * 60),
        subject.clone(),
        task.chapter_id.clone(),
        Some(task.id.clone()),
        bundle.plan.tz(),
    );
    timer.task_id = Some(task.id.clone());
    services::focus::mark_task_started(&state.db.conn, task_id)?;
    state.focus_subject = subject;
    state.focus_chapter = task.chapter_id.clone();
    state.recovered = false;
    state.timer = timer;
    persist_timer(state)?;
    navigate(ui, state, 3);
    show_toast(ui, state, format!("Focus started · {}", task.title), 1);
    Ok(())
}

fn start_for_chapter(ui: &AppWindow, state: &mut AppState, chapter_id: &str) -> AppResult<()> {
    let chapter = repo::get_chapter(&state.db.conn, chapter_id)?;
    let bundle = plan_bundle(state)?;
    let timer = FocusTimer::new(
        Preset::Deep,
        "deep",
        Duration::from_secs(45 * 60),
        Duration::from_secs(10 * 60),
        Some(chapter.subject_id.clone()),
        Some(chapter.id.clone()),
        None,
        bundle.plan.tz(),
    );
    state.focus_subject = Some(chapter.subject_id.clone());
    state.focus_chapter = Some(chapter.id.clone());
    state.recovered = false;
    state.timer = timer;
    persist_timer(state)?;
    navigate(ui, state, 3);
    show_toast(ui, state, format!("Revision focus · {}", chapter.title), 1);
    Ok(())
}

fn persist_timer(state: &AppState) -> AppResult<()> {
    let t = &state.timer;
    let row = repo::TimerRow {
        session_id: t.session_id.clone(),
        plan_id: state.plan_id.clone(),
        task_id: t.task_id.clone(),
        subject_id: t.subject_id.clone(),
        chapter_id: t.chapter_id.clone(),
        mode: t.mode.clone(),
        phase: t.phase.as_str().to_string(),
        state: t.state.as_str().to_string(),
        started_at: t.started_at.to_rfc3339(),
        accumulated_ms: t.focus_ms(),
        break_accumulated_ms: t.break_ms(),
        planned_focus_secs: t.planned_focus.as_secs() as i64,
        planned_break_secs: t.planned_break.as_secs() as i64,
        interruptions: t.interruptions,
        last_checkpoint: Utc::now().to_rfc3339(),
    };
    repo::save_timer(&state.db.conn, &row)
}

fn set_task_status(ui: &AppWindow, state: &mut AppState, id: &str, status: TaskStatus) -> AppResult<()> {
    let mut task = repo::get_task(&state.db.conn, id)?;
    task.status = status;
    task.completed_at = if matches!(status, TaskStatus::Completed) {
        Some(Utc::now().to_rfc3339())
    } else {
        None
    };
    repo::update_task(&state.db.conn, &task)?;
    if matches!(status, TaskStatus::Completed) && state.settings.auto_generate_revisions {
        if let Some(chapter_id) = task.chapter_id.clone() {
            let today = time::today_in(&plan_bundle(state)?.plan.timezone);
            services::revision::generate_for_chapter(
                &mut state.db.conn,
                &state.plan_id,
                &chapter_id,
                &state.settings.revision_intervals,
                today,
            )?;
        }
    }
    refresh_all(ui, state);
    Ok(())
}

fn save_sleep(
    ui: &AppWindow,
    state: &mut AppState,
    kind_index: i32,
    start_text: String,
    end_text: String,
    energy: i32,
    note: String,
) -> AppResult<()> {
    let bundle = plan_bundle(state)?;
    let tz = bundle.plan.tz();
    let today = time::today_in(&bundle.plan.timezone);
    let start_time = time::parse_time(&start_text).map_err(AppError::Validation)?;
    let end_time = time::parse_time(&end_text).map_err(AppError::Validation)?;
    if start_time == end_time {
        return Err(AppError::Validation("Start and end time are identical".into()));
    }
    let start_date = if end_time <= start_time { today - chrono::Duration::days(1) } else { today };
    let start_naive = start_date.and_time(start_time);
    let end_naive = today.and_time(end_time);
    let start_local = tz
        .from_local_datetime(&start_naive)
        .single()
        .ok_or_else(|| AppError::Validation("Invalid local start time".into()))?;
    let end_local = tz
        .from_local_datetime(&end_naive)
        .single()
        .ok_or_else(|| AppError::Validation("Invalid local end time".into()))?;
    let kind = match kind_index {
        1 => SleepKind::Nap,
        2 => SleepKind::SchoolNap,
        _ => SleepKind::Night,
    };
    let entry = SleepEntry {
        id: uuid::Uuid::new_v4().to_string(),
        plan_id: state.plan_id.clone(),
        kind,
        start_at: start_local.to_rfc3339(),
        end_at: end_local.to_rfc3339(),
        energy: Some(energy.clamp(1, 5) as i64),
        notes: note,
        created_at: Utc::now().to_rfc3339(),
    };
    repo::insert_sleep(&state.db.conn, &entry)?;
    ui.global::<crate::OverviewData>().set_sleep_dialog(false);
    refresh_shell(ui, state)?;
    refresh_overview(ui, state)?;
    show_toast(ui, state, "Sleep logged", 1);
    Ok(())
}

fn select_plan(ui: &AppWindow, state: &mut AppState, index: i32) -> AppResult<()> {
    let plans = repo::list_plans(&state.db.conn, false)?;
    let plan = plans
        .get(index.max(0) as usize)
        .ok_or_else(|| AppError::Validation("Unknown plan selection".into()))?;
    state.plan_id = plan.id.clone();
    state.settings.last_plan_id = Some(plan.id.clone());
    repo::save_settings(&state.db.conn, &state.settings)?;
    state.selected_date = time::today_in(&plan.timezone);
    refresh_all(ui, state);
    show_toast(ui, state, format!("Switched to {}", plan.name), 1);
    Ok(())
}

pub 
fn apply_theme(ui: &AppWindow, settings: &Settings) {
    let palette = ui.global::<crate::Palette>();
    if settings.theme == "slate" {
        palette.set_bg(slint::Color::from_rgb_u8(13, 16, 22));
        palette.set_sidebar(slint::Color::from_rgb_u8(18, 22, 30));
        palette.set_surface(slint::Color::from_rgb_u8(24, 29, 39));
        palette.set_elevated(slint::Color::from_rgb_u8(31, 38, 50));
        palette.set_border(slint::Color::from_rgb_u8(45, 54, 69));
    } else {
        palette.set_bg(slint::Color::from_rgb_u8(14, 13, 19));
        palette.set_sidebar(slint::Color::from_rgb_u8(20, 18, 27));
        palette.set_surface(slint::Color::from_rgb_u8(27, 24, 38));
        palette.set_elevated(slint::Color::from_rgb_u8(35, 31, 49));
        palette.set_border(slint::Color::from_rgb_u8(49, 42, 64));
    }
    palette.set_accent(color(&settings.accent));
}

fn builder_file(ui: &AppWindow, state: &AppState) -> AppResult<crate::import_export::format::LockinPlanFile> {
    use crate::import_export::format::*;
    let data = ui.global::<crate::IoData>();
    let name = data.get_builder_name().trim().to_string();
    if name.is_empty() {
        return Err(AppError::Validation("Plan name is required.".into()));
    }
    let start = time::parse_date(&data.get_builder_start()).map_err(AppError::Validation)?;
    let end = time::parse_date(&data.get_builder_end()).map_err(AppError::Validation)?;
    if end < start {
        return Err(AppError::Validation("End date must be on or after start date.".into()));
    }
    let tz = data.get_builder_timezone().trim().to_string();
    if tz.parse::<chrono_tz::Tz>().is_err() {
        return Err(AppError::Validation(format!("'{tz}' is not a known IANA timezone.")));
    }
    let mut id = data.get_builder_id().trim().to_string();
    if id.is_empty() {
        let slug: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        id = format!("{}-{}", slug.trim_matches('-'), &uuid::Uuid::new_v4().to_string()[..6]);
    }
    let mut subject_ids: Vec<(String, String)> = Vec::new();
    let mut subjects = Vec::new();
    let colors = ["#9B7BFF", "#69D6A6", "#EABF73", "#7FB2E6", "#E67F90", "#C6A6FF"];
    for (i, row) in data.get_builder_subjects().iter().enumerate() {
        let subject_id = uuid::Uuid::new_v4().to_string();
        subject_ids.push((row.name.to_string(), subject_id.clone()));
        let chapters = row.chapters
            .split([',', '/', ';'])
            .map(|c| c.trim())
            .filter(|c| !c.is_empty())
            .enumerate()
            .map(|(j, c)| ChapterDef {
                id: uuid::Uuid::new_v4().to_string(),
                title: c.to_string(),
                order: j as i64,
                status: None,
            })
            .collect();
        subjects.push(SubjectDef {
            id: subject_id,
            name: row.name.to_string(),
            color: colors[i % colors.len()].into(),
            sort_order: i as i64,
            chapters,
        });
    }
    if subjects.is_empty() {
        return Err(AppError::Validation("Add at least one subject.".into()));
    }
    let mut exams = Vec::new();
    for row in data.get_builder_exams().iter() {
        let subject_id = subject_ids.iter().find(|(n, _)| n == row.subject.as_str()).map(|(_, id)| id.clone())
            .ok_or_else(|| AppError::Validation(format!("Exam '{}' references an unknown subject.", row.label)))?;
        if time::parse_date(&row.date).is_err() || time::parse_time(&row.time).is_err() {
            return Err(AppError::Validation(format!("Exam '{}' has an invalid date or time.", row.label)));
        }
        exams.push(ExamDef {
            id: uuid::Uuid::new_v4().to_string(),
            subject_id,
            label: row.label.to_string(),
            date: row.date.to_string(),
            start_time: row.time.to_string(),
            session: "".into(),
        });
    }
    let _ = state;
    Ok(LockinPlanFile {
        format: FORMAT_ID.into(),
        schema_version: SCHEMA_VERSION,
        plan: PlanHeader {
            id,
            name,
            timezone: tz,
            start_date: start.to_string(),
            end_date: end.to_string(),
            academic_year: data.get_builder_year().to_string(),
        },
        subjects,
        exams,
        schedule_templates: Vec::new(),
        tasks: Vec::new(),
        revision_policy: Some(RevisionPolicy { interval_days: vec![1, 3, 7], adapt_to_results: true }),
    })
}

fn pick_and_write(ui: &AppWindow, state: &AppState, suggested: &str, extension: &str, content: String) {
    let mut dialog = rfd::FileDialog::new().set_file_name(suggested);
    dialog = dialog.add_filter(extension, &[extension]);
    let Some(path) = dialog.save_file() else { return };
    match std::fs::write(&path, content) {
        Ok(()) => show_toast(ui, state, format!("Saved {}", path.display()), 1),
        Err(e) => show_toast(ui, state, format!("Could not save: {e}"), 2),
    }
}

fn set_io_feedback(ui: &AppWindow, tone: i32, text: impl Into<String>) {
    let data = ui.global::<crate::IoData>();
    data.set_feedback(text.into().into());
    data.set_feedback_tone(tone);
}

pub fn run(mut db: Database, settings: Settings, plan_id: String) -> AppResult<()> {
    db.checkpoint();
    let ui = AppWindow::new().map_err(|e| AppError::Other(e.to_string()))?;
    let selected_date = time::today_in(&settings.timezone);
    let state = Rc::new(RefCell::new(AppState {
        db,
        settings,
        plan_id,
        timer: FocusTimer::default(),
        recovered: false,
        ui: ui.as_weak(),
        toast_seq: Rc::new(Cell::new(0)),
        selected_date,
        selected_subject: None,
        selected_chapter: None,
        focus_subject: None,
        focus_chapter: None,
        editing_template: None,
        editor_subjects: Vec::new(),
        editor_chapters: Vec::new(),
        focus_subjects: Vec::new(),
        focus_chapters: Vec::new(),
        mistake_subjects: Vec::new(),
        mistake_chapters: Vec::new(),
        import_preview: None,
        import_file: None,
        backup_pending: None,
        replan: None,
        wizard: WizardDraft::blank(),
        last_autosave: Instant::now(),
        exam_mode_override: false,
    }));

    // Shell
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::Shell>().on_navigate(move |page| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    navigate(&ui, &mut st, page);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::Shell>().on_plan_selected(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = select_plan(&ui, &mut st, index);
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::Shell>().on_quick_focus(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    navigate(&ui, &mut st, 3);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::Shell>().on_open_new_plan(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    navigate(&ui, &mut st, 7);
                    show_toast(&ui, &st, "Build a plan from a validated Lockin Plan file", 0);
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::Shell>().on_dismiss_toast(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::Shell>().set_toast_open(false);
            }
        });
    }

    // Overview
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::OverviewData>().on_open_page(move |page| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    navigate(&ui, &mut st, page);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::OverviewData>().on_start_focus(move |id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = start_for_task(&ui, &mut st, &id);
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::OverviewData>().on_start_chapter(move |id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = start_for_chapter(&ui, &mut st, &id);
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::OverviewData>().on_complete_task(move |id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = set_task_status(&ui, &mut st, &id, TaskStatus::Completed);
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::OverviewData>().on_reopen_task(move |id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = set_task_status(&ui, &mut st, &id, TaskStatus::Planned);
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::OverviewData>().on_skip_task(move |id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = set_task_status(&ui, &mut st, &id, TaskStatus::Skipped);
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::OverviewData>().on_move_to_today(move |id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let mut task = repo::get_task(&st.db.conn, &id)?;
                        task.date = time::today_in(&plan_bundle(&st)?.plan.timezone);
                        repo::update_task(&st.db.conn, &task)?;
                        refresh_all(&ui, &mut st);
                        show_toast(&ui, &st, "Moved to today", 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::OverviewData>().on_open_sleep(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let data = ui.global::<crate::OverviewData>();
                    data.set_sleep_start("22:30".into());
                    data.set_sleep_end("06:45".into());
                    data.set_sleep_kind_index(0);
                    data.set_sleep_energy(4);
                    data.set_sleep_note("".into());
                    data.set_sleep_dialog(true);
                    let _ = refresh_shell(&ui, &mut st);
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::OverviewData>().on_close_sleep(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::OverviewData>().set_sleep_dialog(false);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::OverviewData>().on_save_sleep(move |kind, start, end, energy, note| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = save_sleep(&ui, &mut st, kind, start.to_string(), end.to_string(), energy, note.to_string());
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::OverviewData>().on_delete_sleep(move |id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = repo::delete_sleep(&st.db.conn, &id)
                        .and_then(|_| refresh_overview(&ui, &mut st));
                    report(&ui, &st, result);
                }
            }
        });
    }


    // ── Timetable ────────────────────────────────────────────────
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_shift_day(move |delta| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    st.selected_date += chrono::Duration::days(delta as i64);
                    let _ = refresh_page(&ui, &mut st, 1);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_go_today(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    match plan_bundle(&st) {
                        Ok(b) => st.selected_date = time::today_in(&b.plan.timezone),
                        Err(e) => show_toast(&ui, &st, e.to_string(), 2),
                    }
                    let _ = refresh_page(&ui, &mut st, 1);
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_set_view(move |v| {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::TimetableData>().set_view_mode(v);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_open_new_task(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let data = ui.global::<crate::TimetableData>();
                    data.set_editor_id("".into());
                    data.set_editor_title("".into());
                    data.set_editor_date(st.selected_date.to_string().into());
                    data.set_editor_start("16:00".into());
                    data.set_editor_end("17:00".into());
                    data.set_editor_subject(0);
                    data.set_editor_chapter(0);
                    data.set_editor_type(0);
                    data.set_editor_priority(1);
                    data.set_editor_status(0);
                    data.set_editor_error("".into());
                    data.set_editor_open(true);
                    let _ = refresh_editor_options(&ui, &mut st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_open_edit_task(move |id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let task = repo::get_task(&st.db.conn, &id)?;
                        let bundle = plan_bundle(&st)?;
                        let subject_index = task.subject_id.as_ref()
                            .and_then(|sid| bundle.subjects.iter().position(|(s, _)| &s.id == sid))
                            .unwrap_or(0);
                        let data = ui.global::<crate::TimetableData>();
                        data.set_editor_id(task.id.clone().into());
                        data.set_editor_title(task.title.clone().into());
                        data.set_editor_date(task.date.to_string().into());
                        data.set_editor_start(time::fmt_time(task.start_time).into());
                        data.set_editor_end(time::fmt_time(task.end_time).into());
                        data.set_editor_subject(subject_index as i32);
                        data.set_editor_type(match task.task_type {
                            TaskType::Study => 0, TaskType::Revision => 1, TaskType::MockTest => 2, TaskType::Backlog => 3,
                        });
                        data.set_editor_priority((task.priority - 1).clamp(0, 2) as i32);
                        data.set_editor_status(task_status_index(task.status));
                        data.set_editor_error("".into());
                        refresh_editor_options(&ui, &mut st)?;
                        if let Some(cid) = &task.chapter_id {
                            if let Some(ci) = st.editor_chapters.iter().position(|c| &c.id == cid) {
                                data.set_editor_chapter(ci as i32);
                            }
                        }
                        data.set_editor_open(true);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_close_editor(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::TimetableData>().set_editor_open(false);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_subject_changed(move |_index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    if let Err(e) = refresh_editor_options(&ui, &mut st) {
                        show_toast(&ui, &st, e.to_string(), 2);
                    }
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_save_task(move |id, title, date, start, end, subject, chapter, kind, priority, status| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let data = ui.global::<crate::TimetableData>();
                        let title = title.trim().to_string();
                        if title.is_empty() {
                            data.set_editor_error("A title is required.".into());
                            return Ok(());
                        }
                        let date = time::parse_date(&date).map_err(AppError::Validation)?;
                        let start = time::parse_time(&start).map_err(AppError::Validation)?;
                        let end = time::parse_time(&end).map_err(AppError::Validation)?;
                        if end <= start {
                            data.set_editor_error("End time must be after start time.".into());
                            return Ok(());
                        }
                        let existing = repo::tasks_for_date(&st.db.conn, &st.plan_id, date)?;
                        for other in &existing {
                            if other.id != id.as_str() && crate::scheduling::overlaps(start, end, other.start_time, other.end_time) {
                                data.set_editor_error(format!("Overlaps '{}' ({})", other.title, time::fmt_time(other.start_time)).into());
                                return Ok(());
                            }
                        }
                        let subject_obj = st.editor_subjects.get(subject.max(0) as usize).cloned();
                        let chapter_obj = st.editor_chapters.get(chapter.max(0) as usize).cloned();
                        let task_type = match kind { 1 => TaskType::Revision, 2 => TaskType::MockTest, 3 => TaskType::Backlog, _ => TaskType::Study };
                        let task_status = match status { 1 => TaskStatus::InProgress, 2 => TaskStatus::Completed, 3 => TaskStatus::PartiallyCompleted, 4 => TaskStatus::Skipped, _ => TaskStatus::Planned };
                        let is_new = id.is_empty();
                        let mut task = if is_new {
                            Task {
                                id: uuid::Uuid::new_v4().to_string(),
                                plan_id: st.plan_id.clone(),
                                date,
                                title,
                                task_type,
                                subject_id: subject_obj.as_ref().map(|s| s.id.clone()),
                                chapter_id: chapter_obj.as_ref().map(|c| c.id.clone()),
                                start_time: start,
                                end_time: end,
                                estimated_minutes: time::minutes_between(start, end),
                                priority: priority as i64 + 1,
                                status: task_status,
                                completed_at: None,
                                notes: String::new(),
                                source_template_id: None,
                                created_at: Utc::now().to_rfc3339(),
                            }
                        } else {
                            let mut existing = repo::get_task(&st.db.conn, &id)?;
                            existing.title = title;
                            existing.date = date;
                            existing.start_time = start;
                            existing.end_time = end;
                            existing.estimated_minutes = time::minutes_between(start, end);
                            existing.subject_id = subject_obj.as_ref().map(|s| s.id.clone());
                            existing.chapter_id = chapter_obj.as_ref().map(|c| c.id.clone());
                            existing.task_type = task_type;
                            existing.priority = priority as i64 + 1;
                            existing.status = task_status;
                            existing
                        };
                        if task_status == TaskStatus::Completed && task.completed_at.is_none() {
                            task.completed_at = Some(Utc::now().to_rfc3339());
                        }
                        if is_new { repo::insert_task(&st.db.conn, &task)?; } else { repo::update_task(&st.db.conn, &task)?; }
                        st.selected_date = date;
                        data.set_editor_open(false);
                        refresh_shell(&ui, &mut st)?;
                        refresh_page(&ui, &mut st, 1)?;
                        show_toast(&ui, &st, if is_new { "Task created" } else { "Task updated" }, 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_delete_task(move |id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = repo::delete_task(&st.db.conn, &id)
                        .and_then(|_| {
                            let data = ui.global::<crate::TimetableData>();
                            data.set_editor_open(false);
                            refresh_page(&ui, &mut st, 1)
                        });
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_set_task_status(move |id, status| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let target = match status { 1 => TaskStatus::InProgress, 2 => TaskStatus::Completed, 3 => TaskStatus::PartiallyCompleted, 4 => TaskStatus::Skipped, _ => TaskStatus::Planned };
                    let result = set_task_status(&ui, &mut st, &id, target);
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_start_task(move |id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = start_for_task(&ui, &mut st, &id);
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_open_replan(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let (outcome, _plan) = services::plan::replan_proposal(&st.db.conn, &st.plan_id, st.selected_date)?;
                        let data = ui.global::<crate::TimetableData>();
                        data.set_replan_added(outcome.added.len() as i32);
                        data.set_replan_moved(outcome.backlog_moves.len() as i32);
                        data.set_replan_protected(outcome.protected_conflicts.len() as i32);
                        data.set_replan_has_work(!outcome.backlog_moves.is_empty() || !outcome.protected_conflicts.is_empty());
                        let summary = if outcome.added.is_empty() && outcome.backlog_moves.is_empty() {
                            "No template changes are needed for this day. Review the template or edit tasks directly.".to_string()
                        } else {
                            format!(
                                "{} new template tasks, {} existing tasks moved to backlog, {} protected conflicts left untouched.",
                                outcome.added.len(), outcome.backlog_moves.len(), outcome.protected_conflicts.len()
                            )
                        };
                        data.set_replan_summary(summary.into());
                        data.set_replan_open(true);
                        st.replan = Some(outcome);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_close_replan(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::TimetableData>().set_replan_open(false);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_confirm_replan(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        if let Some(outcome) = st.replan.take() {
                            services::plan::apply_replan(&mut st.db.conn, &outcome)?;
                            ui.global::<crate::TimetableData>().set_replan_open(false);
                            refresh_page(&ui, &mut st, 1)?;
                            show_toast(&ui, &st, "Replan applied; existing work was preserved", 1);
                        } else {
                            show_toast(&ui, &st, "Generate a proposal first", 0);
                        }
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_set_day_type(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        if index == 0 {
                            repo::delete_day_override(&st.db.conn, &st.plan_id, st.selected_date)?;
                        } else {
                            let day_type = match index { 1 => DayType::Weekday, 2 => DayType::Saturday, 3 => DayType::Sunday, _ => DayType::Holiday };
                            repo::set_day_override(&st.db.conn, &DayOverride {
                                plan_id: st.plan_id.clone(),
                                date: st.selected_date,
                                day_type,
                                note: "Set from timetable".into(),
                            })?;
                        }
                        refresh_page(&ui, &mut st, 1)?;
                        show_toast(&ui, &st, "Day type updated · use Replan to materialise template tasks", 0);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_clear_day_type(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = repo::delete_day_override(&st.db.conn, &st.plan_id, st.selected_date)
                        .and_then(|_| refresh_page(&ui, &mut st, 1));
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_toggle_backlog(move || {
            if let Some(ui) = weak.upgrade() {
                let data = ui.global::<crate::TimetableData>();
                let open = data.get_backlog_open();
                data.set_backlog_open(!open);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_move_backlog_today(move |id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let mut task = repo::get_task(&st.db.conn, &id)?;
                        let today = time::today_in(&plan_bundle(&st)?.plan.timezone);
                        task.date = today;
                        task.status = TaskStatus::Planned;
                        repo::update_task(&st.db.conn, &task)?;
                        st.selected_date = today;
                        refresh_page(&ui, &mut st, 1)?;
                        show_toast(&ui, &st, "Moved to today", 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_select_template(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    if let Ok(bundle) = plan_bundle(&st) {
                        if let Some(t) = bundle.templates.get(index.max(0) as usize) {
                            st.editing_template = Some(t.id.clone());
                        }
                    }
                    let _ = refresh_page(&ui, &mut st, 1);
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_open_template_panel(move || {
            if let Some(ui) = weak.upgrade() {
                let data = ui.global::<crate::TimetableData>();
                let open = data.get_template_open();
                data.set_template_open(!open);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_add_template_slot(move || {
            if let Some(ui) = weak.upgrade() {
                let data = ui.global::<crate::TimetableData>();
                data.set_slot_id("".into());
                data.set_slot_title("".into());
                data.set_slot_start("16:00".into());
                data.set_slot_end("17:00".into());
                data.set_slot_kind(0);
                data.set_slot_subject(0);
                data.set_slot_error("".into());
                data.set_slot_open(true);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_edit_template_slot(move |id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let data = ui.global::<crate::TimetableData>();
                        let template_id = st.editing_template.clone().unwrap_or_default();
                        let templates = repo::templates_for_plan(&st.db.conn, &st.plan_id)?;
                        let template = templates.iter().find(|t| t.id == template_id);
                        if let Some(slot) = template.and_then(|t| t.slots.iter().find(|s| s.id == id.as_str())) {
                            data.set_slot_id(slot.id.clone().into());
                            data.set_slot_title(slot.title.clone().into());
                            data.set_slot_start(time::fmt_time(slot.start_time).into());
                            data.set_slot_end(time::fmt_time(slot.end_time).into());
                            data.set_slot_kind(match slot.kind { SlotKind::Study => 0, SlotKind::Break => 1, SlotKind::Meal => 2 });
                            let bundle = plan_bundle(&st)?;
                            data.set_slot_subject(slot.subject_id.as_ref().and_then(|sid| bundle.subjects.iter().position(|(s, _)| &s.id == sid)).unwrap_or(0) as i32);
                            data.set_slot_error("".into());
                            data.set_slot_open(true);
                        }
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_close_slot_editor(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::TimetableData>().set_slot_open(false);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_save_template_slot(move |id, title, start, end, kind, subject| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let data = ui.global::<crate::TimetableData>();
                        let start_t = match time::parse_time(&start) { Ok(t) => t, Err(e) => { data.set_slot_error(e.into()); return Ok(()); } };
                        let end_t = match time::parse_time(&end) { Ok(t) => t, Err(e) => { data.set_slot_error(e.into()); return Ok(()); } };
                        if end_t <= start_t { data.set_slot_error("End time must be after start.".into()); return Ok(()); }
                        let template_id = st.editing_template.clone().unwrap_or_default();
                        if template_id.is_empty() { data.set_slot_error("Select a template first.".into()); return Ok(()); }
                        let bundle = plan_bundle(&st)?;
                        let subject_id = if kind == 0 {
                            bundle.subjects.get(subject.max(0) as usize).map(|(s, _)| s.id.clone())
                        } else { None };
                        let slot_kind = match kind { 1 => SlotKind::Break, 2 => SlotKind::Meal, _ => SlotKind::Study };
                        let slot = TemplateSlot {
                            id: if id.is_empty() { uuid::Uuid::new_v4().to_string() } else { id.to_string() },
                            template_id: template_id.clone(),
                            start_time: start_t,
                            end_time: end_t,
                            kind: slot_kind,
                            title: if title.trim().is_empty() { slot_kind.label().to_string() } else { title.trim().to_string() },
                            subject_id,
                            chapter_id: None,
                        };
                        if id.is_empty() {
                            let count = bundle.templates.iter().find(|t| t.id == template_id).map(|t| t.slots.len()).unwrap_or(0);
                            repo::insert_slot(&st.db.conn, &slot, count as i64)?;
                        } else {
                            repo::update_slot(&st.db.conn, &slot)?;
                        }
                        data.set_slot_open(false);
                        refresh_page(&ui, &mut st, 1)?;
                        show_toast(&ui, &st, "Template slot saved", 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_delete_template_slot(move |id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = repo::delete_slot(&st.db.conn, &id)
                        .and_then(|_| refresh_page(&ui, &mut st, 1));
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_apply_template(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let (outcome, _plan) = services::plan::replan_proposal(&st.db.conn, &st.plan_id, st.selected_date)?;
                        services::plan::apply_replan(&mut st.db.conn, &outcome)?;
                        refresh_page(&ui, &mut st, 1)?;
                        show_toast(&ui, &st, format!("Template applied · {} tasks added", outcome.added.len()), 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::TimetableData>().on_open_day(move |date| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    match time::parse_date(&date) {
                        Ok(d) => {
                            st.selected_date = d;
                            ui.global::<crate::TimetableData>().set_view_mode(0);
                            let _ = refresh_page(&ui, &mut st, 1);
                        }
                        Err(e) => show_toast(&ui, &st, e, 2),
                    }
                }
            }
        });
    }


    // ── Focus ────────────────────────────────────────────────────
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::FocusData>().on_start(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    if st.timer.state != crate::timer::TimerState::Idle {
                        show_toast(&ui, &st, "A session is already active", 0);
                        return;
                    }
                    let result = (|| -> AppResult<()> {
                        let data = ui.global::<crate::FocusData>();
                        let preset = Preset::from_index(data.get_preset());
                        let focus = match preset.focus_minutes() {
                            Some(m) => Duration::from_secs(m as u64 * 60),
                            None => {
                                let mins: i64 = data.get_custom_focus().parse().unwrap_or(45).clamp(5, 240);
                                Duration::from_secs(mins as u64 * 60)
                            }
                        };
                        let brk = match preset.break_minutes() {
                            Some(m) => Duration::from_secs(m as u64 * 60),
                            None => {
                                let mins: i64 = data.get_custom_break().parse().unwrap_or(10).clamp(1, 60);
                                Duration::from_secs(mins as u64 * 60)
                            }
                        };
                        let subject = st.focus_subjects.get(data.get_subject_index().max(0) as usize).cloned();
                        let chapter = st.focus_chapters.get(data.get_chapter_index().max(0) as usize).cloned();
                        let bundle = plan_bundle(&st)?;
                        st.timer = FocusTimer::new(preset, preset.key(), focus, brk, subject.map(|s| s.id), chapter.map(|c| c.id), None, bundle.plan.tz());
                        st.recovered = false;
                        persist_timer(&st)?;
                        refresh_page(&ui, &mut st, 3)?;
                        show_toast(&ui, &st, "Focus started", 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::FocusData>().on_pause(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    st.timer.pause();
                    let _ = persist_timer(&st);
                    let _ = refresh_page(&ui, &mut st, 3);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::FocusData>().on_resume(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    st.timer.resume();
                    let _ = persist_timer(&st);
                    let _ = refresh_page(&ui, &mut st, 3);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::FocusData>().on_interrupt(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    st.timer.interrupt();
                    let _ = persist_timer(&st);
                    let _ = refresh_page(&ui, &mut st, 3);
                    show_toast(&ui, &st, "Interruption recorded · timer paused", 0);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::FocusData>().on_take_break(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    st.timer.enter_break();
                    let _ = persist_timer(&st);
                    let _ = refresh_page(&ui, &mut st, 3);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::FocusData>().on_end_break(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    st.timer.end_break();
                    let _ = persist_timer(&st);
                    let _ = refresh_page(&ui, &mut st, 3);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::FocusData>().on_finish(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let focused = st.timer.focus_elapsed().as_secs();
                        if focused > 0 {
                            let plan_id = st.plan_id.clone();
                            let timer = st.timer.clone();
                            services::focus::save_session(&mut st.db.conn, &plan_id, &timer)?;
                        }
                        repo::clear_timer(&st.db.conn)?;
                        st.timer = FocusTimer::default();
                        refresh_shell(&ui, &mut st)?;
                        refresh_page(&ui, &mut st, 3)?;
                        if focused > 0 {
                            show_toast(&ui, &st, format!("Session saved · {} focused", time::human_duration(focused as i64)), 1);
                        } else {
                            show_toast(&ui, &st, "Session discarded (no focused time)", 0);
                        }
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::FocusData>().on_discard(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = repo::clear_timer(&st.db.conn).map(|_| {
                        st.timer = FocusTimer::default();
                        st.recovered = false;
                    }).and_then(|_| refresh_page(&ui, &mut st, 3));
                    report(&ui, &st, result);
                    show_toast(&ui, &st, "Active timer discarded", 0);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::FocusData>().on_set_preset(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let data = ui.global::<crate::FocusData>();
                    data.set_preset(index);
                    if st.timer.state == crate::timer::TimerState::Idle {
                        let preset = Preset::from_index(index);
                        data.set_custom_focus(preset.focus_minutes().unwrap_or(45).to_string().into());
                        data.set_custom_break(preset.break_minutes().unwrap_or(10).to_string().into());
                    } else {
                        show_toast(&ui, &st, "Preset will apply to the next session", 0);
                    }
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::FocusData>().on_apply_custom(move |focus, brk| {
            if let Some(ui) = weak.upgrade() {
                let data = ui.global::<crate::FocusData>();
                if focus.parse::<i64>().is_err() || brk.parse::<i64>().is_err() {
                    data.set_error("Custom focus and break must be whole minutes.".into());
                } else {
                    data.set_error("".into());
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::FocusData>().on_set_subject(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let data = ui.global::<crate::FocusData>();
                    data.set_subject_index(index);
                    if let Ok(bundle) = plan_bundle(&st) {
                        if let Some((subject, _)) = bundle.subjects.get(index.max(0) as usize) {
                            st.focus_chapters = repo::chapters_for_subject(&st.db.conn, &subject.id).unwrap_or_default();
                            data.set_chapter_names(model(st.focus_chapters.iter().map(|c| c.title.clone().into()).collect()));
                            data.set_chapter_index(0);
                        }
                    }
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::FocusData>().on_set_chapter(move |index| {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::FocusData>().set_chapter_index(index);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::FocusData>().on_resume_recovered(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    st.timer.state = crate::timer::TimerState::Paused;
                    st.timer.resume();
                    st.recovered = false;
                    ui.global::<crate::FocusData>().set_recovery_open(false);
                    let _ = persist_timer(&st);
                    let _ = refresh_page(&ui, &mut st, 3);
                    show_toast(&ui, &st, "Recovered timer resumed", 1);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::FocusData>().on_discard_recovered(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let _ = repo::clear_timer(&st.db.conn);
                    st.timer = FocusTimer::default();
                    st.recovered = false;
                    ui.global::<crate::FocusData>().set_recovery_open(false);
                    let _ = refresh_page(&ui, &mut st, 3);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::FocusData>().on_open_timetable(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    navigate(&ui, &mut st, 1);
                }
            }
        });
    }


    // ── Syllabus ─────────────────────────────────────────────────
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SyllabusData>().on_select_subject(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    ui.global::<crate::SyllabusData>().set_subject_index(index);
                    let _ = refresh_syllabus(&ui, &mut st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SyllabusData>().on_select_chapter(move |id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let chapter = repo::get_chapter(&st.db.conn, &id)?;
                        let subject = repo::get_subject(&st.db.conn, &chapter.subject_id)?;
                        st.selected_chapter = Some(chapter.id.clone());
                        let data = ui.global::<crate::SyllabusData>();
                        data.set_detail_id(chapter.id.clone().into());
                        data.set_detail_title(chapter.title.clone().into());
                        data.set_detail_subject(subject.name.into());
                        data.set_detail_status(chapter_status_index(chapter.status));
                        data.set_concept(chapter.concept_done);
                        data.set_textbook(chapter.textbook_done);
                        data.set_exercises(chapter.exercises_done);
                        data.set_selftest(chapter.selftest_done);
                        data.set_confidence(chapter.confidence.unwrap_or(0) as i32);
                        data.set_questions(chapter.questions_attempted.to_string().into());
                        data.set_correct(chapter.correct_answers.to_string().into());
                        data.set_accuracy_text(if chapter.questions_attempted > 0 {
                            format!("{:.0}%", chapter.correct_answers as f64 / chapter.questions_attempted as f64 * 100.0).into()
                        } else { "—".into() });
                        data.set_study_text(time::human_duration(chapter.study_seconds).into());
                        data.set_last_studied(chapter.last_studied.map(time::fmt_date_short).unwrap_or_else(|| "—".into()).into());
                        data.set_next_revision(chapter.next_revision.map(time::fmt_date_short).unwrap_or_else(|| "—".into()).into());
                        data.set_notes(chapter.notes.clone().into());
                        data.set_detail_open(true);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::SyllabusData>().on_close_detail(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::SyllabusData>().set_detail_open(false);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SyllabusData>().on_search_changed(move |text| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    ui.global::<crate::SyllabusData>().set_search(text);
                    let _ = refresh_syllabus(&ui, &mut st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SyllabusData>().on_set_filter(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    ui.global::<crate::SyllabusData>().set_filter(index);
                    let _ = refresh_syllabus(&ui, &mut st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SyllabusData>().on_toggle_check(move |item| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let data = ui.global::<crate::SyllabusData>();
                        let id = data.get_detail_id().to_string();
                        let mut chapter = repo::get_chapter(&st.db.conn, &id)?;
                        match item {
                            0 => chapter.concept_done = !chapter.concept_done,
                            1 => chapter.textbook_done = !chapter.textbook_done,
                            2 => chapter.exercises_done = !chapter.exercises_done,
                            _ => chapter.selftest_done = !chapter.selftest_done,
                        }
                        repo::update_chapter(&st.db.conn, &chapter)?;
                        data.set_concept(chapter.concept_done);
                        data.set_textbook(chapter.textbook_done);
                        data.set_exercises(chapter.exercises_done);
                        data.set_selftest(chapter.selftest_done);
                        refresh_syllabus(&ui, &mut st)?;
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SyllabusData>().on_set_status(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let id = ui.global::<crate::SyllabusData>().get_detail_id().to_string();
                        let mut chapter = repo::get_chapter(&st.db.conn, &id)?;
                        chapter.status = match index {
                            1 => ChapterStatus::Learning,
                            2 => ChapterStatus::NeedsPractice,
                            3 => ChapterStatus::NeedsRevision,
                            4 => ChapterStatus::ExamReady,
                            _ => ChapterStatus::NotStarted,
                        };
                        repo::update_chapter(&st.db.conn, &chapter)?;
                        ui.global::<crate::SyllabusData>().set_detail_status(index);
                        refresh_syllabus(&ui, &mut st)?;
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SyllabusData>().on_set_confidence(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let id = ui.global::<crate::SyllabusData>().get_detail_id().to_string();
                        let mut chapter = repo::get_chapter(&st.db.conn, &id)?;
                        chapter.confidence = if index == 0 { None } else { Some(index as i64) };
                        repo::update_chapter(&st.db.conn, &chapter)?;
                        ui.global::<crate::SyllabusData>().set_confidence(index);
                        refresh_syllabus(&ui, &mut st)?;
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SyllabusData>().on_save_metrics(move |questions, correct| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let q: i64 = questions.trim().parse().map_err(|_| AppError::Validation("Questions must be a whole number".into()))?;
                        let c: i64 = correct.trim().parse().map_err(|_| AppError::Validation("Correct must be a whole number".into()))?;
                        if q < 0 || c < 0 || c > q {
                            return Err(AppError::Validation("Correct answers cannot exceed questions attempted".into()));
                        }
                        let data = ui.global::<crate::SyllabusData>();
                        let id = data.get_detail_id().to_string();
                        let mut chapter = repo::get_chapter(&st.db.conn, &id)?;
                        chapter.questions_attempted = q;
                        chapter.correct_answers = c;
                        repo::update_chapter(&st.db.conn, &chapter)?;
                        data.set_accuracy_text(if q > 0 { format!("{:.0}%", c as f64 / q as f64 * 100.0).into() } else { "—".into() });
                        show_toast(&ui, &st, "Practice marks saved", 1);
                        refresh_syllabus(&ui, &mut st)?;
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SyllabusData>().on_save_notes(move |notes| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let id = ui.global::<crate::SyllabusData>().get_detail_id().to_string();
                        let mut chapter = repo::get_chapter(&st.db.conn, &id)?;
                        chapter.notes = notes.to_string();
                        repo::update_chapter(&st.db.conn, &chapter)?;
                        show_toast(&ui, &st, "Notes saved", 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SyllabusData>().on_generate_revisions(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let id = ui.global::<crate::SyllabusData>().get_detail_id().to_string();
                        let today = time::today_in(&plan_bundle(&st)?.plan.timezone);
                        let plan_id = st.plan_id.clone();
                        let intervals = st.settings.revision_intervals.clone();
                        let created = services::revision::generate_for_chapter(
                            &mut st.db.conn,
                            &plan_id,
                            &id,
                            &intervals,
                            today,
                        )?;
                        refresh_shell(&ui, &mut st)?;
                        show_toast(&ui, &st, format!("{created} revision reviews scheduled"), 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::SyllabusData>().on_open_add_chapter(move || {
            if let Some(ui) = weak.upgrade() {
                let data = ui.global::<crate::SyllabusData>();
                data.set_new_chapter_title("".into());
                data.set_add_chapter_open(true);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::SyllabusData>().on_close_add_chapter(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::SyllabusData>().set_add_chapter_open(false);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SyllabusData>().on_add_chapter(move |title| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let title = title.trim().to_string();
                        if title.is_empty() {
                            return Err(AppError::Validation("Chapter title is required".into()));
                        }
                        let subject_id = ui.global::<crate::SyllabusData>().get_subject_id().to_string();
                        if subject_id.is_empty() {
                            return Err(AppError::Validation("Select a subject first".into()));
                        }
                        let existing = repo::chapters_for_subject(&st.db.conn, &subject_id)?;
                        let chapter = Chapter {
                            id: uuid::Uuid::new_v4().to_string(),
                            subject_id,
                            title,
                            sort_order: existing.len() as i64,
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
                        };
                        repo::insert_chapter(&st.db.conn, &chapter)?;
                        ui.global::<crate::SyllabusData>().set_add_chapter_open(false);
                        refresh_syllabus(&ui, &mut st)?;
                        show_toast(&ui, &st, "Chapter added", 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SyllabusData>().on_delete_chapter(move |id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = repo::delete_chapter(&st.db.conn, &id)
                        .and_then(|_| {
                            ui.global::<crate::SyllabusData>().set_detail_open(false);
                            refresh_syllabus(&ui, &mut st)
                        });
                    report(&ui, &st, result);
                }
            }
        });
    }


    // ── Revision ─────────────────────────────────────────────────
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::RevisionData>().on_set_tab(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    ui.global::<crate::RevisionData>().set_tab(index);
                    let _ = refresh_revision(&ui, &mut st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::RevisionData>().on_grade(move |id, result| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let grade = match result { 0 => RevisionResult::Easy, 2 => RevisionResult::Difficult, 3 => RevisionResult::Failed, _ => RevisionResult::Good };
                        let today = time::today_in(&plan_bundle(&st)?.plan.timezone);
                        let intervals = st.settings.revision_intervals.clone();
                        services::revision::grade(&mut st.db.conn, &id, grade, today, &intervals)?;
                        refresh_shell(&ui, &mut st)?;
                        refresh_revision(&ui, &mut st)?;
                        refresh_overview(&ui, &mut st)?;
                        show_toast(&ui, &st, format!("Review graded {}", grade.label()), 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::RevisionData>().on_snooze(move |id, days| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let today = time::today_in(&plan_bundle(&st)?.plan.timezone);
                        services::revision::snooze(&mut st.db.conn, &id, days as i64, today)?;
                        ui.global::<crate::RevisionData>().set_reschedule_open(false);
                        refresh_shell(&ui, &mut st)?;
                        refresh_revision(&ui, &mut st)?;
                        show_toast(&ui, &st, format!("Review snoozed {days} day(s)"), 0);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::RevisionData>().on_open_reschedule(move |id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    if let Ok(bundle) = plan_bundle(&st) {
                        let data = ui.global::<crate::RevisionData>();
                        data.set_reschedule_id(id);
                        data.set_reschedule_date(time::today_in(&bundle.plan.timezone).to_string().into());
                        data.set_reschedule_open(true);
                    }
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::RevisionData>().on_close_reschedule(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::RevisionData>().set_reschedule_open(false);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::RevisionData>().on_save_reschedule(move |date| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let data = ui.global::<crate::RevisionData>();
                        let id = data.get_reschedule_id().to_string();
                        let date = time::parse_date(&date).map_err(AppError::Validation)?;
                        services::revision::reschedule(&mut st.db.conn, &id, date)?;
                        data.set_reschedule_open(false);
                        refresh_shell(&ui, &mut st)?;
                        refresh_revision(&ui, &mut st)?;
                        show_toast(&ui, &st, "Review rescheduled", 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::RevisionData>().on_start_revision(move |chapter_id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = start_for_chapter(&ui, &mut st, &chapter_id);
                    report(&ui, &st, result);
                }
            }
        });
    }

    // ── Exams ────────────────────────────────────────────────────
    {
        let weak = ui.as_weak();
        ui.global::<crate::ExamsData>().on_add_mock(move || {
            if let Some(ui) = weak.upgrade() {
                let data = ui.global::<crate::ExamsData>();
                data.set_mock_name("".into());
                data.set_mock_subject(0);
                data.set_mock_score("0".into());
                data.set_mock_max("100".into());
                data.set_mock_duration("90".into());
                data.set_mock_weak("".into());
                data.set_mock_notes("".into());
                data.set_mock_error("".into());
                data.set_mock_open(true);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::ExamsData>().on_close_mock(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::ExamsData>().set_mock_open(false);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::ExamsData>().on_save_mock(move |name, subject, date, score, max, duration, weak_topics, notes| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let data = ui.global::<crate::ExamsData>();
                        if name.trim().is_empty() { data.set_mock_error("A paper name is required.".into()); return Ok(()); }
                        let date = match time::parse_date(&date) { Ok(d) => d, Err(e) => { data.set_mock_error(e.into()); return Ok(()); } };
                        let score: f64 = match score.trim().parse() { Ok(v) => v, Err(_) => { data.set_mock_error("Score must be a number.".into()); return Ok(()); } };
                        let max_marks: f64 = match max.trim().parse() { Ok(v) => v, Err(_) => { data.set_mock_error("Max marks must be a number.".into()); return Ok(()); } };
                        let duration: i64 = match duration.trim().parse() { Ok(v) => v, Err(_) => { data.set_mock_error("Duration must be whole minutes.".into()); return Ok(()); } };
                        if max_marks <= 0.0 || score < 0.0 || score > max_marks {
                            data.set_mock_error("Score must be between zero and max marks.".into());
                            return Ok(());
                        }
                        let bundle = plan_bundle(&st)?;
                        let subject_obj = bundle.subjects.get(subject.max(0) as usize).cloned();
                        let Some((subject_obj, _)) = subject_obj else {
                            data.set_mock_error("Add a subject first.".into());
                            return Ok(());
                        };
                        let mock = MockTest {
                            id: uuid::Uuid::new_v4().to_string(),
                            plan_id: st.plan_id.clone(),
                            subject_id: subject_obj.id.clone(),
                            name: name.trim().to_string(),
                            date,
                            score,
                            max_marks,
                            duration_minutes: duration.max(1),
                            weak_topics: weak_topics.trim().to_string(),
                            notes: notes.to_string(),
                            created_at: Utc::now().to_rfc3339(),
                        };
                        repo::insert_mock_test(&st.db.conn, &mock)?;
                        data.set_mock_open(false);
                        refresh_exams(&ui, &mut st)?;
                        show_toast(&ui, &st, "Mock test saved", 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::ExamsData>().on_delete_mock(move |id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = repo::delete_mock_test(&st.db.conn, &id)
                        .and_then(|_| refresh_exams(&ui, &mut st));
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::ExamsData>().on_add_mistake(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let data = ui.global::<crate::ExamsData>();
                    data.set_mistake_subject(0);
                    data.set_mistake_chapter(0);
                    data.set_mistake_question("".into());
                    data.set_mistake_explanation("".into());
                    data.set_mistake_error("".into());
                    if let Ok(bundle) = plan_bundle(&st) {
                        st.mistake_subjects = bundle.subjects.iter().map(|(s, _)| s.clone()).collect();
                        st.mistake_chapters = bundle.subjects.first().map(|(_, c)| c.clone()).unwrap_or_default();
                        data.set_chapter_names(model(st.mistake_chapters.iter().map(|c| c.title.clone().into()).collect()));
                    }
                    data.set_mistake_open(true);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::ExamsData>().on_mistake_subject_changed(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let data = ui.global::<crate::ExamsData>();
                    if let Ok(bundle) = plan_bundle(&st) {
                        let (_, chapters) = match bundle.subjects.get(index.max(0) as usize) {
                            Some(pair) => pair.clone(),
                            None => (Subject { id: String::new(), plan_id: String::new(), name: String::new(), color: String::new(), sort_order: 0, portions_pending: false }, Vec::new()),
                        };
                        st.mistake_chapters = chapters;
                        data.set_chapter_names(model(st.mistake_chapters.iter().map(|c| c.title.clone().into()).collect()));
                        data.set_mistake_chapter(0);
                    }
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::ExamsData>().on_close_mistake(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::ExamsData>().set_mistake_open(false);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::ExamsData>().on_save_mistake(move |subject, chapter, question, explanation| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let data = ui.global::<crate::ExamsData>();
                        if question.trim().is_empty() {
                            data.set_mistake_error("Describe the question or trap.".into());
                            return Ok(());
                        }
                        let Some(subject_obj) = st.mistake_subjects.get(subject.max(0) as usize).cloned() else {
                            data.set_mistake_error("Select a subject.".into());
                            return Ok(());
                        };
                        let mistake = Mistake {
                            id: uuid::Uuid::new_v4().to_string(),
                            plan_id: st.plan_id.clone(),
                            mock_test_id: None,
                            subject_id: subject_obj.id,
                            chapter_id: st.mistake_chapters.get(chapter.max(0) as usize).map(|c| c.id.clone()),
                            question: question.trim().to_string(),
                            explanation: explanation.to_string(),
                            resolved: false,
                            created_at: Utc::now().to_rfc3339(),
                        };
                        repo::insert_mistake(&st.db.conn, &mistake)?;
                        data.set_mistake_open(false);
                        refresh_exams(&ui, &mut st)?;
                        show_toast(&ui, &st, "Mistake logged", 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::ExamsData>().on_toggle_mistake(move |id, resolved| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = repo::set_mistake_resolved(&st.db.conn, &id, resolved)
                        .and_then(|_| refresh_exams(&ui, &mut st));
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::ExamsData>().on_set_exam_mode(move |on| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    st.exam_mode_override = on;
                    ui.global::<crate::Shell>().set_exam_mode(on);
                    ui.global::<crate::ExamsData>().set_exam_mode(on);
                    show_toast(&ui, &st, if on { "Exam mode enabled" } else { "Exam mode disabled" }, 0);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::ExamsData>().on_study_paper(move |subject_id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let chapters = repo::chapters_for_subject(&st.db.conn, &subject_id)?;
                        let pick = chapters.iter().filter(|c| !c.status.is_complete()).min_by_key(|c| c.confidence.unwrap_or(0)).or_else(|| chapters.first());
                        match pick {
                            Some(chapter) => start_for_chapter(&ui, &mut st, &chapter.id),
                            None => Err(AppError::NotFound("a chapter for this subject".into())),
                        }
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }

    // ── Analytics ────────────────────────────────────────────────
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::AnalyticsData>().on_set_range(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    ui.global::<crate::AnalyticsData>().set_range(index);
                    let _ = refresh_analytics(&ui, &mut st);
                }
            }
        });
    }


    // ── Import / Export ──────────────────────────────────────────
    {
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_set_tab(move |index| {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::IoData>().set_tab(index);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_editor_changed(move |text| {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::IoData>().set_editor_text(text);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_format_json(move || {
            if let Some(ui) = weak.upgrade() {
                let data = ui.global::<crate::IoData>();
                let raw = data.get_editor_text().to_string();
                match serde_json::from_str::<serde_json::Value>(&raw) {
                    Ok(value) => match serde_json::to_string_pretty(&value) {
                        Ok(text) => { data.set_editor_text(text.into()); set_io_feedback(&ui, 1, "JSON formatted"); }
                        Err(e) => set_io_feedback(&ui, 2, format!("Could not format: {e}")),
                    },
                    Err(e) => set_io_feedback(&ui, 2, format!("Invalid JSON: {e}")),
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_validate_json(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let data = ui.global::<crate::IoData>();
                    let raw = data.get_editor_text().to_string();
                    match crate::import_export::validate::prepare(&st.db.conn, &raw) {
                        Ok(prepared) => {
                            let warnings = prepared.issues.iter().filter(|i| !i.is_error()).count();
                            let errors = prepared.issues.iter().filter(|i| i.is_error()).count();
                            if prepared.has_errors() {
                                set_io_feedback(&ui, 2, prepared.error_text());
                            } else {
                                set_io_feedback(&ui, 1, format!("Valid Lockin Plan · {} warnings", warnings));
                            }
                            let _ = errors;
                            st.import_preview = Some(prepared);
                            let _ = refresh_io(&ui, &mut st);
                        }
                        Err(e) => set_io_feedback(&ui, 2, e.to_string()),
                    }
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_preview_import(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let raw = ui.global::<crate::IoData>().get_editor_text().to_string();
                    match crate::import_export::validate::prepare(&st.db.conn, &raw) {
                        Ok(prepared) => {
                            st.import_preview = Some(prepared);
                            let _ = refresh_io(&ui, &mut st);
                        }
                        Err(e) => set_io_feedback(&ui, 2, e.to_string()),
                    }
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_close_preview(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::IoData>().set_preview_open(false);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_apply_import(move |mode| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let prepared = st.import_preview.clone().ok_or_else(|| AppError::Validation("Validate or preview a file first.".into()))?;
                        if prepared.has_errors() {
                            return Err(AppError::Validation(prepared.error_text()));
                        }
                        let summary = crate::import_export::apply::apply(&mut st.db.conn, &prepared, ImportMode::from_index(mode))?;
                        st.plan_id = summary.plan_id.clone();
                        st.settings.last_plan_id = Some(summary.plan_id.clone());
                        repo::save_settings(&st.db.conn, &st.settings)?;
                        st.import_preview = None;
                        st.selected_date = time::today_in(&plan_bundle(&st)?.plan.timezone);
                        let data = ui.global::<crate::IoData>();
                        data.set_preview_open(false);
                        refresh_all(&ui, &mut st);
                        set_io_feedback(&ui, 1, summary.message());
                        show_toast(&ui, &st, "Import applied", 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_pick_file(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let Some(path) = rfd::FileDialog::new().add_filter("Lockin JSON", &["json"]).pick_file() else { return Ok(()); };
                        let raw = std::fs::read_to_string(&path)?;
                        st.import_file = Some(path.clone());
                        ui.global::<crate::IoData>().set_file_path(path.display().to_string().into());
                        if raw.len() > crate::import_export::MAX_BACKUP_BYTES {
                            return Err(AppError::Validation("File exceeds the backup size limit.".into()));
                        }
                        if let Ok(backup) = crate::import_export::backup::parse(&raw) {
                            let counts = backup.counts();
                            st.backup_pending = Some(backup);
                            set_io_feedback(&ui, 1, format!("Backup validated · {counts} · press Restore backup to apply"));
                            return Ok(());
                        }
                        if raw.len() > crate::import_export::MAX_IMPORT_BYTES {
                            return Err(AppError::Validation("Plan file exceeds the 5 MB import limit.".into()));
                        }
                        match crate::import_export::validate::prepare(&st.db.conn, &raw) {
                            Ok(prepared) => {
                                st.import_preview = Some(prepared);
                                let _ = refresh_io(&ui, &mut st);
                                set_io_feedback(&ui, 1, "File loaded · review the import preview");
                            }
                            Err(e) => set_io_feedback(&ui, 2, e.to_string()),
                        }
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_copy_json(move || {
            if let Some(ui) = weak.upgrade() {
                let data = ui.global::<crate::IoData>();
                let text = data.get_editor_text().to_string();
                match arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
                    Ok(()) => show_toast(&ui, &state.borrow(), "JSON copied", 1),
                    Err(e) => show_toast(&ui, &state.borrow(), format!("Clipboard error: {e}"), 2),
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_empty_template(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(st) = state.try_borrow() {
                    match crate::import_export::format::to_pretty_json(&crate::import_export::format::LockinPlanFile::empty_template()) {
                        Ok(text) => {
                            ui.global::<crate::IoData>().set_editor_text(text.into());
                            set_io_feedback(&ui, 1, "Empty template loaded");
                        }
                        Err(e) => set_io_feedback(&ui, 2, e.to_string()),
                    }
                    let _ = &st;
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_save_json_file(move || {
            if let Some(ui) = weak.upgrade() {
                let data = ui.global::<crate::IoData>();
                let text = data.get_editor_text().to_string();
                let st = state.borrow();
                pick_and_write(&ui, &st, "lockin-plan.json", "json", text);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_export_schema(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(st) = state.try_borrow() {
                    pick_and_write(&ui, &st, "lockin-plan.schema.json", "json", crate::import_export::format::SCHEMA_JSON.to_string());
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_export_plan(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(st) = state.try_borrow() {
                    match crate::import_export::format::export_plan(&st.db.conn, &st.plan_id)
                        .and_then(|file| crate::import_export::format::to_pretty_json(&file).map_err(AppError::from))
                    {
                        Ok(text) => {
                            let name = format!("{}.json", st.plan_id);
                            pick_and_write(&ui, &st, &name, "json", text);
                        }
                        Err(e) => set_io_feedback(&ui, 2, e.to_string()),
                    }
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_export_backup(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(st) = state.try_borrow() {
                    match crate::import_export::backup::export(&st.db.conn)
                        .and_then(|b| crate::import_export::backup::to_json(&b))
                    {
                        Ok(text) => pick_and_write(&ui, &st, "lockin-backup.json", "json", text),
                        Err(e) => set_io_feedback(&ui, 2, e.to_string()),
                    }
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_restore_backup(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let backup = if let Some(b) = st.backup_pending.clone() {
                            b
                        } else {
                            let Some(path) = rfd::FileDialog::new().add_filter("Lockin backup", &["json"]).pick_file() else { return Ok(()); };
                            let raw = std::fs::read_to_string(&path)?;
                            crate::import_export::backup::parse(&raw)?
                        };
                        crate::import_export::backup::restore(&mut st.db.conn, &backup)?;
                        st.backup_pending = None;
                        st.settings = repo::load_settings(&st.db.conn)?;
                        st.plan_id = services::plan::current_plan_id(&st.db.conn, &st.settings)?;
                        st.selected_date = time::today_in(&plan_bundle(&st)?.plan.timezone);
                        apply_theme(&ui, &st.settings);
                        refresh_all(&ui, &mut st);
                        set_io_feedback(&ui, 1, "Backup restored");
                        show_toast(&ui, &st, "Backup restored", 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_load_builder_current(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let file = crate::import_export::format::export_plan(&st.db.conn, &st.plan_id)?;
                        let data = ui.global::<crate::IoData>();
                        data.set_builder_name(file.plan.name.clone().into());
                        data.set_builder_id(file.plan.id.clone().into());
                        data.set_builder_year(file.plan.academic_year.clone().into());
                        data.set_builder_start(file.plan.start_date.clone().into());
                        data.set_builder_end(file.plan.end_date.clone().into());
                        data.set_builder_timezone(file.plan.timezone.clone().into());
                        data.set_builder_subjects(model(file.subjects.iter().map(|s| crate::BuilderSubjectRow {
                            index: 0,
                            name: s.name.clone().into(),
                            tint: color(&s.color),
                            chapters: s.chapters.iter().map(|c| c.title.clone()).collect::<Vec<_>>().join(", ").into(),
                        }).collect()));
                        data.set_builder_exams(model(file.exams.iter().map(|e| crate::BuilderExamRow {
                            index: 0,
                            label: e.label.clone().into(),
                            subject: file.subjects.iter().find(|s| s.id == e.subject_id).map(|s| s.name.clone()).unwrap_or_default().into(),
                            date: e.date.clone().into(),
                            time: e.start_time.clone().into(),
                        }).collect()));
                        data.set_builder_subject_names(model(file.subjects.iter().map(|s| s.name.clone().into()).collect()));
                        set_io_feedback(&ui, 1, "Builder filled from the current plan");
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_builder_add_subject(move |name, chapters| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let data = ui.global::<crate::IoData>();
                    if name.trim().is_empty() { data.set_builder_error("Subject name is required".into()); return; }
                    let mut rows: Vec<crate::BuilderSubjectRow> = data.get_builder_subjects().iter().collect();
                    rows.push(crate::BuilderSubjectRow {
                        index: rows.len() as i32,
                        name: name.trim().into(),
                        tint: color("#9B7BFF"),
                        chapters: chapters.clone(),
                    });
                    data.set_builder_subjects(model(rows.clone()));
                    data.set_builder_subject_names(model(rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>()));
                    data.set_builder_new_subject("".into());
                    data.set_builder_new_chapters("".into());
                    data.set_builder_error("".into());
                    let _ = &mut st;
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_builder_remove_subject(move |index| {
            if let Some(ui) = weak.upgrade() {
                let data = ui.global::<crate::IoData>();
                let mut rows: Vec<crate::BuilderSubjectRow> = data.get_builder_subjects().iter().collect();
                if index >= 0 && (index as usize) < rows.len() { rows.remove(index as usize); }
                data.set_builder_subjects(model(rows.clone()));
                data.set_builder_subject_names(model(rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>()));
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_builder_add_exam(move |label, subject, date, time| {
            if let Some(ui) = weak.upgrade() {
                let data = ui.global::<crate::IoData>();
                if label.trim().is_empty() { data.set_builder_error("Exam name is required".into()); return; }
                let subject_name = data.get_builder_subject_names().row_data(subject.max(0) as usize)
                    .map(|s| s.to_string()).unwrap_or_default();
                if subject_name.is_empty() { data.set_builder_error("Add a subject before adding exams".into()); return; }
                if time::parse_date(&date).is_err() || time::parse_time(&time).is_err() {
                    data.set_builder_error("Exam date/time is invalid".into());
                    return;
                }
                let mut rows: Vec<crate::BuilderExamRow> = data.get_builder_exams().iter().collect();
                rows.push(crate::BuilderExamRow {
                    index: rows.len() as i32,
                    label: label.trim().into(),
                    subject: subject_name.into(),
                    date: date.clone(),
                    time: time.clone(),
                });
                data.set_builder_exams(model(rows));
                data.set_builder_exam_label("".into());
                data.set_builder_error("".into());
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_builder_remove_exam(move |index| {
            if let Some(ui) = weak.upgrade() {
                let data = ui.global::<crate::IoData>();
                let mut rows: Vec<crate::BuilderExamRow> = data.get_builder_exams().iter().collect();
                if index >= 0 && (index as usize) < rows.len() { rows.remove(index as usize); }
                data.set_builder_exams(model(rows));
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_builder_build(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let file = builder_file(&ui, &st)?;
                        let text = crate::import_export::format::to_pretty_json(&file)?;
                        let data = ui.global::<crate::IoData>();
                        data.set_editor_text(text.into());
                        data.set_builder_error("".into());
                        data.set_tab(1);
                        set_io_feedback(&ui, 1, "JSON built · validate and preview before importing");
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::IoData>().on_open_json_help(move || {
            if let Some(ui) = weak.upgrade() {
                set_io_feedback(&ui, 0, "Lockin Plan Format v1: see PLAN_FORMAT.md and schemas/lockin-plan.schema.json in the project.");
            }
        });
    }


    // ── Settings ─────────────────────────────────────────────────
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SettingsData>().on_save(move |display_name, accent_index, theme_index, timezone, goal_minutes, sleep_target, sound, notifications, auto_revisions, import_mode, intervals_text| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let accents = ["#9B7BFF", "#69D6A6", "#EABF73", "#7FB2E6", "#E67F90", "#C6A6FF"];
                        let mut settings = st.settings.clone();
                        settings.display_name = display_name.to_string();
                        settings.accent = accents[accent_index.max(0) as usize % accents.len()].to_string();
                        settings.theme = if theme_index == 1 { "slate".into() } else { "midnight".into() };
                        settings.timezone = timezone.trim().to_string();
                        if settings.timezone.parse::<chrono_tz::Tz>().is_err() { return Err(AppError::Validation("Default timezone is invalid.".into())); }
                        settings.daily_goal_minutes = goal_minutes.trim().parse().map_err(|_| AppError::Validation("Daily goal must be a number.".into()))?;
                        settings.sleep_target_hours = sleep_target.trim().parse().map_err(|_| AppError::Validation("Sleep target must be a number.".into()))?;
                        settings.sound_enabled = sound;
                        settings.notifications_enabled = notifications;
                        settings.auto_generate_revisions = auto_revisions;
                        settings.default_import_mode = if import_mode == 1 { "replace".into() } else { "merge".into() };
                        let intervals: Vec<i64> = intervals_text.split(',').filter_map(|s| s.trim().parse().ok()).filter(|n| *n > 0).collect();
                        if intervals.is_empty() { return Err(AppError::Validation("Revision intervals must contain positive numbers.".into())); }
                        settings.revision_intervals = intervals;
                        st.settings = settings.clone();
                        repo::save_settings(&st.db.conn, &settings)?;
                        apply_theme(&ui, &settings);
                        refresh_all(&ui, &mut st);
                        show_toast(&ui, &st, "Settings saved", 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SettingsData>().on_backup(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(st) = state.try_borrow() {
                    match crate::import_export::backup::export(&st.db.conn).and_then(|b| crate::import_export::backup::to_json(&b)) {
                        Ok(text) => {
                            let path = std::path::PathBuf::from(format!("{}/lockin-backup.json", std::env::var("HOME").unwrap_or_default()));
                            match std::fs::write(&path, text) { Ok(()) => show_toast(&ui, &st, format!("Backup written to {}", path.display()), 1), Err(e) => show_toast(&ui, &st, format!("Backup failed: {e}"), 2) }
                        }
                        Err(e) => show_toast(&ui, &st, e.to_string(), 2),
                    }
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SettingsData>().on_restore(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let Some(path) = rfd::FileDialog::new().add_filter("Lockin backup", &["json"]).pick_file() else { return Ok(()); };
                        let raw = std::fs::read_to_string(&path)?;
                        let backup = crate::import_export::backup::parse(&raw)?;
                        crate::import_export::backup::restore(&mut st.db.conn, &backup)?;
                        st.settings = repo::load_settings(&st.db.conn)?;
                        st.plan_id = services::plan::current_plan_id(&st.db.conn, &st.settings)?;
                        st.selected_date = time::today_in(&plan_bundle(&st)?.plan.timezone);
                        apply_theme(&ui, &st.settings);
                        refresh_all(&ui, &mut st);
                        show_toast(&ui, &st, "Backup restored", 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SettingsData>().on_open_wizard(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    st.wizard = crate::services::wizard::WizardDraft::blank();
                    publish_wizard(&ui, &st);
                    ui.global::<crate::WizardData>().set_open(true);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SettingsData>().on_switch_plan(move |plan_id| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        st.settings.last_plan_id = Some(plan_id.to_string());
                        st.plan_id = plan_id.to_string();
                        repo::save_settings(&st.db.conn, &st.settings)?;
                        st.selected_date = time::today_in(&plan_bundle(&st)?.plan.timezone);
                        refresh_all(&ui, &mut st);
                        show_toast(&ui, &st, "Plan switched", 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SettingsData>().on_archive_plan(move |plan_id, archived| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let mut plan = repo::get_plan(&st.db.conn, &plan_id)?;
                        plan.archived = archived;
                        repo::update_plan(&st.db.conn, &plan)?;
                        refresh_settings(&ui, &mut st);
                        show_toast(&ui, &st, if archived { "Plan archived" } else { "Plan unarchived" }, 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SettingsData>().on_delete_plan(move |plan_id| {
            let plan_id = plan_id.to_string();
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let result = (|| -> AppResult<()> {
                        let history: i64 = st.db.conn.query_row(
                            "SELECT (SELECT COUNT(*) FROM task_occurrences WHERE plan_id=?1 AND status='completed') + (SELECT COUNT(*) FROM focus_sessions WHERE plan_id=?1 AND status='completed') + (SELECT COUNT(*) FROM sleep_entries WHERE plan_id=?1) + (SELECT COUNT(*) FROM exam_mocks WHERE plan_id=?1) + (SELECT COUNT(*) FROM revision_history WHERE plan_id=?1)",
                            rusqlite::params![plan_id], |r| r.get(0)).unwrap_or(0);
                        if history > 0 {
                            return Err(AppError::Validation(format!("Plan has {history} completed records. Delete is refused to protect history.")));
                        }
                        repo::delete_plan(&st.db.conn, &plan_id)?;
                        if st.plan_id == plan_id {
                            st.settings.last_plan_id = repo::list_plans(&st.db.conn, false).ok().and_then(|p| p.first().map(|x| x.id.clone()));
                            st.plan_id = st.settings.last_plan_id.clone().unwrap_or_default();
                            repo::save_settings(&st.db.conn, &st.settings)?;
                            st.selected_date = time::today_in(&plan_bundle(&st)?.plan.timezone);
                        }
                        refresh_all(&ui, &mut st);
                        show_toast(&ui, &st, "Plan deleted", 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::SettingsData>().on_open_reset(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::SettingsData>().set_reset_open(true);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.global::<crate::SettingsData>().on_close_reset(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::SettingsData>().set_reset_open(false);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SettingsData>().on_confirm_reset(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let data = ui.global::<crate::SettingsData>();
                    let result = (|| -> AppResult<()> {
                        if data.get_reset_text().trim() != "RESET" { return Err(AppError::Validation("Type RESET to confirm.".into())); }
                        let conn = &mut st.db.conn;
                        for table in ["exam_mistakes","exam_mocks","revision_history","sleep_entries","focus_sessions","task_occurrences","chapter_checklists","chapter_notes","chapter_revisions","chapters","subjects","exams","schedule_templates","plans","settings"] {
                            conn.execute(&format!("DELETE FROM {table}"), [])?;
                        }
                        crate::db::seed::seed_if_empty(conn)?;
                        drop(conn);
                        st.settings = repo::load_settings(&st.db.conn)?;
                        st.plan_id = services::plan::current_plan_id(&st.db.conn, &st.settings)?;
                        st.selected_date = time::today_in(&plan_bundle(&st)?.plan.timezone);
                        st.import_preview = None;
                        st.backup_pending = None;
                        st.wizard = crate::services::wizard::WizardDraft::blank();
                        apply_theme(&ui, &st.settings);
                        refresh_all(&ui, &mut st);
                        data.set_reset_open(false);
                        data.set_reset_text("".into());
                        show_toast(&ui, &st, "All data reset", 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::SettingsData>().on_refresh_plans(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    let _ = refresh_settings(&ui, &mut st);
                }
            }
        });
    }


    // ── Wizard ───────────────────────────────────────────────────
    fn sync_wizard_basics(ui: &AppWindow, st: &mut AppState) {
        let data = ui.global::<crate::WizardData>();
        st.wizard.name = data.get_name().to_string();
        st.wizard.start_date = data.get_start().to_string();
        st.wizard.end_date = data.get_end().to_string();
        st.wizard.timezone = data.get_timezone().to_string();
        st.wizard.academic_year = data.get_year().to_string();

    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_next(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    sync_wizard_basics(&ui, &mut st);
                    let _errors = st.wizard.validate();
                    publish_wizard(&ui, &st);
                    let cur = ui.global::<crate::WizardData>().get_step();
                    if cur < 4 { ui.global::<crate::WizardData>().set_step(cur + 1); publish_wizard(&ui, &st); }
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_back(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    sync_wizard_basics(&ui, &mut st);
                    let cur = ui.global::<crate::WizardData>().get_step();
                    ui.global::<crate::WizardData>().set_step((cur - 1).max(0));
                    publish_wizard(&ui, &st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_close_wizard(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::WizardData>().set_open(false);
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_add_subject(move |name| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    sync_wizard_basics(&ui, &mut st);
                    if name.trim().is_empty() { ui.global::<crate::WizardData>().set_subject_error("Subject name is required".into()); publish_wizard(&ui, &st); return; }
                    let color = ["#9B7BFF","#69D6A6","#EABF73","#7FB2E6","#E67F90","#C6A6FF"][st.wizard.subjects.len() % 6].to_string();
                    st.wizard.subjects.push(crate::services::wizard::WizardSubject {
                        id: uuid::Uuid::new_v4().to_string(), name: name.trim().to_string(), color,
                        chapters: Vec::new(),
                    });
                    ui.global::<crate::WizardData>().set_subject_error("".into());
                    publish_wizard(&ui, &st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_remove_subject(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    sync_wizard_basics(&ui, &mut st);
                    if index >= 0 && (index as usize) < st.wizard.subjects.len() { st.wizard.subjects.remove(index as usize); }
                    publish_wizard(&ui, &st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_select_subject(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    ui.global::<crate::WizardData>().set_subject_index(index.max(0));
                    publish_wizard(&ui, &st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_add_exam(move |label, subject_index, date, time, session| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    sync_wizard_basics(&ui, &mut st);
                    if label.trim().is_empty() { ui.global::<crate::WizardData>().set_exam_error("Exam name is required".into()); publish_wizard(&ui, &st); return; }
                    let subject_id = st.wizard.subjects.get(subject_index as usize).map(|s| s.id.clone()).unwrap_or_default();
                    if subject_id.is_empty() { ui.global::<crate::WizardData>().set_exam_error("Add a subject first".into()); publish_wizard(&ui, &st); return; }
                    if time::parse_date(&date).is_err() || time::parse_time(&time).is_err() { ui.global::<crate::WizardData>().set_exam_error("Exam date/time is invalid".into()); publish_wizard(&ui, &st); return; }
                    st.wizard.exams.push(crate::services::wizard::WizardExam {
                        id: uuid::Uuid::new_v4().to_string(), label: label.trim().to_string(),
                        subject_id, date: date.to_string(), start_time: time.to_string(), session: session.to_string(),
                    });
                    ui.global::<crate::WizardData>().set_subject_error("".into());
                    publish_wizard(&ui, &st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_remove_exam(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    sync_wizard_basics(&ui, &mut st);
                    if index >= 0 && (index as usize) < st.wizard.exams.len() { st.wizard.exams.remove(index as usize); }
                    publish_wizard(&ui, &st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_add_slot(move |title, start, end, kind, subject_index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    sync_wizard_basics(&ui, &mut st);
                    if time::parse_time(&start).is_err() || time::parse_time(&end).is_err() { ui.global::<crate::WizardData>().set_schedule_error("Slot times are invalid".into()); publish_wizard(&ui, &st); return; }
                    let slot_kind = match kind { 1 => crate::models::SlotKind::Break, 2 => crate::models::SlotKind::Meal, _ => crate::models::SlotKind::Study };
                    let subject_id = if subject_index < 0 || subject_index as usize >= st.wizard.subjects.len() { String::new() } else { st.wizard.subjects[subject_index as usize].id.clone() };
                    let slot_title = if title.trim().is_empty() { slot_kind.label().to_string() } else { title.trim().to_string() };
                    let slot = crate::services::wizard::WizardSlot {
                        id: uuid::Uuid::new_v4().to_string(),
                        kind: slot_kind,
                        subject_id,
                        start_time: start.to_string(),
                        end_time: end.to_string(),
                        title: slot_title,
                    };
                    let ti = ui.global::<crate::WizardData>().get_template_index().max(0) as usize;
                    let len = st.wizard.templates.len().max(1);
                    let template = &mut st.wizard.templates[ti % len];
                    template.slots.push(slot);
                    ui.global::<crate::WizardData>().set_subject_error("".into());
                    publish_wizard(&ui, &st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_remove_slot(move |slot_index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    sync_wizard_basics(&ui, &mut st);
                    let ti = ui.global::<crate::WizardData>().get_template_index().max(0) as usize;
                    if let Some(t) = st.wizard.templates.get_mut(ti) {
                        if slot_index >= 0 && (slot_index as usize) < t.slots.len() { t.slots.remove(slot_index as usize); }
                    }
                    publish_wizard(&ui, &st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_create(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    sync_wizard_basics(&ui, &mut st);
                    let errors = st.wizard.validate();
                    publish_wizard(&ui, &st);
                    if !errors.is_empty() { return; }
                    let result = (|| -> AppResult<()> {
                        let draft = st.wizard.clone();
                        let new_id = draft.commit(&mut st.db.conn)?;
                        st.plan_id = new_id.clone();
                        st.settings.last_plan_id = Some(new_id);
                        repo::save_settings(&st.db.conn, &st.settings)?;
                        st.selected_date = time::today_in(&plan_bundle(&st)?.plan.timezone);
                        ui.global::<crate::WizardData>().set_open(false);
                        refresh_all(&ui, &mut st);
                        show_toast(&ui, &st, "Plan created", 1);
                        Ok(())
                    })();
                    report(&ui, &st, result);
                }
            }
        });
    }


    // Wizard extras
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_add_chapter(move |title| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    sync_wizard_basics(&ui, &mut st);
                    let idx = ui.global::<crate::WizardData>().get_subject_index().max(0) as usize;
                    if let Some(subject) = st.wizard.subjects.get_mut(idx) {
                        if !title.trim().is_empty() {
                            subject.chapters.push(crate::services::wizard::WizardChapter { id: uuid::Uuid::new_v4().to_string(), title: title.trim().to_string() });
                        }
                    }
                    publish_wizard(&ui, &st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_remove_chapter(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    sync_wizard_basics(&ui, &mut st);
                    let sidx = ui.global::<crate::WizardData>().get_subject_index().max(0) as usize;
                    if let Some(subject) = st.wizard.subjects.get_mut(sidx) {
                        if index >= 0 && (index as usize) < subject.chapters.len() { subject.chapters.remove(index as usize); }
                    }
                    publish_wizard(&ui, &st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_rename_subject(move |name| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    sync_wizard_basics(&ui, &mut st);
                    let idx = ui.global::<crate::WizardData>().get_subject_index().max(0) as usize;
                    if let Some(subject) = st.wizard.subjects.get_mut(idx) {
                        if !name.trim().is_empty() { subject.name = name.trim().to_string(); }
                    }
                    publish_wizard(&ui, &st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_edit_slot(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    sync_wizard_basics(&ui, &mut st);
                    let ti = ui.global::<crate::WizardData>().get_template_index().max(0) as usize;
                    if let Some(slot) = st.wizard.templates.get(ti).and_then(|t| t.slots.get(index as usize)) {
                        let data = ui.global::<crate::WizardData>();
                        data.set_slot_title(slot.title.clone().into());
                        data.set_slot_start(slot.start_time.clone().into());
                        data.set_slot_end(slot.end_time.clone().into());
                        data.set_slot_kind(match slot.kind { crate::models::SlotKind::Break => 1, crate::models::SlotKind::Meal => 2, _ => 0 });
                        let subject_idx = st.wizard.subjects.iter().position(|s| s.id == slot.subject_id).map(|i| i as i32).unwrap_or(-1);
                        data.set_slot_subject(subject_idx);
                        data.set_editing_slot(slot.id.clone().into());
                    }
                    publish_wizard(&ui, &st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_save_slot(move |title, start, end, kind, subject_index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    sync_wizard_basics(&ui, &mut st);
                    if time::parse_time(&start).is_err() || time::parse_time(&end).is_err() { ui.global::<crate::WizardData>().set_schedule_error("Slot times are invalid".into()); publish_wizard(&ui, &st); return; }
                    let ti = ui.global::<crate::WizardData>().get_template_index().max(0) as usize;
                    let slot_kind = match kind { 1 => crate::models::SlotKind::Break, 2 => crate::models::SlotKind::Meal, _ => crate::models::SlotKind::Study };
                    let subject_id = if subject_index < 0 || subject_index as usize >= st.wizard.subjects.len() { String::new() } else { st.wizard.subjects[subject_index as usize].id.clone() };
                    let slot_title = if title.trim().is_empty() { slot_kind.label().to_string() } else { title.trim().to_string() };
                    let editing = ui.global::<crate::WizardData>().get_editing_slot().to_string();
                    if let Some(template) = st.wizard.templates.get_mut(ti) {
                        if !editing.is_empty() {
                            if let Some(slot) = template.slots.iter_mut().find(|s| s.id == editing) {
                                slot.title = slot_title;
                                slot.start_time = start.to_string();
                                slot.end_time = end.to_string();
                                slot.kind = slot_kind;
                                slot.subject_id = subject_id;
                            }
                        } else {
                            template.slots.push(crate::services::wizard::WizardSlot {
                                id: uuid::Uuid::new_v4().to_string(),
                                kind: slot_kind,
                                subject_id,
                                start_time: start.to_string(),
                                end_time: end.to_string(),
                                title: slot_title,
                            });
                        }
                    }
                    ui.global::<crate::WizardData>().set_editing_slot("".into());
                    ui.global::<crate::WizardData>().set_slot_title("".into());
                    ui.global::<crate::WizardData>().set_subject_error("".into());
                    publish_wizard(&ui, &st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_set_template(move |index| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    publish_wizard(&ui, &st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_set_generate_tasks(move |value| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    st.wizard.generate_tasks = value;
                    publish_wizard(&ui, &st);
                }
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_cancel(move || {
            if let Some(ui) = weak.upgrade() {
                ui.global::<crate::WizardData>().set_open(false);
                let _ = state;
            }
        });
    }
    {
        let state = state.clone();
        let weak = ui.as_weak();
        ui.global::<crate::WizardData>().on_refresh_review(move || {
            if let Some(ui) = weak.upgrade() {
                if let Ok(mut st) = state.try_borrow_mut() {
                    sync_wizard_basics(&ui, &mut st);
                    publish_wizard(&ui, &st);
                }
            }
        });
    }

    // Theme + initial paint.
    if let Ok(mut st) = state.try_borrow_mut() {
        apply_theme(&ui, &st.settings);
        let shell = ui.global::<crate::Shell>();
        shell.set_current_page(0);
        shell.set_page_title(PAGE_TITLES[0].into());
        if let Ok(bundle) = plan_bundle(&st) {
            shell.set_page_context(page_context(0, &bundle.plan.name, &bundle.plan.timezone).into());
        }
        // Recover a crashed/interrupted focus session if one exists.
        if let Ok(Some(row)) = repo::load_timer(&st.db.conn) {
            if !row.session_id.is_empty() && row.state != "idle" {
                let preset = match row.mode.as_str() {
                    "short" => crate::timer::Preset::Short,
                    "extended" => crate::timer::Preset::Extended,
                    "custom" => crate::timer::Preset::Custom,
                    "stopwatch" => crate::timer::Preset::Stopwatch,
                    _ => crate::timer::Preset::Deep,
                };
                let phase = crate::timer::TimerPhase::from_str(&row.phase);
                let tstate = crate::timer::TimerState::from_str(&row.state);
                let started = chrono::DateTime::parse_from_rfc3339(&row.started_at)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now());
                let tz = st.settings.timezone.parse().unwrap_or(chrono_tz::Asia::Kolkata);
                let recovered = crate::timer::FocusTimer::recovered(
                    preset,
                    &row.mode,
                    std::time::Duration::from_secs(row.planned_focus_secs as u64),
                    std::time::Duration::from_secs(row.planned_break_secs as u64),
                    std::time::Duration::from_millis(row.accumulated_ms as u64),
                    std::time::Duration::from_millis(row.break_accumulated_ms as u64),
                    row.subject_id.clone(),
                    row.chapter_id.clone(),
                    row.task_id.clone(),
                    row.session_id.clone(),
                    started,
                    phase,
                    tstate,
                    row.interruptions,
                    tz,
                );
                st.timer = recovered;
                st.recovered = tstate != crate::timer::TimerState::Idle;
                if st.recovered {
                    let data = ui.global::<crate::FocusData>();
                    data.set_recovery_open(true);
                    data.set_recovery_text(format!("A {} session from {} is still open. Resume or discard?", row.mode, row.started_at).into());
                }
            }
        }
        refresh_all(&ui, &mut st);
    }

    // Periodic focus tick + checkpoint.
    let tick_state = state.clone();
    let tick_weak = ui.as_weak();
    let mut ticker = slint::Timer::default();
    ticker.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(1), move || {
        if let Some(ui) = tick_weak.upgrade() {
            if let Ok(mut st) = tick_state.try_borrow_mut() {
                if st.timer.state == crate::timer::TimerState::Running {
                    let _ = persist_timer(&st);
                }
                let page = ui.global::<crate::Shell>().get_current_page();
                if page == 3 {
                    let _ = refresh_page(&ui, &mut st, 3);
                }
            }
        }
    });

    ui.run().map_err(|e| AppError::Other(e.to_string()))?;
    state.borrow().db.checkpoint();
    Ok(())
}
