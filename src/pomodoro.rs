//! Port of `pomodoroStore.ts`: a focus/break timer that survives restarts.
//!
//! Only transitions (start, pause, resume, reset, finish) change state; the countdown is derived
//! from `ends_at` by whoever draws it. The session is persisted as `preferences.pomodoroSession`
//! (null while idle), with durations from `pomodoroWorkMinutes` / `pomodoroShortBreakMinutes` /
//! `pomodoroLongBreakMinutes`, the same keys the web build uses.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const CYCLES_PER_LONG_BREAK: u32 = 4;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    #[default]
    Idle,
    Work,
    ShortBreak,
    LongBreak,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    #[default]
    Idle,
    Running,
    Paused,
    Finished,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Pomodoro {
    pub phase: Phase,
    pub status: Status,
    /// Epoch ms the running phase ends at.
    pub ends_at: Option<u64>,
    /// Remaining time frozen while paused.
    pub remaining_ms_at_pause: Option<u64>,
    pub cycles_completed: u32,
    pub focus_todo_id: Option<String>,
}

/// Minutes per phase, read from preferences.
#[derive(Clone, Copy, Debug)]
pub struct Durations {
    pub work: u64,
    pub short_break: u64,
    pub long_break: u64,
}

impl Durations {
    pub fn from_prefs(extra: &Map<String, Value>) -> Self {
        let minutes = |key: &str, default: u64| {
            extra
                .get(key)
                .and_then(Value::as_u64)
                .filter(|m| *m > 0)
                .unwrap_or(default)
        };
        Self {
            work: minutes("pomodoroWorkMinutes", 25),
            short_break: minutes("pomodoroShortBreakMinutes", 5),
            long_break: minutes("pomodoroLongBreakMinutes", 15),
        }
    }

    pub fn minutes(&self, phase: Phase) -> u64 {
        match phase {
            Phase::Work | Phase::Idle => self.work,
            Phase::ShortBreak => self.short_break,
            Phase::LongBreak => self.long_break,
        }
    }
}

impl Pomodoro {
    /// The persisted session; a phase that ended while the app was closed shows as finished
    /// instead of resuming with negative time.
    pub fn hydrate(extra: &Map<String, Value>, now: u64) -> Self {
        let mut session: Pomodoro = extra
            .get("pomodoroSession")
            .filter(|v| !v.is_null())
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        if session.status == Status::Running && session.ends_at.is_none_or(|end| end <= now) {
            session.status = Status::Finished;
            session.ends_at = None;
        }
        session
    }

    pub fn persist(&self, extra: &mut Map<String, Value>) {
        let value = if self.status == Status::Idle {
            Value::Null
        } else {
            serde_json::to_value(self).unwrap_or(Value::Null)
        };
        extra.insert("pomodoroSession".into(), value);
    }

    /// Starts `phase`, or the one that comes next: work after idle or a break, a short break after
    /// work (a long one every fourth cycle).
    pub fn start(&mut self, phase: Option<Phase>, durations: Durations, now: u64) {
        let phase = phase.unwrap_or(match self.phase {
            Phase::Work
                if self.cycles_completed > 0
                    && self.cycles_completed % CYCLES_PER_LONG_BREAK == 0 =>
            {
                Phase::LongBreak
            }
            Phase::Work => Phase::ShortBreak,
            _ => Phase::Work,
        });
        self.phase = phase;
        self.status = Status::Running;
        self.ends_at = Some(now + durations.minutes(phase) * 60_000);
        self.remaining_ms_at_pause = None;
    }

    pub fn pause(&mut self, now: u64) {
        let (Status::Running, Some(end)) = (self.status, self.ends_at) else {
            return;
        };
        self.status = Status::Paused;
        self.remaining_ms_at_pause = Some(end.saturating_sub(now));
        self.ends_at = None;
    }

    pub fn resume(&mut self, now: u64) {
        let (Status::Paused, Some(remaining)) = (self.status, self.remaining_ms_at_pause) else {
            return;
        };
        self.status = Status::Running;
        self.ends_at = Some(now + remaining);
        self.remaining_ms_at_pause = None;
    }

    pub fn reset(&mut self) {
        *self = Pomodoro::default();
    }

    /// Ends the running phase once its time is up; returns the phase that just ended.
    pub fn tick(&mut self, now: u64) -> Option<Phase> {
        let (Status::Running, Some(end)) = (self.status, self.ends_at) else {
            return None;
        };
        if now < end {
            return None;
        }
        self.status = Status::Finished;
        self.ends_at = None;
        if self.phase == Phase::Work {
            self.cycles_completed += 1;
        }
        Some(self.phase)
    }

    pub fn remaining_ms(&self, now: u64) -> u64 {
        match self.status {
            Status::Paused => self.remaining_ms_at_pause.unwrap_or(0),
            Status::Running => self.ends_at.map_or(0, |end| end.saturating_sub(now)),
            _ => 0,
        }
    }

    /// 0..=1 of the current phase elapsed.
    pub fn progress(&self, durations: Durations, now: u64) -> f32 {
        if self.status == Status::Finished {
            return 1.0;
        }
        let total = (durations.minutes(self.phase).max(1) * 60_000) as f32;
        (1.0 - self.remaining_ms(now) as f32 / total).clamp(0.0, 1.0)
    }

    pub fn is_break(&self) -> bool {
        matches!(self.phase, Phase::ShortBreak | Phase::LongBreak)
    }
}

/// `m:ss`, rounding up so a phase never shows 0:00 while it still runs.
pub fn countdown(ms: u64) -> String {
    let seconds = ms.div_ceil(1000);
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: Durations = Durations {
        work: 25,
        short_break: 5,
        long_break: 15,
    };

    #[test]
    fn work_then_short_break_then_long_every_fourth() {
        let mut p = Pomodoro::default();
        let mut now = 0;
        for cycle in 1..=4 {
            p.start(None, D, now);
            assert_eq!(p.phase, Phase::Work);
            now += 25 * 60_000;
            assert_eq!(p.tick(now), Some(Phase::Work));
            assert_eq!(p.cycles_completed, cycle);
            p.start(None, D, now);
            let expected = if cycle == 4 {
                Phase::LongBreak
            } else {
                Phase::ShortBreak
            };
            assert_eq!(p.phase, expected);
            now += D.minutes(expected) * 60_000;
            assert_eq!(p.tick(now), Some(expected));
        }
    }

    #[test]
    fn pause_freezes_and_resume_continues() {
        let mut p = Pomodoro::default();
        p.start(Some(Phase::Work), D, 0);
        p.pause(60_000);
        assert_eq!(p.remaining_ms(999_999), 24 * 60_000);
        p.resume(100_000);
        assert_eq!(p.ends_at, Some(100_000 + 24 * 60_000));
        assert_eq!(countdown(p.remaining_ms(100_001)), "24:00");
    }

    #[test]
    fn hydrate_marks_an_expired_phase_finished_and_idle_persists_as_null() {
        let mut extra = Map::new();
        let mut p = Pomodoro::default();
        p.start(Some(Phase::Work), D, 0);
        p.persist(&mut extra);
        assert_eq!(extra["pomodoroSession"]["phase"], "work");
        let restored = Pomodoro::hydrate(&extra, 30 * 60_000);
        assert_eq!(restored.status, Status::Finished);
        p.reset();
        p.persist(&mut extra);
        assert!(extra["pomodoroSession"].is_null());
    }
}
