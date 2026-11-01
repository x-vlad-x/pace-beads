// SPDX-License-Identifier: GPL-3.0-only
//! SQLite persistence. Totals and the counter base change in one transaction.

use crate::counter::CounterBase;
use crate::csv::{date_from_legacy_millis, History};
use crate::settings::{first_weekday, key, BackupPolicy, Settings, DEFAULT_BACKUP_RETENTION};
use crate::stats::GoalHistory;
use crate::units::Profile;
use chrono::NaiveDate;
use rusqlite::{params, types::ValueRef, Connection, OpenFlags, OptionalExtension};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

const SCHEMA_VERSION: i32 = 1;
const JOURNAL_RETENTION_DAYS: i64 = 30;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("cannot prepare the data directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("the database uses newer schema version {0}")]
    NewerSchema(i32),
    #[error("stored date is invalid: {0}")]
    BadDate(String),
    #[error("the file has no Stepsy history table")]
    NotStepsy,
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub struct Store {
    connection: Connection,
}

/// One atomic update of the totals and the counter position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit<'a> {
    pub day: NaiveDate,
    pub steps: u64,
    pub base: CounterBase,
    pub wall_time: i64,
    pub note: &'a str,
    pub notified: &'a [(&'a str, NaiveDate)],
}

fn date_text(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn parse_date(text: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| StoreError::BadDate(text.to_owned()))
}

/// Default database location under the user's data directory.
pub fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"));
    base.join("pace-beads/history.sqlite")
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::prepare(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(connection: Connection) -> Result<Self> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        let store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let version: i32 = self.connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(StoreError::NewerSchema(version));
        }
        if version < 1 {
            self.connection.execute_batch(
                "BEGIN;
                 CREATE TABLE days (date TEXT PRIMARY KEY, steps INTEGER NOT NULL);
                 CREATE TABLE counter (
                     id INTEGER PRIMARY KEY CHECK (id = 1),
                     boot_id TEXT NOT NULL,
                     value INTEGER NOT NULL,
                     wall_time INTEGER NOT NULL
                 );
                 CREATE TABLE journal (
                     id INTEGER PRIMARY KEY,
                     wall_time INTEGER NOT NULL,
                     boot_id TEXT NOT NULL,
                     value INTEGER NOT NULL,
                     day TEXT NOT NULL,
                     steps INTEGER NOT NULL,
                     note TEXT NOT NULL
                 );
                 CREATE TABLE goal_changes (date TEXT PRIMARY KEY, goal INTEGER NOT NULL);
                 CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE notified (kind TEXT NOT NULL, day TEXT NOT NULL, PRIMARY KEY (kind, day));
                 PRAGMA user_version = 1;
                 COMMIT;",
            )?;
        }
        Ok(())
    }

    pub fn history(&self) -> Result<History> {
        let mut statement = self.connection.prepare("SELECT date, steps FROM days")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
        let mut history = History::new();
        for row in rows {
            let (date, steps) = row?;
            history.insert(parse_date(&date)?, steps.max(0) as u64);
        }
        Ok(history)
    }

    pub fn day_steps(&self, date: NaiveDate) -> Result<u64> {
        let steps: Option<i64> = self
            .connection
            .query_row("SELECT steps FROM days WHERE date = ?1", [date_text(date)], |row| row.get(0))
            .optional()?;
        Ok(steps.unwrap_or(0).max(0) as u64)
    }

    pub fn lifetime_steps(&self) -> Result<u64> {
        let total: i64 = self.connection.query_row("SELECT COALESCE(SUM(steps), 0) FROM days", [], |row| row.get(0))?;
        Ok(total.max(0) as u64)
    }

    pub fn set_day(&self, date: NaiveDate, steps: u64) -> Result<()> {
        self.connection.execute(
            "INSERT INTO days (date, steps) VALUES (?1, ?2)
             ON CONFLICT (date) DO UPDATE SET steps = excluded.steps",
            params![date_text(date), steps as i64],
        )?;
        Ok(())
    }

    /// Replaces all daily totals in one transaction.
    pub fn replace_history(&mut self, history: &History) -> Result<()> {
        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM days", [])?;
        {
            let mut insert = transaction.prepare("INSERT INTO days (date, steps) VALUES (?1, ?2)")?;
            for (date, steps) in history {
                insert.execute(params![date_text(*date), *steps as i64])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn counter_base(&self) -> Result<Option<(CounterBase, i64)>> {
        Ok(self
            .connection
            .query_row("SELECT boot_id, value, wall_time FROM counter WHERE id = 1", [], |row| {
                Ok((CounterBase { boot_id: row.get(0)?, value: row.get::<_, i64>(1)? as u32 }, row.get(2)?))
            })
            .optional()?)
    }

    pub fn commit(&mut self, commit: &Commit<'_>) -> Result<()> {
        let day = date_text(commit.day);
        let transaction = self.connection.transaction()?;
        if commit.steps > 0 {
            transaction.execute(
                "INSERT INTO days (date, steps) VALUES (?1, ?2)
                 ON CONFLICT (date) DO UPDATE SET steps = steps + excluded.steps",
                params![day, commit.steps as i64],
            )?;
        }
        transaction.execute(
            "INSERT INTO counter (id, boot_id, value, wall_time) VALUES (1, ?1, ?2, ?3)
             ON CONFLICT (id) DO UPDATE SET boot_id = excluded.boot_id,
                 value = excluded.value, wall_time = excluded.wall_time",
            params![commit.base.boot_id, i64::from(commit.base.value), commit.wall_time],
        )?;
        transaction.execute(
            "INSERT INTO journal (wall_time, boot_id, value, day, steps, note)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                commit.wall_time,
                commit.base.boot_id,
                i64::from(commit.base.value),
                day,
                commit.steps as i64,
                commit.note
            ],
        )?;
        for (kind, date) in commit.notified {
            transaction.execute(
                "INSERT OR IGNORE INTO notified (kind, day) VALUES (?1, ?2)",
                params![kind, date_text(*date)],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Removes journal rows and stale notification marks after their retention.
    pub fn prune(&self, today: NaiveDate, now: i64) -> Result<()> {
        self.connection.execute("DELETE FROM journal WHERE wall_time < ?1", [now - JOURNAL_RETENTION_DAYS * 86_400])?;
        self.connection.execute("DELETE FROM notified WHERE day < ?1", [date_text(today)])?;
        Ok(())
    }

    pub fn journal_len(&self) -> Result<u64> {
        let count: i64 = self.connection.query_row("SELECT COUNT(*) FROM journal", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    pub fn notified_kinds(&self, date: NaiveDate) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare("SELECT kind FROM notified WHERE day = ?1")?;
        let rows = statement.query_map([date_text(date)], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn goals(&self) -> Result<GoalHistory> {
        let mut statement = self.connection.prepare("SELECT date, goal FROM goal_changes")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
        let mut changes = Vec::new();
        for row in rows {
            let (date, goal) = row?;
            changes.push((parse_date(&date)?, goal.max(1) as u64));
        }
        Ok(GoalHistory::new(changes))
    }

    /// Sets the goal from `date` onward; later changes stay untouched.
    pub fn set_goal(&self, date: NaiveDate, goal: u64) -> Result<()> {
        self.connection.execute(
            "INSERT INTO goal_changes (date, goal) VALUES (?1, ?2)
             ON CONFLICT (date) DO UPDATE SET goal = excluded.goal",
            params![date_text(date), goal.max(1) as i64],
        )?;
        Ok(())
    }

    pub fn goal_on(&self, date: NaiveDate) -> Result<u64> {
        Ok(self.goals()?.goal_on(date))
    }

    pub fn setting(&self, name: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row("SELECT value FROM settings WHERE key = ?1", [name], |row| row.get(0))
            .optional()?)
    }

    pub fn set_setting(&self, name: &str, value: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![name, value],
        )?;
        Ok(())
    }

    pub fn remove_setting(&self, name: &str) -> Result<()> {
        self.connection.execute("DELETE FROM settings WHERE key = ?1", [name])?;
        Ok(())
    }

    fn parsed<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>> {
        Ok(self.setting(name)?.and_then(|value| value.parse().ok()))
    }

    fn flag(&self, name: &str, default: bool) -> Result<bool> {
        Ok(self.parsed(name)?.unwrap_or(default))
    }

    pub fn settings(&self) -> Result<Settings> {
        let defaults = Profile::default();
        let first_weekday_code = self.parsed(key::FIRST_WEEKDAY)?.unwrap_or(1);
        let interval_days: u32 = self.parsed(key::BACKUP_INTERVAL_DAYS)?.unwrap_or(0);
        let directory = self.setting(key::BACKUP_DIRECTORY)?.filter(|path| !path.is_empty());
        Ok(Settings {
            theme: self.parsed(key::THEME)?.unwrap_or(0),
            date_format: self.parsed(key::DATE_FORMAT)?.unwrap_or(0),
            first_weekday: first_weekday(first_weekday_code),
            first_weekday_code,
            imperial: self.flag(key::IMPERIAL, false)?,
            profile: Profile {
                height_cm: self.parsed(key::HEIGHT_CM)?.unwrap_or(defaults.height_cm),
                weight_kg: self.parsed(key::WEIGHT_KG)?.unwrap_or(defaults.weight_kg),
                step_length_cm: self.parsed(key::STEP_LENGTH_CM)?,
            },
            notify_goal: self.flag(key::NOTIFY_GOAL, true)?,
            notify_encourage: self.flag(key::NOTIFY_ENCOURAGE, true)?,
            notify_milestones: self.flag(key::NOTIFY_MILESTONES, true)?,
            chart_current_week: self.flag(key::CHART_CURRENT_WEEK, false)?,
            backup: directory.filter(|_| interval_days > 0).map(|directory| BackupPolicy {
                directory,
                interval_days,
                retention: self.parsed(key::BACKUP_RETENTION).ok().flatten().unwrap_or(DEFAULT_BACKUP_RETENTION),
            }),
        })
    }

    /// Test helper: makes counter writes fail until `unblock_writes`.
    #[cfg(test)]
    pub(crate) fn block_writes(&self) {
        self.connection
            .execute_batch(
                "CREATE TEMP TRIGGER block_writes BEFORE INSERT ON counter
                 BEGIN SELECT RAISE(ABORT, 'blocked'); END;",
            )
            .unwrap();
    }

    #[cfg(test)]
    pub(crate) fn unblock_writes(&self) {
        self.connection.execute_batch("DROP TRIGGER block_writes;").unwrap();
    }
}

/// Reads the daily history from a copy of a Stepsy database, without changing it.
pub fn read_stepsy_database(path: &Path) -> Result<History> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement = connection.prepare("SELECT date, stepsy FROM History").map_err(|_| StoreError::NotStepsy)?;
    let mut rows = statement.query([])?;
    let mut history = History::new();
    while let Some(row) = rows.next()? {
        let date = match row.get_ref(0)? {
            ValueRef::Text(text) => parse_date(&String::from_utf8_lossy(text))?,
            ValueRef::Integer(millis) => {
                date_from_legacy_millis(millis).ok_or_else(|| StoreError::BadDate(millis.to_string()))?
            }
            other => return Err(StoreError::BadDate(format!("{other:?}"))),
        };
        let steps: i64 = row.get(1)?;
        *history.entry(date).or_insert(0) += steps.max(0) as u64;
    }
    Ok(history)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    pub fn temp_path(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("pace-{name}-{}-{unique}", std::process::id()))
    }

    fn base(value: u32) -> CounterBase {
        CounterBase { boot_id: "a".into(), value }
    }

    #[test]
    fn commit_adds_steps_and_moves_the_base_together() {
        let mut store = Store::open_in_memory().unwrap();
        for value in [10, 25] {
            store
                .commit(&Commit {
                    day: day(2024, 1, 1),
                    steps: 5,
                    base: base(value),
                    wall_time: 1,
                    note: "",
                    notified: &[],
                })
                .unwrap();
        }
        assert_eq!(store.day_steps(day(2024, 1, 1)).unwrap(), 10);
        assert_eq!(store.counter_base().unwrap().unwrap().0, base(25));
        assert_eq!(store.journal_len().unwrap(), 2);
    }

    #[test]
    fn replace_history_is_rolled_back_with_a_failed_transaction() {
        let mut store = Store::open_in_memory().unwrap();
        store.set_day(day(2024, 1, 1), 5).unwrap();
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER reject BEFORE INSERT ON days WHEN NEW.steps = 666
             BEGIN SELECT RAISE(ABORT, 'rejected'); END;",
            )
            .unwrap();
        let bad = History::from([(day(2020, 1, 1), 1), (day(2020, 1, 2), 666)]);
        assert!(store.replace_history(&bad).is_err());
        assert_eq!(store.history().unwrap(), History::from([(day(2024, 1, 1), 5)]));
    }

    #[test]
    fn replace_history_replaces_everything() {
        let mut store = Store::open_in_memory().unwrap();
        store.set_day(day(2024, 1, 1), 5).unwrap();
        let next = History::from([(day(2020, 1, 1), 1)]);
        store.replace_history(&next).unwrap();
        assert_eq!(store.history().unwrap(), next);
    }

    #[test]
    fn data_survives_reopening() {
        let path = temp_path("reopen");
        Store::open(&path).unwrap().set_day(day(2024, 1, 1), 5).unwrap();
        assert_eq!(Store::open(&path).unwrap().day_steps(day(2024, 1, 1)).unwrap(), 5);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn newer_schema_is_refused() {
        let path = temp_path("newer");
        Connection::open(&path).unwrap().pragma_update(None, "user_version", 99).unwrap();
        assert!(matches!(Store::open(&path), Err(StoreError::NewerSchema(99))));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn goal_changes_apply_from_their_date() {
        let store = Store::open_in_memory().unwrap();
        store.set_goal(day(2024, 1, 1), 100).unwrap();
        store.set_goal(day(2024, 2, 1), 200).unwrap();
        assert_eq!(store.goal_on(day(2024, 1, 31)).unwrap(), 100);
        assert_eq!(store.goal_on(day(2024, 2, 1)).unwrap(), 200);
    }

    #[test]
    fn settings_have_documented_defaults() {
        let settings = Store::open_in_memory().unwrap().settings().unwrap();
        assert_eq!(settings.profile, Profile::default());
        assert!(settings.notify_goal && settings.backup.is_none());
    }

    #[test]
    fn backup_policy_needs_a_directory_and_an_interval() {
        let store = Store::open_in_memory().unwrap();
        store.set_setting(key::BACKUP_INTERVAL_DAYS, "3").unwrap();
        assert!(store.settings().unwrap().backup.is_none());
        store.set_setting(key::BACKUP_DIRECTORY, "/tmp/x").unwrap();
        let policy = store.settings().unwrap().backup.unwrap();
        assert_eq!((policy.interval_days, policy.retention), (3, 5));
    }

    fn stepsy_fixture(schema: &str, rows: &str) -> PathBuf {
        let path = temp_path("stepsy");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(schema).unwrap();
        connection.execute_batch(rows).unwrap();
        path
    }

    #[test]
    fn stepsy_v2_database_is_read() {
        let path = stepsy_fixture(
            "CREATE TABLE History (date TEXT PRIMARY KEY, stepsy INT NOT NULL);",
            "INSERT INTO History VALUES ('2024-01-02', 42);",
        );
        assert_eq!(read_stepsy_database(&path).unwrap(), History::from([(day(2024, 1, 2), 42)]));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn stepsy_legacy_rows_are_snapped_and_summed() {
        let path = stepsy_fixture(
            "CREATE TABLE History (date INTEGER PRIMARY KEY, stepsy INT NOT NULL);",
            "INSERT INTO History VALUES (1711836000000, 1), (1711839600000, 2);",
        );
        assert_eq!(read_stepsy_database(&path).unwrap(), History::from([(day(2024, 3, 31), 3)]));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn other_databases_are_refused() {
        let path = stepsy_fixture("CREATE TABLE other (a);", "");
        assert!(matches!(read_stepsy_database(&path), Err(StoreError::NotStepsy)));
        let _ = std::fs::remove_file(path);
    }
}
