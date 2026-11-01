// SPDX-License-Identifier: GPL-3.0-only
//! Daily history interchange: `yyyy-MM-dd,steps` lines without a header.

use chrono::{DateTime, NaiveDate};
use std::collections::BTreeMap;
use thiserror::Error;

pub type History = BTreeMap<NaiveDate, u64>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CsvError {
    #[error("line {line}: expected date,steps")]
    Shape { line: usize },
    #[error("line {line}: invalid date")]
    Date { line: usize },
    #[error("line {line}: invalid step count")]
    Steps { line: usize },
    #[error("the file contains no history")]
    Empty,
}

pub fn write(history: &History) -> String {
    history.iter().map(|(date, steps)| format!("{},{}\r\n", date.format("%Y-%m-%d"), steps)).collect()
}

/// Converts a millisecond timestamp to the nearest UTC midnight's date.
pub fn date_from_legacy_millis(millis: i64) -> Option<NaiveDate> {
    const DAY: i64 = 86_400_000;
    let snapped = millis.checked_add(DAY / 2)?.div_euclid(DAY).checked_mul(DAY)?;
    DateTime::from_timestamp_millis(snapped).map(|moment| moment.date_naive())
}

fn parse_date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .ok()
        .or_else(|| text.parse::<i64>().ok().and_then(date_from_legacy_millis))
}

/// Parses a whole file; any invalid line rejects it. Repeated dates are summed.
pub fn parse(text: &str) -> Result<History, CsvError> {
    let mut history = History::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim().trim_start_matches('\u{feff}');
        if trimmed.is_empty() {
            continue;
        }
        let (date, steps) = trimmed.split_once(',').ok_or(CsvError::Shape { line })?;
        let date = parse_date(date.trim()).ok_or(CsvError::Date { line })?;
        let steps: u64 = steps.trim().parse().map_err(|_| CsvError::Steps { line })?;
        *history.entry(date).or_insert(0) += steps;
    }
    if history.is_empty() {
        return Err(CsvError::Empty);
    }
    Ok(history)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn round_trip_preserves_history() {
        let history = History::from([(day(2024, 2, 29), 12_345), (day(2024, 3, 1), 0)]);
        assert_eq!(parse(&write(&history)).unwrap(), history);
    }

    #[test]
    fn output_uses_crlf_and_no_header() {
        let history = History::from([(day(2024, 1, 2), 3)]);
        assert_eq!(write(&history), "2024-01-02,3\r\n");
    }

    #[test]
    fn legacy_milliseconds_snap_to_the_nearest_midnight() {
        // 2024-03-31 00:00 UTC minus two hours, as a zone-shifted local midnight.
        assert_eq!(parse("1711836000000,5\n").unwrap(), History::from([(day(2024, 3, 31), 5)]));
    }

    #[test]
    fn duplicate_dates_are_summed() {
        let parsed = parse("2024-01-01,1\n2024-01-01,2\n").unwrap();
        assert_eq!(parsed[&day(2024, 1, 1)], 3);
    }

    #[test]
    fn bad_lines_reject_the_file_with_a_location() {
        assert_eq!(parse("2024-01-01,1\nnonsense\n"), Err(CsvError::Shape { line: 2 }));
        assert_eq!(parse("2024-13-01,1\n"), Err(CsvError::Date { line: 1 }));
        assert_eq!(parse("2024-01-01,-1\n"), Err(CsvError::Steps { line: 1 }));
    }

    #[test]
    fn empty_input_is_an_error() {
        assert_eq!(parse("\r\n"), Err(CsvError::Empty));
    }

    #[test]
    fn byte_order_mark_is_ignored() {
        assert_eq!(parse("\u{feff}2024-01-01,1\n").unwrap().len(), 1);
    }
}
