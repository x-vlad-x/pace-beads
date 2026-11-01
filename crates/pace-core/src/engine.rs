// SPDX-License-Identifier: GPL-3.0-only
//! Recorder behavior without any operating-system dependency.
//!
//! The persisted base always matches the persisted totals. Readings held in
//! memory between flushes are recomputed from the base after a crash, so an
//! interrupted recorder loses no steps as long as the hardware kept counting.

use crate::counter::{advance, CounterBase, Observation};
use crate::settings::{key, PauseState, Settings};
use crate::stats::{milestones, GoalHistory};
use crate::store::{Commit, Result, Store};
use chrono::{DateTime, Local, NaiveDate};
use std::collections::HashSet;

/// Longest time a counted reading stays in memory only.
pub const FLUSH_SECONDS: i64 = 60;

const KIND_ENCOURAGE_LOW: &str = "encourage_15";
const KIND_ENCOURAGE_HIGH: &str = "encourage_75";
const KIND_GOAL: &str = "goal";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alert {
    GoalReached { goal: u64 },
    Encouragement { percent: u32 },
    Milestone { threshold: u64 },
}

pub struct Engine {
    store: Store,
    boot_id: String,
    position: Option<u32>,
    position_time: i64,
    persisted: Option<CounterBase>,
    sampled: bool,
    discard_next: bool,
    pending: u64,
    pending_since: Option<i64>,
    pending_note: &'static str,
    day: NaiveDate,
    day_flushed: u64,
    lifetime_flushed: u64,
    goals: GoalHistory,
    settings: Settings,
    notified: HashSet<String>,
    milestone_floor: u64,
    pause: PauseState,
}

fn note_for(observation: Observation) -> &'static str {
    match observation {
        Observation::FirstReading => "first",
        Observation::Advanced => "",
        Observation::NewBoot => "new-boot",
        Observation::CounterReset => "reset",
        Observation::Implausible => "implausible",
    }
}

impl Engine {
    pub fn new(store: Store, boot_id: String, now: DateTime<Local>) -> Result<Self> {
        let base = store.counter_base()?;
        let pause = PauseState::decode(&store.setting(key::PAUSE)?.unwrap_or_default());
        let settings = store.settings()?;
        let mut engine = Self {
            position: base.as_ref().filter(|(b, _)| b.boot_id == boot_id).map(|(b, _)| b.value),
            position_time: base.as_ref().map_or(now.timestamp(), |(_, time)| *time),
            persisted: base.map(|(base, _)| base),
            store,
            boot_id,
            sampled: false,
            discard_next: false,
            pending: 0,
            pending_since: None,
            pending_note: "",
            day: now.date_naive(),
            day_flushed: 0,
            lifetime_flushed: 0,
            goals: GoalHistory::default(),
            settings,
            notified: HashSet::new(),
            milestone_floor: 0,
            pause,
        };
        if pause.expired(now) {
            // The pause ended while nothing recorded; that interval is unknown.
            engine.discard_next = true;
            engine.set_pause(PauseState::Running)?;
        }
        engine.load_caches()?;
        Ok(engine)
    }

    fn load_caches(&mut self) -> Result<()> {
        self.day_flushed = self.store.day_steps(self.day)?;
        self.lifetime_flushed = self.store.lifetime_steps()?;
        self.goals = self.store.goals()?;
        self.settings = self.store.settings()?;
        self.notified = self.store.notified_kinds(self.day)?.into_iter().collect();
        let stored_floor = self.store.setting(key::MILESTONE_FLOOR)?.and_then(|text| text.parse().ok()).unwrap_or(0);
        // Totals changed by editing or importing never raise a walking alert.
        let reached = milestones().filter(|t| *t <= self.lifetime_flushed + self.pending).last();
        self.milestone_floor = reached.unwrap_or(0).max(stored_floor);
        Ok(())
    }

    pub fn today_steps(&self) -> u64 {
        self.day_flushed + self.pending
    }

    pub fn pause_state(&self) -> PauseState {
        self.pause
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Moment at which the pending readings must reach the disk.
    pub fn flush_deadline(&self) -> Option<i64> {
        self.pending_since.map(|since| since + FLUSH_SECONDS)
    }

    fn commit(&mut self, wall_time: i64, notified: &[(&str, NaiveDate)]) -> Result<()> {
        let Some(value) = self.position else {
            return Ok(());
        };
        let base = CounterBase { boot_id: self.boot_id.clone(), value };
        let unchanged = self.persisted.as_ref() == Some(&base);
        if unchanged && self.pending == 0 && notified.is_empty() {
            return Ok(());
        }
        self.store.commit(&Commit {
            day: self.day,
            steps: self.pending,
            base: base.clone(),
            wall_time,
            note: self.pending_note,
            notified,
        })?;
        self.persisted = Some(base);
        self.day_flushed += self.pending;
        self.lifetime_flushed += self.pending;
        self.pending = 0;
        self.pending_since = None;
        self.pending_note = "";
        Ok(())
    }

    /// Writes pending readings. Safe to call at any time.
    pub fn flush(&mut self, now: DateTime<Local>) -> Result<()> {
        self.commit(now.timestamp(), &[])
    }

    /// Closes the previous day when the local date moved.
    pub fn roll_day(&mut self, now: DateTime<Local>) -> Result<bool> {
        let today = now.date_naive();
        if today == self.day {
            return Ok(false);
        }
        self.flush(now)?;
        self.day = today;
        self.store.prune(today, now.timestamp())?;
        self.load_caches()?;
        Ok(true)
    }

    /// Accepts one cumulative reading. `uptime_seconds` bounds a reading after a reboot.
    pub fn on_sample(&mut self, value: u32, now: DateTime<Local>, uptime_seconds: u64) -> Result<Vec<Alert>> {
        self.roll_day(now)?;
        let previous = self.position.map(|value| CounterBase { boot_id: self.boot_id.clone(), value });
        let base = previous.or_else(|| self.persisted.clone());
        let same_boot = base.as_ref().is_some_and(|base| base.boot_id == self.boot_id);
        let elapsed = if same_boot { (now.timestamp() - self.position_time).max(0) as u64 } else { uptime_seconds };
        let progress = advance(base.as_ref(), &self.boot_id, value, elapsed);
        self.position = Some(value);
        self.position_time = now.timestamp();
        self.sampled = true;

        let note = note_for(progress.observation);
        if !note.is_empty() {
            self.pending_note = note;
        }
        let discarded = std::mem::take(&mut self.discard_next);
        if self.pause.is_paused() || discarded || progress.steps == 0 {
            if discarded || self.persisted.is_none() || progress.observation != Observation::Advanced {
                self.commit(now.timestamp(), &[])?;
            }
            return Ok(Vec::new());
        }

        self.pending += progress.steps;
        self.pending_since.get_or_insert(now.timestamp());
        let (alerts, marks) = self.evaluate_alerts()?;
        if !alerts.is_empty() || !marks.is_empty() {
            let marked: Vec<_> = marks.iter().map(|kind| (*kind, self.day)).collect();
            self.commit(now.timestamp(), &marked)?;
            self.notified.extend(marks.iter().map(|kind| (*kind).to_owned()));
        }
        Ok(alerts)
    }

    fn evaluate_alerts(&mut self) -> Result<(Vec<Alert>, Vec<&'static str>)> {
        let total = self.today_steps();
        let goal = self.goals.goal_on(self.day);
        let crossed = |percent: u64| total * 100 >= goal * percent;
        let pending = |kind: &str, notified: &HashSet<String>| !notified.contains(kind);

        let mut marks = Vec::new();
        let mut alerts = Vec::new();
        let mut highest = None;
        for (percent, kind) in [(15u32, KIND_ENCOURAGE_LOW), (75, KIND_ENCOURAGE_HIGH)] {
            if crossed(u64::from(percent)) && pending(kind, &self.notified) {
                marks.push(kind);
                highest = Some(percent);
            }
        }
        if crossed(100) && pending(KIND_GOAL, &self.notified) {
            marks.push(KIND_GOAL);
            highest = None;
            if self.settings.notify_goal {
                alerts.push(Alert::GoalReached { goal });
            }
        }
        if let Some(percent) = highest.filter(|_| self.settings.notify_encourage) {
            alerts.push(Alert::Encouragement { percent });
        }

        let lifetime = self.lifetime_flushed + self.pending;
        if let Some(threshold) = milestones().filter(|t| *t <= lifetime).last() {
            if threshold > self.milestone_floor {
                self.milestone_floor = threshold;
                self.store.set_setting(key::MILESTONE_FLOOR, &threshold.to_string())?;
                if self.settings.notify_milestones {
                    alerts.push(Alert::Milestone { threshold });
                }
            }
        }
        Ok((alerts, marks))
    }

    fn set_pause(&mut self, pause: PauseState) -> Result<()> {
        self.pause = pause;
        self.store.set_setting(key::PAUSE, &pause.encode())
    }

    pub fn pause(&mut self, until: Option<i64>, now: DateTime<Local>) -> Result<()> {
        self.flush(now)?;
        self.set_pause(until.map_or(PauseState::Indefinite, PauseState::Until))
    }

    pub fn resume(&mut self, now: DateTime<Local>) -> Result<()> {
        if !self.pause.is_paused() {
            return Ok(());
        }
        self.flush(now)?;
        // Without a reading in this run the base predates the pause.
        self.discard_next = !self.sampled;
        self.set_pause(PauseState::Running)
    }

    /// Ends an expired timed pause. Feed the current reading first.
    pub fn tick(&mut self, now: DateTime<Local>) -> Result<bool> {
        if self.pause.expired(now) {
            self.resume(now)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Sets today's total. The counter base stays, so no later step is dropped.
    pub fn set_today(&mut self, steps: u64, now: DateTime<Local>) -> Result<()> {
        self.roll_day(now)?;
        self.flush(now)?;
        self.store.set_day(self.day, steps)?;
        self.load_caches()
    }

    /// Picks up history, goal and settings changed by another process.
    pub fn reload(&mut self, now: DateTime<Local>) -> Result<()> {
        self.roll_day(now)?;
        self.flush(now)?;
        self.load_caches()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::{day, temp_path};
    use chrono::{Duration, TimeZone};

    const UPTIME: u64 = 86_400;

    fn at(hour: u32, minute: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2024, 6, 10, hour, minute, 0).single().unwrap()
    }

    fn engine() -> Engine {
        let store = Store::open_in_memory().unwrap();
        store.set_goal(day(2024, 1, 1), 1000).unwrap();
        Engine::new(store, "boot-a".into(), at(9, 0)).unwrap()
    }

    fn stored(engine: &Engine, date: NaiveDate) -> u64 {
        engine.store.day_steps(date).unwrap()
    }

    #[test]
    fn first_sample_only_sets_the_base() {
        let mut e = engine();
        e.on_sample(500, at(9, 0), UPTIME).unwrap();
        assert_eq!(e.today_steps(), 0);
        assert_eq!(e.store.counter_base().unwrap().unwrap().0.value, 500);
    }

    #[test]
    fn samples_accumulate_and_flush_keeps_the_total() {
        let mut e = engine();
        e.on_sample(100, at(9, 0), UPTIME).unwrap();
        e.on_sample(110, at(9, 1), UPTIME).unwrap();
        e.on_sample(115, at(9, 2), UPTIME).unwrap();
        assert_eq!(e.today_steps(), 15);
        assert_eq!(stored(&e, day(2024, 6, 10)), 0);
        assert_eq!(e.flush_deadline(), Some(at(9, 1).timestamp() + FLUSH_SECONDS));
        e.flush(at(9, 3)).unwrap();
        assert_eq!(stored(&e, day(2024, 6, 10)), 15);
        assert_eq!(e.today_steps(), 15);
        assert_eq!(e.flush_deadline(), None);
    }

    #[test]
    fn crash_between_flushes_loses_no_steps() {
        let path = temp_path("crash");
        let today = day(2024, 6, 10);
        {
            let store = Store::open(&path).unwrap();
            store.set_goal(day(2024, 1, 1), 1000).unwrap();
            let mut e = Engine::new(store, "boot-a".into(), at(9, 0)).unwrap();
            e.on_sample(100, at(9, 0), UPTIME).unwrap();
            e.on_sample(150, at(9, 1), UPTIME).unwrap();
            e.flush(at(9, 1)).unwrap();
            e.on_sample(170, at(9, 2), UPTIME).unwrap();
            // Dropped here without a flush, as in a crash.
        }
        let mut e = Engine::new(Store::open(&path).unwrap(), "boot-a".into(), at(9, 5)).unwrap();
        assert_eq!(e.today_steps(), 50);
        e.on_sample(200, at(9, 6), UPTIME).unwrap();
        e.flush(at(9, 6)).unwrap();
        assert_eq!(stored(&e, today), 100);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn steps_walked_while_the_recorder_was_stopped_are_counted() {
        let path = temp_path("downtime");
        {
            let mut e = Engine::new(Store::open(&path).unwrap(), "boot-a".into(), at(9, 0)).unwrap();
            e.on_sample(100, at(9, 0), UPTIME).unwrap();
        }
        let mut e = Engine::new(Store::open(&path).unwrap(), "boot-a".into(), at(10, 0)).unwrap();
        e.on_sample(1100, at(10, 0), UPTIME).unwrap();
        assert_eq!(e.today_steps(), 1000);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn reboot_counts_steps_since_activation() {
        let path = temp_path("reboot");
        {
            let mut e = Engine::new(Store::open(&path).unwrap(), "boot-a".into(), at(9, 0)).unwrap();
            e.on_sample(5000, at(9, 0), UPTIME).unwrap();
        }
        let mut e = Engine::new(Store::open(&path).unwrap(), "boot-b".into(), at(10, 0)).unwrap();
        e.on_sample(30, at(10, 0), 300).unwrap();
        assert_eq!(e.today_steps(), 30);
        e.on_sample(35, at(10, 1), 360).unwrap();
        assert_eq!(e.today_steps(), 35);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn midnight_flushes_the_old_day_and_later_steps_go_to_the_new_one() {
        let mut e = engine();
        let before = Local.with_ymd_and_hms(2024, 6, 10, 23, 59, 30).single().unwrap();
        let after = before + Duration::minutes(1);
        e.on_sample(100, before, UPTIME).unwrap();
        e.on_sample(120, before + Duration::seconds(5), UPTIME).unwrap();
        e.on_sample(130, after, UPTIME).unwrap();
        e.flush(after).unwrap();
        assert_eq!(stored(&e, day(2024, 6, 10)), 20);
        assert_eq!(stored(&e, day(2024, 6, 11)), 10);
        assert_eq!(e.today_steps(), 10);
    }

    #[test]
    fn midnight_keeps_the_base_and_drops_no_step() {
        let mut e = engine();
        let midnight = Local.with_ymd_and_hms(2024, 6, 11, 0, 0, 1).single().unwrap();
        e.on_sample(100, at(23, 0), UPTIME).unwrap();
        e.on_sample(150, at(23, 30), UPTIME).unwrap();
        assert!(e.roll_day(midnight).unwrap());
        e.on_sample(151, midnight + Duration::seconds(30), UPTIME).unwrap();
        assert_eq!(e.today_steps(), 1);
    }

    #[test]
    fn delta_received_after_midnight_belongs_to_the_day_of_receipt() {
        let mut e = engine();
        let morning = Local.with_ymd_and_hms(2024, 6, 11, 7, 0, 0).single().unwrap();
        e.on_sample(100, at(23, 0), UPTIME).unwrap();
        e.on_sample(600, morning, UPTIME).unwrap();
        e.flush(morning).unwrap();
        assert_eq!(stored(&e, day(2024, 6, 10)), 0);
        assert_eq!(stored(&e, day(2024, 6, 11)), 500);
    }

    #[test]
    fn manual_edit_keeps_steps_after_the_edit() {
        let mut e = engine();
        e.on_sample(100, at(9, 0), UPTIME).unwrap();
        e.on_sample(150, at(9, 1), UPTIME).unwrap();
        e.set_today(1000, at(9, 2)).unwrap();
        assert_eq!(e.today_steps(), 1000);
        e.on_sample(160, at(9, 3), UPTIME).unwrap();
        assert_eq!(e.today_steps(), 1010);
    }

    #[test]
    fn paused_steps_are_excluded_and_counting_continues() {
        let mut e = engine();
        e.on_sample(100, at(9, 0), UPTIME).unwrap();
        e.on_sample(110, at(9, 1), UPTIME).unwrap();
        e.pause(None, at(9, 2)).unwrap();
        e.on_sample(500, at(9, 3), UPTIME).unwrap();
        e.resume(at(9, 4)).unwrap();
        e.on_sample(505, at(9, 5), UPTIME).unwrap();
        assert_eq!(e.today_steps(), 15);
    }

    #[test]
    fn timed_pause_ends_at_its_deadline() {
        let mut e = engine();
        e.on_sample(100, at(9, 0), UPTIME).unwrap();
        e.pause(Some(at(9, 30).timestamp()), at(9, 0)).unwrap();
        e.on_sample(200, at(9, 29), UPTIME).unwrap();
        assert!(!e.tick(at(9, 29)).unwrap());
        assert!(e.tick(at(9, 30)).unwrap());
        assert_eq!(e.pause_state(), PauseState::Running);
        e.on_sample(203, at(9, 31), UPTIME).unwrap();
        assert_eq!(e.today_steps(), 3);
    }

    #[test]
    fn pause_survives_a_restart_and_adds_no_paused_interval() {
        let path = temp_path("pause");
        {
            let mut e = Engine::new(Store::open(&path).unwrap(), "boot-a".into(), at(9, 0)).unwrap();
            e.on_sample(100, at(9, 0), UPTIME).unwrap();
            e.pause(None, at(9, 1)).unwrap();
        }
        let mut e = Engine::new(Store::open(&path).unwrap(), "boot-a".into(), at(9, 30)).unwrap();
        assert_eq!(e.pause_state(), PauseState::Indefinite);
        e.resume(at(9, 31)).unwrap();
        e.on_sample(900, at(9, 32), UPTIME).unwrap();
        e.on_sample(903, at(9, 33), UPTIME).unwrap();
        assert_eq!(e.today_steps(), 3);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pause_expired_during_downtime_does_not_add_the_interval() {
        let path = temp_path("pause-expired");
        {
            let mut e = Engine::new(Store::open(&path).unwrap(), "boot-a".into(), at(9, 0)).unwrap();
            e.on_sample(100, at(9, 0), UPTIME).unwrap();
            e.pause(Some(at(9, 30).timestamp()), at(9, 1)).unwrap();
        }
        let mut e = Engine::new(Store::open(&path).unwrap(), "boot-a".into(), at(10, 0)).unwrap();
        assert_eq!(e.pause_state(), PauseState::Running);
        e.on_sample(900, at(10, 1), UPTIME).unwrap();
        assert_eq!(e.today_steps(), 0);
        e.on_sample(901, at(10, 2), UPTIME).unwrap();
        assert_eq!(e.today_steps(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn goal_alert_fires_once_and_survives_a_restart() {
        let path = temp_path("goal");
        let store = Store::open(&path).unwrap();
        store.set_goal(day(2024, 1, 1), 100).unwrap();
        let mut e = Engine::new(store, "boot-a".into(), at(9, 0)).unwrap();
        e.on_sample(0, at(9, 0), UPTIME).unwrap();
        let alerts = e.on_sample(100, at(9, 5), UPTIME).unwrap();
        assert!(alerts.contains(&Alert::GoalReached { goal: 100 }));
        assert!(e.on_sample(150, at(9, 6), UPTIME).unwrap().is_empty());
        drop(e);
        let mut e = Engine::new(Store::open(&path).unwrap(), "boot-a".into(), at(9, 7)).unwrap();
        assert!(e.on_sample(160, at(9, 8), UPTIME).unwrap().is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn encouragements_fire_at_their_thresholds() {
        let mut e = engine();
        e.on_sample(0, at(9, 0), UPTIME).unwrap();
        assert!(e.on_sample(149, at(9, 30), UPTIME).unwrap().is_empty());
        assert_eq!(e.on_sample(150, at(9, 31), UPTIME).unwrap(), [Alert::Encouragement { percent: 15 }]);
        assert_eq!(e.on_sample(750, at(10, 30), UPTIME).unwrap(), [Alert::Encouragement { percent: 75 }]);
    }

    #[test]
    fn disabled_notifications_stay_silent() {
        let store = Store::open_in_memory().unwrap();
        store.set_goal(day(2024, 1, 1), 100).unwrap();
        for name in [key::NOTIFY_GOAL, key::NOTIFY_ENCOURAGE, key::NOTIFY_MILESTONES] {
            store.set_setting(name, "false").unwrap();
        }
        let mut e = Engine::new(store, "boot-a".into(), at(9, 0)).unwrap();
        e.on_sample(0, at(9, 0), UPTIME).unwrap();
        assert!(e.on_sample(20_000, at(13, 0), UPTIME).unwrap().is_empty());
    }

    #[test]
    fn milestone_alert_is_once_and_import_does_not_fire_it() {
        let mut e = engine();
        e.store.set_day(day(2024, 6, 1), 9_990).unwrap();
        e.reload(at(9, 0)).unwrap();
        e.on_sample(0, at(9, 0), UPTIME).unwrap();
        let alerts = e.on_sample(20, at(9, 30), UPTIME).unwrap();
        assert!(alerts.contains(&Alert::Milestone { threshold: 10_000 }));

        e.store.set_day(day(2024, 6, 2), 60_000).unwrap();
        e.reload(at(9, 31)).unwrap();
        let alerts = e.on_sample(21, at(9, 32), UPTIME).unwrap();
        assert!(!alerts.iter().any(|alert| matches!(alert, Alert::Milestone { .. })));
    }

    #[test]
    fn reload_uses_a_new_goal_without_losing_pending_steps() {
        let mut e = engine();
        e.on_sample(0, at(9, 0), UPTIME).unwrap();
        e.on_sample(50, at(9, 30), UPTIME).unwrap();
        e.store.set_goal(day(2024, 6, 10), 80).unwrap();
        e.reload(at(9, 31)).unwrap();
        assert_eq!(e.today_steps(), 50);
        let alerts = e.on_sample(80, at(9, 40), UPTIME).unwrap();
        assert!(alerts.contains(&Alert::GoalReached { goal: 80 }));
    }

    #[test]
    fn failed_write_keeps_pending_steps_for_a_retry() {
        let mut e = engine();
        e.on_sample(0, at(9, 0), UPTIME).unwrap();
        e.on_sample(30, at(9, 1), UPTIME).unwrap();
        e.store.block_writes();
        assert!(e.flush(at(9, 2)).is_err());
        e.store.unblock_writes();
        assert_eq!(e.today_steps(), 30);
        e.flush(at(9, 3)).unwrap();
        assert_eq!(stored(&e, day(2024, 6, 10)), 30);
    }
}
