//! Typed domain models. These are the single source of truth for every layer;
//! the database and the JSON plan format both convert to and from them.

pub mod time;

use chrono::{DateTime, NaiveDate, NaiveTime};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

pub use time::*;

macro_rules! text_enum {
    ($name:ident, $def:expr, { $($variant:ident => $txt:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $txt),+ }
            }
            pub fn from_str(s: &str) -> Option<Self> {
                match s.trim().to_ascii_lowercase().as_str() {
                    $($txt => Some(Self::$variant),)+
                    _ => None,
                }
            }
            pub fn parse_or(s: &str, fallback: Self) -> Self {
                Self::from_str(s).unwrap_or(fallback)
            }
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
        }

        impl Default for $name {
            fn default() -> Self { $def }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

text_enum!(ChapterStatus, ChapterStatus::NotStarted, {
    NotStarted => "not_started",
    Learning => "learning",
    NeedsPractice => "needs_practice",
    NeedsRevision => "needs_revision",
    ExamReady => "exam_ready",
});

impl ChapterStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::NotStarted => "Not started",
            Self::Learning => "Learning",
            Self::NeedsPractice => "Needs practice",
            Self::NeedsRevision => "Needs revision",
            Self::ExamReady => "Exam ready",
        }
    }
    pub fn is_complete(self) -> bool {
        matches!(self, Self::ExamReady)
    }
    pub fn index(self) -> i32 {
        match self {
            Self::NotStarted => 0,
            Self::Learning => 1,
            Self::NeedsPractice => 2,
            Self::NeedsRevision => 3,
            Self::ExamReady => 4,
        }
    }
}

text_enum!(TaskType, TaskType::Study, {
    Study => "study",
    Revision => "revision",
    MockTest => "mock_test",
    Backlog => "backlog",
});

impl TaskType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Study => "Study",
            Self::Revision => "Revision",
            Self::MockTest => "Mock test",
            Self::Backlog => "Backlog",
        }
    }
}

text_enum!(TaskStatus, TaskStatus::Planned, {
    Planned => "planned",
    InProgress => "in_progress",
    Completed => "completed",
    PartiallyCompleted => "partially_completed",
    Skipped => "skipped",
});

impl TaskStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Planned => "Planned",
            Self::InProgress => "In progress",
            Self::Completed => "Completed",
            Self::PartiallyCompleted => "Partially done",
            Self::Skipped => "Skipped",
        }
    }
}

text_enum!(SlotKind, SlotKind::Study, {
    Study => "study",
    Break => "break",
    Meal => "meal",
});

impl SlotKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Study => "Study",
            Self::Break => "Break",
            Self::Meal => "Meal",
        }
    }
}

text_enum!(DayType, DayType::Weekday, {
    Weekday => "weekday",
    Saturday => "saturday",
    Sunday => "sunday",
    Holiday => "holiday",
    Custom => "custom",
});

impl DayType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Weekday => "School day",
            Self::Saturday => "Saturday",
            Self::Sunday => "Sunday",
            Self::Holiday => "Holiday",
            Self::Custom => "Custom day",
        }
    }
}

text_enum!(RevisionResult, RevisionResult::Good, {
    Easy => "easy",
    Good => "good",
    Difficult => "difficult",
    Failed => "failed",
});

impl RevisionResult {
    pub fn label(self) -> &'static str {
        match self {
            Self::Easy => "Easy",
            Self::Good => "Good",
            Self::Difficult => "Difficult",
            Self::Failed => "Failed",
        }
    }
}

text_enum!(RevisionState, RevisionState::Pending, {
    Pending => "pending",
    Completed => "completed",
    Snoozed => "snoozed",
});

text_enum!(SleepKind, SleepKind::Night, {
    Night => "night",
    Nap => "nap",
    SchoolNap => "school_nap",
});

impl SleepKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Night => "Night sleep",
            Self::Nap => "Nap",
            Self::SchoolNap => "School nap",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub name: String,
    pub academic_year: String,
    pub timezone: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub archived: bool,
    pub created_at: String,
}

impl Plan {
    pub fn tz(&self) -> Tz {
        parse_tz(&self.timezone)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subject {
    pub id: String,
    pub plan_id: String,
    pub name: String,
    pub color: String,
    pub sort_order: i64,
    /// True for subjects whose syllabus has not been entered yet (e.g. Hindi).
    pub portions_pending: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    pub id: String,
    pub subject_id: String,
    pub title: String,
    pub sort_order: i64,
    pub status: ChapterStatus,
    pub concept_done: bool,
    pub textbook_done: bool,
    pub exercises_done: bool,
    pub selftest_done: bool,
    pub confidence: Option<i64>,
    pub questions_attempted: i64,
    pub correct_answers: i64,
    pub last_studied: Option<NaiveDate>,
    pub next_revision: Option<NaiveDate>,
    pub study_seconds: i64,
    pub notes: String,
}

impl Chapter {
    pub fn checklist_done(&self) -> i64 {
        [
            self.concept_done,
            self.textbook_done,
            self.exercises_done,
            self.selftest_done,
        ]
        .iter()
        .filter(|v| **v)
        .count() as i64
    }

    pub fn accuracy(&self) -> Option<f64> {
        if self.questions_attempted <= 0 {
            None
        } else {
            Some(self.correct_answers as f64 / self.questions_attempted as f64)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exam {
    pub id: String,
    pub plan_id: String,
    pub subject_id: String,
    pub label: String,
    pub date: NaiveDate,
    pub start_time: NaiveTime,
    pub session: String,
    pub duration_minutes: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateSlot {
    pub id: String,
    pub template_id: String,
    pub start_time: NaiveTime,
    pub end_time: NaiveTime,
    pub kind: SlotKind,
    pub title: String,
    pub subject_id: Option<String>,
    pub chapter_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduleTemplate {
    pub id: String,
    pub plan_id: String,
    pub name: String,
    pub day_types: Vec<DayType>,
    pub slots: Vec<TemplateSlot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub plan_id: String,
    pub date: NaiveDate,
    pub title: String,
    pub task_type: TaskType,
    pub subject_id: Option<String>,
    pub chapter_id: Option<String>,
    pub start_time: NaiveTime,
    pub end_time: NaiveTime,
    pub estimated_minutes: i64,
    /// 1 = low, 2 = normal, 3 = high.
    pub priority: i64,
    pub status: TaskStatus,
    pub completed_at: Option<String>,
    pub notes: String,
    pub source_template_id: Option<String>,
    pub created_at: String,
}

impl Task {
    pub fn priority_label(&self) -> &'static str {
        match self.priority {
            3 => "High",
            1 => "Low",
            _ => "Normal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DayOverride {
    pub plan_id: String,
    pub date: NaiveDate,
    pub day_type: DayType,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FocusSession {
    pub id: String,
    pub plan_id: String,
    pub task_id: Option<String>,
    pub subject_id: Option<String>,
    pub chapter_id: Option<String>,
    pub mode: String,
    pub started_at: String,
    pub on_date: NaiveDate,
    pub ended_at: Option<String>,
    pub focused_seconds: i64,
    pub break_seconds: i64,
    pub interruptions: i64,
    pub status: String,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Revision {
    pub id: String,
    pub plan_id: String,
    pub chapter_id: String,
    pub due_date: NaiveDate,
    /// The interval this review was created with (1, 3, 7 … days).
    pub interval_days: i64,
    /// Ease factor used to adapt future intervals after a result.
    pub ease: f64,
    pub state: RevisionState,
    pub result: Option<RevisionResult>,
    pub completed_at: Option<String>,
    pub snoozed_until: Option<NaiveDate>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MockTest {
    pub id: String,
    pub plan_id: String,
    pub subject_id: String,
    pub name: String,
    pub date: NaiveDate,
    pub score: f64,
    pub max_marks: f64,
    pub duration_minutes: i64,
    pub weak_topics: String,
    pub notes: String,
    pub created_at: String,
}

impl MockTest {
    pub fn percentage(&self) -> f64 {
        if self.max_marks <= 0.0 {
            0.0
        } else {
            (self.score / self.max_marks * 100.0).clamp(0.0, 100.0)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mistake {
    pub id: String,
    pub plan_id: String,
    pub mock_test_id: Option<String>,
    pub subject_id: String,
    pub chapter_id: Option<String>,
    pub question: String,
    pub explanation: String,
    pub resolved: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SleepEntry {
    pub id: String,
    pub plan_id: String,
    pub kind: SleepKind,
    pub start_at: String,
    pub end_at: String,
    pub energy: Option<i64>,
    pub notes: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub display_name: String,
    pub accent: String,
    pub theme: String,
    pub timezone: String,
    pub daily_goal_minutes: i64,
    pub sleep_target_hours: f64,
    pub sound_enabled: bool,
    pub notifications_enabled: bool,
    pub default_import_mode: String,
    pub revision_intervals: Vec<i64>,
    pub auto_generate_revisions: bool,
    pub last_plan_id: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            display_name: "Alex".into(),
            accent: "#9B7BFF".into(),
            theme: "dark".into(),
            timezone: DEFAULT_TZ.into(),
            daily_goal_minutes: 300,
            sleep_target_hours: 8.5,
            sound_enabled: true,
            notifications_enabled: true,
            default_import_mode: "merge".into(),
            revision_intervals: vec![1, 3, 7],
            auto_generate_revisions: true,
            last_plan_id: None,
        }
    }
}

/// A single entry shown in a timetable day: either a task or a fixed
/// break/meal period from the active template.
#[derive(Debug, Clone)]
pub struct DayEntry {
    pub task: Option<Task>,
    pub slot: Option<TemplateSlot>,
    pub start_time: NaiveTime,
    pub end_time: NaiveTime,
}

#[derive(Debug, Clone)]
pub struct PlanBundle {
    pub plan: Plan,
    pub subjects: Vec<(Subject, Vec<Chapter>)>,
    pub exams: Vec<Exam>,
    pub templates: Vec<ScheduleTemplate>,
}

#[derive(Debug, Clone, Default)]
pub struct DashboardStats {
    pub planned_minutes_today: i64,
    pub focused_minutes_today: i64,
    pub completed_chapters: i64,
    pub total_chapters: i64,
    pub revisions_due: i64,
    pub revisions_overdue: i64,
    pub streak_days: i64,
    pub overdue_tasks: i64,
}

#[derive(Debug, Clone)]
pub struct Recommendation {
    pub kind: String,
    pub title: String,
    pub subject: String,
    pub reason: String,
    pub task_id: Option<String>,
    pub minutes: i64,
}

pub type UtcStamp = DateTime<chrono::Utc>;
