//! Background scheduled decode tasks.
//!
//! An operator can arm receive jobs for a future window (mode + frequency).
//! The engine runs them when the radio is free — even if that mode's panel is
//! not open — and when two windows overlap, the last-created task wins.
//!
//! Schedules are one-shot or recurring (daily / weekly / fixed interval).
//! Wall-clock fields may be interpreted in UTC or the station's local zone.

use serde::{Deserialize, Serialize};

use crate::Mode;

/// What a scheduled task is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BackgroundTaskStatus {
    /// Outside its schedule window, or waiting for the window to open.
    #[default]
    Scheduled,
    /// Inside the window, but the radio is busy (TX, scan, digi TX, …).
    Waiting,
    /// Engine is on this task's mode and frequency.
    Running,
    /// Schedule finished (one-shot ended, or recurring `until` passed).
    Done,
}

impl BackgroundTaskStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Waiting => "waiting",
            Self::Running => "running",
            Self::Done => "done",
        }
    }
}

/// How the start/end window repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BackgroundTaskSchedule {
    /// Single absolute window [`BackgroundTask::start_unix`]..[`BackgroundTask::end_unix`].
    #[default]
    Once,
    /// Same wall-clock window every day (duration from start→end).
    Daily,
    /// Same wall-clock window on selected weekdays (`weekdays` bitmask).
    Weekly,
    /// Window of duration start→end, repeating every `period_secs` from `start_unix`.
    Interval,
}

impl BackgroundTaskSchedule {
    pub const ALL: [Self; 4] = [Self::Once, Self::Daily, Self::Weekly, Self::Interval];

    pub fn label(self) -> &'static str {
        match self {
            Self::Once => "one time",
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Interval => "interval",
        }
    }

    pub fn is_recurring(self) -> bool {
        !matches!(self, Self::Once)
    }
}

/// Wall-clock zone for editing and for daily/weekly matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BackgroundTaskTz {
    #[default]
    Utc,
    Local,
}

impl BackgroundTaskTz {
    pub const ALL: [Self; 2] = [Self::Utc, Self::Local];

    pub fn label(self) -> &'static str {
        match self {
            Self::Utc => "UTC",
            Self::Local => "local time",
        }
    }
}

/// Sunday bit in [`BackgroundTask::weekdays`].
pub const WEEKDAY_SUN: u8 = 1 << 0;
/// Monday bit in [`BackgroundTask::weekdays`].
pub const WEEKDAY_MON: u8 = 1 << 1;
/// Tuesday bit in [`BackgroundTask::weekdays`].
pub const WEEKDAY_TUE: u8 = 1 << 2;
/// Wednesday bit in [`BackgroundTask::weekdays`].
pub const WEEKDAY_WED: u8 = 1 << 3;
/// Thursday bit in [`BackgroundTask::weekdays`].
pub const WEEKDAY_THU: u8 = 1 << 4;
/// Friday bit in [`BackgroundTask::weekdays`].
pub const WEEKDAY_FRI: u8 = 1 << 5;
/// Saturday bit in [`BackgroundTask::weekdays`].
pub const WEEKDAY_SAT: u8 = 1 << 6;
/// All seven days.
pub const WEEKDAY_ALL: u8 = 0b0111_1111;

/// One calendar decode job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackgroundTask {
    /// Stable id — monotonically increasing within the station's task list.
    pub id: u64,
    /// Inclusive start of the (first) run window (Unix seconds, UTC).
    pub start_unix: i64,
    /// Exclusive end of the (first) run window (Unix seconds, UTC).
    /// For recurring schedules this sets the per-occurrence duration.
    pub end_unix: i64,
    /// Mode to put the radio into for the window.
    pub mode: Mode,
    /// Dial frequency in Hz.
    pub freq_hz: f64,
    /// Display name of the radio/backend this task belongs to (informational;
    /// each engine only runs tasks for its own radio).
    #[serde(default)]
    pub device: String,
    /// Live status — not authoritative across restarts (`Running` → `Scheduled`).
    #[serde(default)]
    pub status: BackgroundTaskStatus,
    /// Creation / last-edit time (Unix seconds). Higher wins on conflict.
    #[serde(default)]
    pub updated_unix: i64,
    /// One-shot vs recurring.
    #[serde(default)]
    pub schedule: BackgroundTaskSchedule,
    /// For [`BackgroundTaskSchedule::Weekly`]: bits [`WEEKDAY_SUN`]…[`WEEKDAY_SAT`].
    /// Zero is treated as all days after [`Self::sane`].
    #[serde(default)]
    pub weekdays: u8,
    /// For [`BackgroundTaskSchedule::Interval`]: seconds between window starts.
    #[serde(default)]
    pub period_secs: u64,
    /// Optional hard stop for recurring tasks (Unix). `0` = keep going.
    #[serde(default)]
    pub until_unix: i64,
    /// Zone for wall-clock editing and daily/weekly matching.
    #[serde(default)]
    pub tz: BackgroundTaskTz,
}

impl BackgroundTask {
    /// Schedule identity used when merging an import: same window shape and
    /// mode/freq, ignoring id / status / device / updated stamp.
    pub fn same_spec(&self, other: &Self) -> bool {
        self.schedule == other.schedule
            && self.mode == other.mode
            && (self.freq_hz - other.freq_hz).abs() < 0.5
            && self.start_unix == other.start_unix
            && self.end_unix == other.end_unix
            && self.weekdays == other.weekdays
            && self.period_secs == other.period_secs
            && self.until_unix == other.until_unix
            && self.tz == other.tz
    }

    pub fn sane(mut self) -> Self {
        if self.end_unix <= self.start_unix {
            self.end_unix = self.start_unix + 3600;
        }
        if !self.freq_hz.is_finite() || self.freq_hz < 1.0 {
            self.freq_hz = 14_074_000.0;
        }
        self.freq_hz = self.freq_hz.clamp(1.0, 6_000_000_000.0);
        if self.device.trim().is_empty() {
            self.device = "radio".into();
        }
        if self.schedule == BackgroundTaskSchedule::Weekly && self.weekdays == 0 {
            self.weekdays = WEEKDAY_ALL;
        }
        self.weekdays &= WEEKDAY_ALL;
        if self.schedule == BackgroundTaskSchedule::Interval {
            let dur = self.duration_secs().max(60) as u64;
            if self.period_secs < dur {
                self.period_secs = dur.max(3600);
            }
        }
        if self.until_unix < 0 {
            self.until_unix = 0;
        }
        if self.schedule == BackgroundTaskSchedule::Once {
            self.until_unix = 0;
            self.period_secs = 0;
            self.weekdays = 0;
        }
        self
    }

    pub fn duration_secs(&self) -> i64 {
        (self.end_unix - self.start_unix).max(60)
    }

    /// True when the recurrence (or one-shot) will never cover `now` again.
    pub fn is_finished(&self, now: i64) -> bool {
        match self.schedule {
            BackgroundTaskSchedule::Once => now >= self.end_unix,
            _ => self.until_unix > 0 && now >= self.until_unix,
        }
    }

    /// Whether `now` falls inside an active occurrence of this task.
    ///
    /// `offset_at(unix)` returns seconds east of UTC at that instant (0 for a
    /// UTC-only clock). Used when [`Self::tz`] is [`BackgroundTaskTz::Local`].
    pub fn covers(&self, now: i64, offset_at: &impl Fn(i64) -> i64) -> bool {
        if self.is_finished(now) || now < self.start_unix {
            return false;
        }
        let dur = self.duration_secs();
        let shift = |t: i64| -> i64 {
            match self.tz {
                BackgroundTaskTz::Utc => 0,
                BackgroundTaskTz::Local => offset_at(t),
            }
        };
        match self.schedule {
            BackgroundTaskSchedule::Once => now < self.end_unix,
            BackgroundTaskSchedule::Daily => {
                let tod = (self.start_unix + shift(self.start_unix)).rem_euclid(86_400);
                let now_tod = (now + shift(now)).rem_euclid(86_400);
                tod_covers(now_tod, tod, dur)
            }
            BackgroundTaskSchedule::Weekly => {
                let days = if self.weekdays == 0 { WEEKDAY_ALL } else { self.weekdays };
                let dow = weekday_sun0(now + shift(now));
                if days & (1 << dow) == 0 {
                    return false;
                }
                let tod = (self.start_unix + shift(self.start_unix)).rem_euclid(86_400);
                let now_tod = (now + shift(now)).rem_euclid(86_400);
                tod_covers(now_tod, tod, dur)
            }
            BackgroundTaskSchedule::Interval => {
                let period = self.period_secs.max(dur as u64).max(1) as i64;
                let elapsed = now - self.start_unix;
                let into = elapsed.rem_euclid(period);
                into < dur
            }
        }
    }
}

fn tod_covers(now_tod: i64, start_tod: i64, dur: i64) -> bool {
    if dur >= 86_400 {
        return true;
    }
    let end_tod = start_tod + dur;
    if end_tod <= 86_400 {
        now_tod >= start_tod && now_tod < end_tod
    } else {
        // Window wraps past midnight in the task's zone.
        now_tod >= start_tod || now_tod < end_tod - 86_400
    }
}

/// Day-of-week with Sunday = 0 … Saturday = 6 (Unix epoch was a Thursday).
fn weekday_sun0(unix: i64) -> u8 {
    let days = unix.div_euclid(86_400);
    ((days + 4).rem_euclid(7)) as u8
}

/// Persisted list (`background_tasks.json`) plus which task currently owns the dial.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BackgroundTasksConfig {
    pub tasks: Vec<BackgroundTask>,
    /// Next id to hand out on create.
    pub next_id: u64,
    /// Task the engine last put on the dial, if any.
    pub active_id: Option<u64>,
}

impl BackgroundTasksConfig {
    pub fn sane(mut self) -> Self {
        for t in &mut self.tasks {
            *t = t.clone().sane();
        }
        if self.next_id == 0 {
            self.next_id = self.tasks.iter().map(|t| t.id).max().unwrap_or(0).saturating_add(1);
        }
        self
    }

    /// Among tasks covering `now`, the last-updated one (ties → highest id).
    pub fn winner_at(&self, now: i64, offset_at: &impl Fn(i64) -> i64) -> Option<&BackgroundTask> {
        self.tasks
            .iter()
            .filter(|t| t.covers(now, offset_at))
            .max_by(|a, b| a.updated_unix.cmp(&b.updated_unix).then_with(|| a.id.cmp(&b.id)))
    }

    /// Pretty JSON for export (tasks only — ids are reassigned on import).
    pub fn export_json(&self) -> Result<String, String> {
        #[derive(Serialize)]
        struct File<'a> {
            version: u32,
            tasks: &'a [BackgroundTask],
        }
        let file = File { version: 1, tasks: &self.tasks };
        serde_json::to_string_pretty(&file).map_err(|e| e.to_string())
    }

    /// Parse an export file: `{version, tasks}`, a bare task array, or a full
    /// [`BackgroundTasksConfig`].
    pub fn parse_import_json(text: &str) -> Result<Vec<BackgroundTask>, String> {
        let v: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
        if let Ok(tasks) = serde_json::from_value::<Vec<BackgroundTask>>(v.clone()) {
            return Ok(tasks.into_iter().map(|t| t.sane()).collect());
        }
        if let Some(arr) = v.get("tasks") {
            let tasks: Vec<BackgroundTask> = serde_json::from_value(arr.clone())
                .map_err(|e| format!("invalid tasks array: {e}"))?;
            return Ok(tasks.into_iter().map(|t| t.sane()).collect());
        }
        Err("JSON must be a task list or an object with a \"tasks\" array".into())
    }

    /// How many of `imported` would append vs replace under [`Self::merge_import`].
    pub fn import_preview(&self, imported: &[BackgroundTask]) -> (usize, usize) {
        let mut added = 0usize;
        let mut replaced = 0usize;
        for t in imported {
            if self.tasks.iter().any(|e| e.same_spec(t)) {
                replaced += 1;
            } else {
                added += 1;
            }
        }
        (added, replaced)
    }

    /// Merge imported tasks: duplicate specs replace the existing entry;
    /// otherwise append with a fresh id. `device` stamps the open radio label
    /// when `Some`; otherwise the imported device string is kept.
    pub fn merge_import(
        &mut self,
        imported: Vec<BackgroundTask>,
        device: Option<&str>,
        now_unix: i64,
    ) -> (usize, usize) {
        let mut added = 0usize;
        let mut replaced = 0usize;
        for mut t in imported {
            t = t.sane();
            t.status = BackgroundTaskStatus::Scheduled;
            t.updated_unix = now_unix;
            if let Some(d) = device.filter(|s| !s.trim().is_empty()) {
                t.device = d.to_string();
            }
            if let Some(slot) = self.tasks.iter_mut().find(|e| e.same_spec(&t)) {
                t.id = slot.id;
                *slot = t;
                replaced += 1;
            } else {
                let id = self
                    .next_id
                    .max(self.tasks.iter().map(|x| x.id).max().unwrap_or(0).saturating_add(1));
                t.id = id;
                self.next_id = id.saturating_add(1);
                self.tasks.push(t);
                added += 1;
            }
        }
        self.tasks.sort_by_key(|t| t.start_unix);
        (added, replaced)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mode;

    fn utc(_: i64) -> i64 {
        0
    }

    fn task(id: u64, start: i64, end: i64, updated: i64) -> BackgroundTask {
        BackgroundTask {
            id,
            start_unix: start,
            end_unix: end,
            mode: Mode::Ft8,
            freq_hz: 14_074_000.0,
            device: "radio".into(),
            status: BackgroundTaskStatus::Scheduled,
            updated_unix: updated,
            schedule: BackgroundTaskSchedule::Once,
            weekdays: 0,
            period_secs: 0,
            until_unix: 0,
            tz: BackgroundTaskTz::Utc,
        }
        .sane()
    }

    #[test]
    fn last_updated_wins_on_overlap() {
        let cfg = BackgroundTasksConfig {
            tasks: vec![
                task(1, 100, 200, 10),
                task(2, 150, 250, 20), // later edit
                task(3, 300, 400, 30), // outside window
            ],
            next_id: 4,
            active_id: None,
        };
        assert_eq!(cfg.winner_at(160, &utc).map(|t| t.id), Some(2));
        assert_eq!(cfg.winner_at(120, &utc).map(|t| t.id), Some(1));
        assert_eq!(cfg.winner_at(50, &utc), None);
    }

    #[test]
    fn higher_id_breaks_updated_tie() {
        let cfg = BackgroundTasksConfig {
            tasks: vec![task(1, 0, 100, 5), task(9, 0, 100, 5)],
            next_id: 10,
            active_id: None,
        };
        assert_eq!(cfg.winner_at(50, &utc).map(|t| t.id), Some(9));
    }

    #[test]
    fn daily_repeats_same_tod() {
        let mut t = task(1, 10 * 3600, 11 * 3600, 1);
        t.schedule = BackgroundTaskSchedule::Daily;
        assert!(t.covers(10 * 3600 + 30 * 60, &utc));
        assert!(!t.covers(12 * 3600, &utc));
        assert!(t.covers(86_400 + 10 * 3600 + 60, &utc));
        assert!(!t.covers(86_400 + 9 * 3600, &utc));
    }

    #[test]
    fn daily_local_uses_offset() {
        // UTC 08:00 start → local 10:00 at +2h.
        let mut t = task(1, 8 * 3600, 9 * 3600, 1);
        t.schedule = BackgroundTaskSchedule::Daily;
        t.tz = BackgroundTaskTz::Local;
        let east2 = |_: i64| 7200i64;
        // Local 10:30 (= UTC 08:30).
        assert!(t.covers(8 * 3600 + 30 * 60, &east2));
        // Local 09:00 (= UTC 07:00) — before window.
        assert!(!t.covers(7 * 3600, &east2));
        // Next day local 10:15.
        assert!(t.covers(86_400 + 8 * 3600 + 15 * 60, &east2));
    }

    #[test]
    fn weekly_respects_weekday_mask() {
        let mut t = task(1, 0, 3600, 1);
        t.schedule = BackgroundTaskSchedule::Weekly;
        t.weekdays = WEEKDAY_THU;
        assert!(t.covers(30 * 60, &utc));
        assert!(!t.covers(86_400 + 30 * 60, &utc));
        assert!(t.covers(7 * 86_400 + 30 * 60, &utc));
    }

    #[test]
    fn interval_repeats_every_period() {
        let mut t = task(1, 0, 600, 1);
        t.schedule = BackgroundTaskSchedule::Interval;
        t.period_secs = 3600;
        t = t.sane();
        assert!(t.covers(100, &utc));
        assert!(!t.covers(700, &utc));
        assert!(t.covers(3600 + 100, &utc));
        assert!(!t.covers(3600 + 700, &utc));
    }

    #[test]
    fn recurring_until_stops_coverage() {
        let mut t = task(1, 0, 600, 1);
        t.schedule = BackgroundTaskSchedule::Daily;
        t.until_unix = 86_400;
        assert!(t.covers(100, &utc));
        assert!(!t.covers(86_400 + 100, &utc));
        assert!(t.is_finished(86_400));
    }

    #[test]
    fn merge_import_replaces_same_spec() {
        let mut cfg = BackgroundTasksConfig {
            tasks: vec![task(1, 100, 200, 1)],
            next_id: 2,
            active_id: None,
        };
        let incoming = task(99, 100, 200, 1); // same schedule/mode/freq window
        let (added, replaced) = cfg.merge_import(vec![incoming], Some("rtl0"), 50);
        assert_eq!((added, replaced), (0, 1));
        assert_eq!(cfg.tasks.len(), 1);
        assert_eq!(cfg.tasks[0].id, 1);
        assert_eq!(cfg.tasks[0].device, "rtl0");
        assert_eq!(cfg.tasks[0].updated_unix, 50);
    }

    #[test]
    fn merge_import_appends_new_spec() {
        let mut cfg = BackgroundTasksConfig {
            tasks: vec![task(1, 100, 200, 1)],
            next_id: 2,
            active_id: None,
        };
        let (added, replaced) = cfg.merge_import(vec![task(0, 300, 400, 1)], None, 50);
        assert_eq!((added, replaced), (1, 0));
        assert_eq!(cfg.tasks.len(), 2);
        assert_eq!(cfg.tasks[1].id, 2);
    }

    #[test]
    fn export_round_trips() {
        let cfg = BackgroundTasksConfig {
            tasks: vec![task(1, 100, 200, 1)],
            next_id: 2,
            active_id: None,
        };
        let json = cfg.export_json().unwrap();
        let parsed = BackgroundTasksConfig::parse_import_json(&json).unwrap();
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].same_spec(&cfg.tasks[0]));
    }
}
