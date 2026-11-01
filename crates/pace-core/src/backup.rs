// SPDX-License-Identifier: GPL-3.0-only
//! Automatic history copies with atomic publication and retention.

use crate::csv::{self, History};
use crate::settings::{key, BackupPolicy};
use crate::store::{Result as StoreResult, Store};
use chrono::{DateTime, Local, NaiveDate};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const PREFIX: &str = "pace-beads_";
const SUFFIX: &str = ".csv";

pub fn is_due(last: Option<NaiveDate>, today: NaiveDate, interval_days: u32) -> bool {
    interval_days > 0 && last.is_none_or(|last| (today - last).num_days() >= i64::from(interval_days))
}

pub fn file_name(now: DateTime<Local>) -> String {
    format!("{PREFIX}{}{SUFFIX}", now.format("%Y%m%d-%H%M%S"))
}

/// Writes one copy; a failure leaves neither a partial nor a final file.
pub fn write_copy(directory: &Path, name: &str, history: &History) -> io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let target = directory.join(name);
    let partial = directory.join(format!(".{name}.partial"));
    let result = (|| {
        let mut file = File::create(&partial)?;
        file.write_all(csv::write(history).as_bytes())?;
        file.sync_all()?;
        fs::rename(&partial, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result.map(|()| target)
}

/// Keeps the newest `keep` copies; zero keeps all. Other files are not touched.
pub fn prune(directory: &Path, keep: u32) -> io::Result<u32> {
    if keep == 0 {
        return Ok(0);
    }
    let mut copies: Vec<PathBuf> = fs::read_dir(directory)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(PREFIX) && name.ends_with(SUFFIX))
        })
        .collect();
    copies.sort();
    let surplus = copies.len().saturating_sub(keep as usize);
    for path in &copies[..surplus] {
        fs::remove_file(path)?;
    }
    Ok(surplus as u32)
}

fn copy_now(policy: &BackupPolicy, history: &History, now: DateTime<Local>) -> io::Result<()> {
    let directory = Path::new(&policy.directory);
    write_copy(directory, &file_name(now), history)?;
    prune(directory, policy.retention)?;
    Ok(())
}

/// Runs a due copy and records its outcome for the interface.
pub fn run_if_due(store: &Store, now: DateTime<Local>) -> StoreResult<bool> {
    let Some(policy) = store.settings()?.backup else {
        return Ok(false);
    };
    let today = now.date_naive();
    let last = store.setting(key::BACKUP_LAST_DATE)?.and_then(|text| NaiveDate::parse_from_str(&text, "%Y-%m-%d").ok());
    if !is_due(last, today, policy.interval_days) {
        return Ok(false);
    }
    match copy_now(&policy, &store.history()?, now) {
        Ok(()) => {
            store.set_setting(key::BACKUP_LAST_DATE, &today.format("%Y-%m-%d").to_string())?;
            store.remove_setting(key::BACKUP_LAST_ERROR)?;
            Ok(true)
        }
        Err(error) => {
            store.set_setting(key::BACKUP_LAST_ERROR, &error.to_string())?;
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::{day, temp_path};
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, 0, 0, 1).single().unwrap()
    }

    #[test]
    fn due_rules() {
        assert!(is_due(None, day(2024, 1, 1), 3));
        assert!(!is_due(Some(day(2024, 1, 1)), day(2024, 1, 3), 3));
        assert!(is_due(Some(day(2024, 1, 1)), day(2024, 1, 4), 3));
        assert!(!is_due(None, day(2024, 1, 1), 0));
    }

    #[test]
    fn copy_is_a_readable_csv_without_leftovers() {
        let directory = temp_path("backup");
        let history = History::from([(day(2024, 1, 1), 5)]);
        let path = write_copy(&directory, "pace-beads_1.csv", &history).unwrap();
        assert_eq!(csv::parse(&fs::read_to_string(path).unwrap()).unwrap(), history);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_copy_leaves_no_partial_file() {
        let directory = temp_path("backup-fail");
        fs::create_dir_all(directory.join("taken.csv")).unwrap();
        assert!(write_copy(&directory, "taken.csv", &History::new()).is_err());
        let names: Vec<_> = fs::read_dir(&directory).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["taken.csv"]);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn retention_keeps_the_newest_and_ignores_other_files() {
        let directory = temp_path("backup-retain");
        fs::create_dir_all(&directory).unwrap();
        for name in ["pace-beads_1.csv", "pace-beads_2.csv", "pace-beads_3.csv", "notes.csv"] {
            fs::write(directory.join(name), "").unwrap();
        }
        assert_eq!(prune(&directory, 2).unwrap(), 1);
        assert!(!directory.join("pace-beads_1.csv").exists());
        assert!(directory.join("pace-beads_2.csv").exists());
        assert!(directory.join("notes.csv").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn zero_retention_keeps_everything() {
        let directory = temp_path("backup-all");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("pace-beads_1.csv"), "").unwrap();
        assert_eq!(prune(&directory, 0).unwrap(), 0);
        fs::remove_dir_all(directory).unwrap();
    }

    fn configured(directory: &Path) -> Store {
        let store = Store::open_in_memory().unwrap();
        store.set_setting(key::BACKUP_DIRECTORY, directory.to_str().unwrap()).unwrap();
        store.set_setting(key::BACKUP_INTERVAL_DAYS, "1").unwrap();
        store.set_day(day(2024, 1, 1), 5).unwrap();
        store
    }

    #[test]
    fn scheduled_copy_runs_once_per_interval() {
        let directory = temp_path("backup-run");
        let store = configured(&directory);
        assert!(run_if_due(&store, at(2024, 1, 2)).unwrap());
        assert!(!run_if_due(&store, at(2024, 1, 2)).unwrap());
        assert!(run_if_due(&store, at(2024, 1, 3)).unwrap());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unavailable_directory_is_reported_and_recovers() {
        let blocker = temp_path("backup-blocker");
        fs::write(&blocker, "").unwrap();
        let store = configured(&blocker.join("child"));
        assert!(!run_if_due(&store, at(2024, 1, 2)).unwrap());
        assert!(store.setting(key::BACKUP_LAST_ERROR).unwrap().is_some());

        fs::remove_file(&blocker).unwrap();
        assert!(run_if_due(&store, at(2024, 1, 2)).unwrap());
        assert!(store.setting(key::BACKUP_LAST_ERROR).unwrap().is_none());
        fs::remove_dir_all(blocker).unwrap();
    }
}
