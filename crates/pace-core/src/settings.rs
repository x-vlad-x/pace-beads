// SPDX-License-Identifier: GPL-3.0-only
//! Typed view of the persisted preferences.

use crate::units::Profile;
use chrono::{DateTime, Local, NaiveDate, Weekday};

pub const DEFAULT_GOAL: u64 = 10_000;
pub const DEFAULT_BACKUP_RETENTION: u32 = 5;

pub mod key {
    pub const THEME: &str = "theme";
    pub const DATE_FORMAT: &str = "date_format";
    pub const FIRST_WEEKDAY: &str = "first_weekday";
    pub const IMPERIAL: &str = "imperial";
    pub const HEIGHT_CM: &str = "height_cm";
    pub const WEIGHT_KG: &str = "weight_kg";
    pub const STEP_LENGTH_CM: &str = "step_length_cm";
    pub const NOTIFY_GOAL: &str = "notify_goal";
    pub const NOTIFY_ENCOURAGE: &str = "notify_encourage";
    pub const NOTIFY_MILESTONES: &str = "notify_milestones";
    pub const CHART_CURRENT_WEEK: &str = "chart_current_week";
    pub const BACKUP_DIRECTORY: &str = "backup_directory";
    pub const BACKUP_INTERVAL_DAYS: &str = "backup_interval_days";
    pub const BACKUP_RETENTION: &str = "backup_retention";
    pub const BACKUP_LAST_DATE: &str = "backup_last_date";
    pub const BACKUP_LAST_ERROR: &str = "backup_last_error";
    pub const PAUSE: &str = "pause";
    pub const MILESTONE_FLOOR: &str = "milestone_floor";
}

pub const DATE_FORMATS: [&str; 5] = ["%Y-%m-%d", "%d.%m.%Y", "%m/%d/%Y", "%d/%m/%Y", "%d-%m-%Y"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PauseState {
    #[default]
    Running,
    Indefinite,
    Until(i64),
}

impl PauseState {
    pub fn encode(self) -> String {
        match self {
            Self::Running => "running".into(),
            Self::Indefinite => "indefinite".into(),
            Self::Until(moment) => format!("until:{moment}"),
        }
    }

    pub fn decode(text: &str) -> Self {
        match text {
            "indefinite" => Self::Indefinite,
            other => {
                other.strip_prefix("until:").and_then(|moment| moment.parse().ok()).map_or(Self::Running, Self::Until)
            }
        }
    }

    pub fn is_paused(self) -> bool {
        !matches!(self, Self::Running)
    }

    pub fn expired(self, now: DateTime<Local>) -> bool {
        matches!(self, Self::Until(moment) if now.timestamp() >= moment)
    }
}

/// Weekday stored as 0 = Sunday, 1 = Monday, 6 = Saturday.
pub fn first_weekday(code: u32) -> Weekday {
    match code {
        0 => Weekday::Sun,
        6 => Weekday::Sat,
        _ => Weekday::Mon,
    }
}

pub fn format_date(date: NaiveDate, format_index: usize) -> String {
    date.format(DATE_FORMATS[format_index.min(DATE_FORMATS.len() - 1)]).to_string()
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackupPolicy {
    pub directory: String,
    pub interval_days: u32,
    pub retention: u32,
}

/// Preferences read as one consistent snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub theme: u32,
    pub date_format: usize,
    pub first_weekday: Weekday,
    pub first_weekday_code: u32,
    pub imperial: bool,
    pub profile: Profile,
    pub notify_goal: bool,
    pub notify_encourage: bool,
    pub notify_milestones: bool,
    pub chart_current_week: bool,
    pub backup: Option<BackupPolicy>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_state_round_trips() {
        for state in [PauseState::Running, PauseState::Indefinite, PauseState::Until(1_700_000_000)] {
            assert_eq!(PauseState::decode(&state.encode()), state);
        }
    }

    #[test]
    fn unreadable_pause_state_means_running() {
        assert_eq!(PauseState::decode("until:soon"), PauseState::Running);
        assert_eq!(PauseState::decode(""), PauseState::Running);
    }

    #[test]
    fn weekday_codes() {
        assert_eq!(first_weekday(0), Weekday::Sun);
        assert_eq!(first_weekday(1), Weekday::Mon);
        assert_eq!(first_weekday(6), Weekday::Sat);
    }

    #[test]
    fn date_formats_cover_all_variants() {
        let date = NaiveDate::from_ymd_opt(2012, 12, 31).unwrap();
        let rendered: Vec<_> = (0..5).map(|index| format_date(date, index)).collect();
        assert_eq!(rendered, ["2012-12-31", "31.12.2012", "12/31/2012", "31/12/2012", "31-12-2012"]);
    }
}
