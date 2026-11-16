//! Monotonic focus timer.
//!
//! Elapsed time is always derived from a monotonic `Instant` plus a persisted
//! checkpoint. Paused time is never counted. A one-second UI tick only paints
//! the display; it is never the source of truth.

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerPhase {
    Focus,
    Break,
}

impl TimerPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::Break => "break",
        }
    }
    pub fn from_str(s: &str) -> Self {
        if s == "break" {
            Self::Break
        } else {
            Self::Focus
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerState {
    Idle,
    Running,
    Paused,
}

impl TimerState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Paused => "paused",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "paused" => Self::Paused,
            _ => Self::Idle,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Deep,
    Short,
    Extended,
    Custom,
    Stopwatch,
}

impl Preset {
    pub fn from_index(i: i32) -> Self {
        match i {
            1 => Self::Short,
            2 => Self::Extended,
            3 => Self::Custom,
            4 => Self::Stopwatch,
            _ => Self::Deep,
        }
    }
    pub fn focus_minutes(self) -> Option<i64> {
        match self {
            Self::Deep => Some(45),
            Self::Short => Some(25),
            Self::Extended => Some(50),
            Self::Custom => None,
            Self::Stopwatch => None,
        }
    }
    pub fn break_minutes(self) -> Option<i64> {
        match self {
            Self::Deep => Some(10),
            Self::Short => Some(5),
            Self::Extended => Some(10),
            Self::Custom => None,
            Self::Stopwatch => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Deep => "Deep focus · 45/10",
            Self::Short => "Short focus · 25/5",
            Self::Extended => "Extended · 50/10",
            Self::Custom => "Custom",
            Self::Stopwatch => "Stopwatch",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Deep => "deep",
            Self::Short => "short",
            Self::Extended => "extended",
            Self::Custom => "custom",
            Self::Stopwatch => "stopwatch",
        }
    }
}

/// Pure elapsed calculation, kept separate so pause/resume arithmetic is
/// testable without sleeping.
pub fn elapsed_ms(accumulated_ms: i64, running_since_ms: Option<i64>, now_ms: i64) -> i64 {
    match running_since_ms {
        Some(since) => accumulated_ms + (now_ms - since).max(0),
        None => accumulated_ms.max(0),
    }
}

#[derive(Debug, Clone)]
pub struct FocusTimer {
    pub state: TimerState,
    pub phase: TimerPhase,
    pub preset: Preset,
    pub mode: String,
    pub planned_focus: Duration,
    pub planned_break: Duration,
    /// Focused time is split by phase, so break time never pollutes study time.
    accumulated_phase: Duration,
    finished_focus: Duration,
    finished_break: Duration,
    run_since: Option<Instant>,
    pub started_at: DateTime<Utc>,
    pub started_at_local: DateTime<Tz>,
    pub on_date: NaiveDate,
    pub session_id: String,
    pub task_id: Option<String>,
    pub subject_id: Option<String>,
    pub chapter_id: Option<String>,
    pub interruptions: i64,
    pub last_checkpoint: Option<Instant>,
}

impl Default for FocusTimer {
    fn default() -> Self {
        Self {
            state: TimerState::Idle,
            phase: TimerPhase::Focus,
            preset: Preset::Deep,
            mode: "deep".into(),
            planned_focus: Duration::from_secs(45 * 60),
            planned_break: Duration::from_secs(10 * 60),
            accumulated_phase: Duration::ZERO,
            finished_focus: Duration::ZERO,
            finished_break: Duration::ZERO,
            run_since: None,
            started_at: Utc::now(),
            started_at_local: Utc::now().with_timezone(&chrono_tz::Asia::Kolkata),
            on_date: Utc::now().date_naive(),
            session_id: String::new(),
            task_id: None,
            subject_id: None,
            chapter_id: None,
            interruptions: 0,
            last_checkpoint: None,
        }
    }
}

impl FocusTimer {
    pub fn new(
        preset: Preset,
        mode: &str,
        focus: Duration,
        brk: Duration,
        subject_id: Option<String>,
        chapter_id: Option<String>,
        task_id: Option<String>,
        tz: Tz,
    ) -> Self {
        let now = Utc::now();
        Self {
            state: TimerState::Running,
            phase: TimerPhase::Focus,
            preset,
            mode: mode.to_string(),
            planned_focus: focus,
            planned_break: brk,
            accumulated_phase: Duration::ZERO,
            finished_focus: Duration::ZERO,
            finished_break: Duration::ZERO,
            run_since: Some(Instant::now()),
            started_at: now,
            started_at_local: now.with_timezone(&tz),
            on_date: now.with_timezone(&tz).date_naive(),
            session_id: uuid::Uuid::new_v4().to_string(),
            task_id,
            subject_id,
            chapter_id,
            interruptions: 0,
            last_checkpoint: Some(Instant::now()),
        }
    }

    /// Rebuild a timer from a persisted checkpoint after a restart/crash.
    pub fn recovered(
        preset: Preset,
        mode: &str,
        focus: Duration,
        brk: Duration,
        focused: Duration,
        break_time: Duration,
        subject_id: Option<String>,
        chapter_id: Option<String>,
        task_id: Option<String>,
        session_id: String,
        started_at: DateTime<Utc>,
        phase: TimerPhase,
        state: TimerState,
        interruptions: i64,
        tz: Tz,
    ) -> Self {
        let now = Utc::now();
        Self {
            state,
            phase,
            preset,
            mode: mode.to_string(),
            planned_focus: focus,
            planned_break: brk,
            accumulated_phase: if phase == TimerPhase::Focus { focused } else { break_time },
            finished_focus: if phase == TimerPhase::Focus { Duration::ZERO } else { focused },
            finished_break: if phase == TimerPhase::Break { Duration::ZERO } else { break_time },
            run_since: None,
            started_at,
            started_at_local: started_at.with_timezone(&tz),
            on_date: started_at.with_timezone(&tz).date_naive(),
            session_id,
            task_id,
            subject_id,
            chapter_id,
            interruptions,
            last_checkpoint: Some(Instant::now()),
        }
    }

    pub fn elapsed_phase(&self) -> Duration {
        self.accumulated_phase + self.run_since.map(|s| s.elapsed()).unwrap_or_default()
    }

    pub fn focus_elapsed(&self) -> Duration {
        if self.phase == TimerPhase::Focus {
            self.finished_focus + self.elapsed_phase()
        } else {
            self.finished_focus
        }
    }

    pub fn break_elapsed(&self) -> Duration {
        if self.phase == TimerPhase::Break {
            self.finished_break + self.elapsed_phase()
        } else {
            self.finished_break
        }
    }

    pub fn remaining(&self) -> Option<Duration> {
        if self.planned_focus.is_zero() || self.preset == Preset::Stopwatch {
            return None;
        }
        let planned = if self.phase == TimerPhase::Focus {
            self.planned_focus
        } else {
            self.planned_break
        };
        Some(planned.saturating_sub(self.elapsed_phase()))
    }

    pub fn phase_complete(&self) -> bool {
        self.remaining().map(|r| r.is_zero()).unwrap_or(false)
    }

    pub fn pause(&mut self) {
        if self.state != TimerState::Running {
            return;
        }
        self.accumulated_phase = self.elapsed_phase();
        self.run_since = None;
        self.state = TimerState::Paused;
    }

    pub fn resume(&mut self) {
        if self.state == TimerState::Paused {
            self.run_since = Some(Instant::now());
            self.state = TimerState::Running;
        }
    }

    pub fn interrupt(&mut self) {
        self.interruptions += 1;
        self.pause();
    }

    /// End the focus phase, banking its time and switching to break.
    pub fn enter_break(&mut self) {
        self.finished_focus += self.elapsed_phase();
        self.accumulated_phase = Duration::ZERO;
        self.phase = TimerPhase::Break;
        self.run_since = if self.state == TimerState::Running {
            Some(Instant::now())
        } else {
            None
        };
    }

    pub fn end_break(&mut self) {
        self.finished_break += self.elapsed_phase();
        self.accumulated_phase = Duration::ZERO;
        self.phase = TimerPhase::Focus;
        self.run_since = None;
        self.state = TimerState::Paused;
    }

    pub fn accumulated_phase_ms(&self) -> i64 {
        self.accumulated_phase.as_millis() as i64
    }

    pub fn focus_ms(&self) -> i64 {
        self.focus_elapsed().as_millis() as i64
    }

    pub fn break_ms(&self) -> i64 {
        self.break_elapsed().as_millis() as i64
    }

    pub fn display(&self) -> String {
        match self.remaining() {
            Some(rem) => format_clock(rem.as_secs() as i64),
            None => format_clock(self.elapsed_phase().as_secs() as i64),
        }
    }
}

pub fn format_clock(total_seconds: i64) -> String {
    let s = total_seconds.max(0);
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m:02}:{sec:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_excludes_paused_time() {
        assert_eq!(elapsed_ms(60_000, None, 999_999), 60_000);
        assert_eq!(elapsed_ms(60_000, Some(1_000), 5_000), 64_000);
        // Running-since after now (clock weirdness) never subtracts time.
        assert_eq!(elapsed_ms(60_000, Some(5_000), 1_000), 60_000);
    }

    #[test]
    fn focus_and_break_are_counted_separately() {
        let mut t = FocusTimer::new(
            Preset::Deep,
            "deep",
            Duration::from_secs(2700),
            Duration::from_secs(600),
            None,
            None,
            None,
            chrono_tz::Asia::Kolkata,
        );
        t.pause();
        t.finished_focus = Duration::from_secs(120);
        t.enter_break();
        t.accumulated_phase = Duration::from_secs(30);
        assert_eq!(t.focus_elapsed().as_secs(), 120);
        assert_eq!(t.break_elapsed().as_secs(), 30);
    }

    #[test]
    fn clock_formats() {
        assert_eq!(format_clock(0), "00:00");
        assert_eq!(format_clock(65), "01:05");
        assert_eq!(format_clock(3661), "1:01:01");
    }
}
