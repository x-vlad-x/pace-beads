// SPDX-License-Identifier: GPL-3.0-only
//! Pure calculations over the daily history.

use crate::csv::History;
use crate::settings::DEFAULT_GOAL;
use chrono::{Datelike, Days, NaiveDate, Weekday};
use std::collections::BTreeMap;

pub const MILESTONES: [u64; 19] = [
    10_000, 50_000, 100_000, 500_000, 750_000, 1_000_000, 1_500_000, 2_000_000, 3_000_000, 4_000_000, 5_000_000,
    6_000_000, 7_000_000, 8_000_000, 9_000_000, 10_000_000, 12_500_000, 15_000_000, 20_000_000,
];

pub fn milestones() -> impl Iterator<Item = u64> {
    MILESTONES.into_iter()
}

pub fn milestone_badge(threshold: u64) -> &'static str {
    match threshold {
        ..=50_000 => "🥉",
        ..=500_000 => "🥈",
        ..=1_000_000 => "🥇",
        ..=5_000_000 => "🏅",
        ..=10_000_000 => "🏆",
        _ => "👑",
    }
}

/// Goal changes, each applying from its date until the next change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoalHistory(BTreeMap<NaiveDate, u64>);

impl GoalHistory {
    pub fn new(changes: impl IntoIterator<Item = (NaiveDate, u64)>) -> Self {
        Self(changes.into_iter().collect())
    }

    /// Days before the first recorded change use that first goal.
    pub fn goal_on(&self, date: NaiveDate) -> u64 {
        self.0.range(..=date).next_back().or_else(|| self.0.iter().next()).map_or(DEFAULT_GOAL, |(_, goal)| *goal)
    }

    fn met(&self, history: &History, date: NaiveDate) -> bool {
        history.get(&date).is_some_and(|steps| *steps >= self.goal_on(date))
    }
}

fn previous(date: NaiveDate) -> NaiveDate {
    date.pred_opt().unwrap_or(date)
}

/// Consecutive days meeting their goal. An unfinished today does not end it.
pub fn current_streak(history: &History, goals: &GoalHistory, today: NaiveDate) -> u32 {
    let mut count = u32::from(goals.met(history, today));
    let mut date = previous(today);
    while goals.met(history, date) {
        count += 1;
        date = previous(date);
    }
    count
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayRecord {
    pub date: NaiveDate,
    pub steps: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeRecord {
    pub first: NaiveDate,
    pub last: NaiveDate,
    pub steps: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonthRecord {
    pub year: i32,
    pub month: u32,
    pub steps: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreakRecord {
    pub days: u32,
    pub first: NaiveDate,
    pub last: NaiveDate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MilestoneRecord {
    pub threshold: u64,
    pub date: NaiveDate,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Achievements {
    pub first_date: Option<NaiveDate>,
    pub total: u64,
    pub average: u64,
    pub top_days: Vec<DayRecord>,
    pub best_week: Option<RangeRecord>,
    pub best_month: Option<MonthRecord>,
    pub longest_streak: Option<StreakRecord>,
    pub milestones: Vec<MilestoneRecord>,
}

pub fn week_start(date: NaiveDate, first: Weekday) -> NaiveDate {
    date.week(first).first_day()
}

pub fn achievements(history: &History, goals: &GoalHistory, first_weekday: Weekday) -> Achievements {
    let mut result = Achievements { first_date: history.keys().next().copied(), ..Achievements::default() };
    if history.is_empty() {
        return result;
    }

    let mut weeks: BTreeMap<NaiveDate, RangeRecord> = BTreeMap::new();
    let mut months: BTreeMap<(i32, u32), u64> = BTreeMap::new();
    let mut thresholds = milestones().peekable();
    let mut streak: Option<StreakRecord> = None;

    for (&date, &steps) in history {
        result.total += steps;

        weeks
            .entry(week_start(date, first_weekday))
            .and_modify(|week| {
                week.steps += steps;
                week.last = date;
            })
            .or_insert(RangeRecord { first: date, last: date, steps });
        *months.entry((date.year(), date.month())).or_insert(0) += steps;

        while let Some(threshold) = thresholds.next_if(|threshold| result.total >= *threshold) {
            result.milestones.push(MilestoneRecord { threshold, date });
        }

        if steps >= goals.goal_on(date) {
            let extended = streak.filter(|run| run.last.succ_opt() == Some(date));
            let run = extended.map_or(StreakRecord { days: 1, first: date, last: date }, |run| StreakRecord {
                days: run.days + 1,
                first: run.first,
                last: date,
            });
            if result.longest_streak.is_none_or(|best| run.days > best.days) {
                result.longest_streak = Some(run);
            }
            streak = Some(run);
        } else {
            streak = None;
        }
    }

    result.average = result.total / history.len() as u64;

    let mut ranked: Vec<_> = history.iter().map(|(&date, &steps)| DayRecord { date, steps }).collect();
    ranked.sort_by(|a, b| b.steps.cmp(&a.steps).then(a.date.cmp(&b.date)));
    ranked.truncate(3);
    result.top_days = ranked;

    result.best_week = weeks.into_values().reduce(|best, week| if week.steps > best.steps { week } else { best });
    result.best_month = months
        .into_iter()
        .map(|((year, month), steps)| MonthRecord { year, month, steps })
        .reduce(|best, month| if month.steps > best.steps { month } else { best });
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Today,
    Week,
    Month,
    Last7,
    Last30,
    Year,
    AllTime,
}

/// Inclusive date range of a period.
pub fn period_range(
    period: Period,
    history: &History,
    today: NaiveDate,
    first_weekday: Weekday,
) -> (NaiveDate, NaiveDate) {
    let back = |days: u64| today.checked_sub_days(Days::new(days)).unwrap_or(today);
    let start = match period {
        Period::Today => today,
        Period::Week => week_start(today, first_weekday),
        Period::Month => today.with_day(1).unwrap_or(today),
        Period::Last7 => back(6),
        Period::Last30 => back(29),
        Period::Year => today.with_ordinal(1).unwrap_or(today),
        Period::AllTime => history.keys().next().copied().unwrap_or(today).min(today),
    };
    (start, today)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub total: u64,
    pub days: u64,
    pub average: u64,
}

/// Totals for a range; the average covers calendar days in the range.
pub fn summarize(history: &History, first: NaiveDate, last: NaiveDate) -> Summary {
    let total = history.range(first..=last).map(|(_, steps)| *steps).sum();
    let days = (last - first).num_days().max(0) as u64 + 1;
    Summary { total, days, average: total / days }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Day,
    Month,
    Year,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bucket {
    pub first: NaiveDate,
    pub steps: u64,
}

/// Chart bars for a range, including empty days.
pub fn buckets(history: &History, first: NaiveDate, last: NaiveDate) -> (Resolution, Vec<Bucket>) {
    let span = (last - first).num_days();
    let resolution = match span {
        ..=62 => Resolution::Day,
        ..=730 => Resolution::Month,
        _ => Resolution::Year,
    };
    let key = |date: NaiveDate| match resolution {
        Resolution::Day => date,
        Resolution::Month => date.with_day(1).unwrap_or(date),
        Resolution::Year => date.with_ordinal(1).unwrap_or(date),
    };
    let mut bars: BTreeMap<NaiveDate, u64> = BTreeMap::new();
    let mut date = first;
    while date <= last {
        bars.entry(key(date)).or_insert(0);
        date = match date.succ_opt() {
            Some(next) => next,
            None => break,
        };
    }
    for (&date, &steps) in history.range(first..=last) {
        *bars.entry(key(date)).or_insert(0) += steps;
    }
    (resolution, bars.into_iter().map(|(first, steps)| Bucket { first, steps }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn goals(goal: u64) -> GoalHistory {
        GoalHistory::new([(day(2000, 1, 1), goal)])
    }

    #[test]
    fn unfinished_today_keeps_yesterdays_streak() {
        let history = History::from([(day(2024, 5, 1), 100), (day(2024, 5, 2), 100), (day(2024, 5, 3), 5)]);
        assert_eq!(current_streak(&history, &goals(100), day(2024, 5, 3)), 2);
    }

    #[test]
    fn met_today_extends_the_streak() {
        let history = History::from([(day(2024, 5, 2), 100), (day(2024, 5, 3), 100)]);
        assert_eq!(current_streak(&history, &goals(100), day(2024, 5, 3)), 2);
    }

    #[test]
    fn a_missing_day_ends_the_streak() {
        let history = History::from([(day(2024, 5, 1), 100), (day(2024, 5, 3), 100)]);
        assert_eq!(current_streak(&history, &goals(100), day(2024, 5, 3)), 1);
    }

    #[test]
    fn raising_the_goal_does_not_rewrite_past_days() {
        let changes = GoalHistory::new([(day(2024, 1, 1), 100), (day(2024, 5, 3), 1000)]);
        let history = History::from([(day(2024, 5, 1), 100), (day(2024, 5, 2), 100)]);
        assert_eq!(current_streak(&history, &changes, day(2024, 5, 3)), 2);
    }

    #[test]
    fn days_before_the_first_change_use_the_first_goal() {
        assert_eq!(goals(7).goal_on(day(1999, 1, 1)), 7);
        assert_eq!(GoalHistory::default().goal_on(day(2024, 1, 1)), DEFAULT_GOAL);
    }

    #[test]
    fn achievements_of_empty_history() {
        assert_eq!(achievements(&History::new(), &goals(1), Weekday::Mon), Achievements::default());
    }

    #[test]
    fn average_uses_recorded_days() {
        let history = History::from([(day(2024, 1, 1), 10), (day(2024, 3, 1), 21)]);
        assert_eq!(achievements(&history, &goals(1), Weekday::Mon).average, 15);
    }

    #[test]
    fn top_days_are_limited_and_ordered() {
        let history =
            History::from([(day(2024, 1, 1), 1), (day(2024, 1, 2), 4), (day(2024, 1, 3), 3), (day(2024, 1, 4), 2)]);
        let top: Vec<_> =
            achievements(&history, &goals(1), Weekday::Mon).top_days.iter().map(|record| record.steps).collect();
        assert_eq!(top, [4, 3, 2]);
    }

    #[test]
    fn best_week_respects_the_first_weekday() {
        // 2024-06-02 is a Sunday.
        let history = History::from([(day(2024, 6, 1), 5), (day(2024, 6, 2), 5), (day(2024, 6, 3), 3)]);
        let monday = achievements(&history, &goals(1), Weekday::Mon).best_week.unwrap();
        assert_eq!((monday.first, monday.steps), (day(2024, 6, 1), 10));
        let sunday = achievements(&history, &goals(1), Weekday::Sun).best_week.unwrap();
        assert_eq!((sunday.first, sunday.steps), (day(2024, 6, 2), 8));
    }

    #[test]
    fn best_month_sums_the_calendar_month() {
        let history = History::from([(day(2024, 1, 31), 5), (day(2024, 2, 1), 3), (day(2024, 2, 2), 3)]);
        let best = achievements(&history, &goals(1), Weekday::Mon).best_month.unwrap();
        assert_eq!((best.year, best.month, best.steps), (2024, 2, 6));
    }

    #[test]
    fn longest_streak_uses_the_goal_of_each_day() {
        let changes = GoalHistory::new([(day(2024, 1, 1), 10), (day(2024, 1, 3), 100)]);
        let history = History::from([
            (day(2024, 1, 1), 10),
            (day(2024, 1, 2), 10),
            (day(2024, 1, 3), 10),
            (day(2024, 1, 4), 100),
        ]);
        let best = achievements(&history, &changes, Weekday::Mon).longest_streak.unwrap();
        assert_eq!((best.days, best.first, best.last), (2, day(2024, 1, 1), day(2024, 1, 2)));
    }

    #[test]
    fn a_gap_breaks_the_longest_streak() {
        let history = History::from([(day(2024, 1, 1), 1), (day(2024, 1, 3), 1)]);
        assert_eq!(achievements(&history, &goals(1), Weekday::Mon).longest_streak.unwrap().days, 1);
    }

    #[test]
    fn milestones_are_dated_at_the_day_they_are_crossed() {
        let history = History::from([(day(2024, 1, 1), 9_999), (day(2024, 1, 2), 1)]);
        let reached = achievements(&history, &goals(1), Weekday::Mon).milestones;
        assert_eq!(reached, [MilestoneRecord { threshold: 10_000, date: day(2024, 1, 2) }]);
    }

    #[test]
    fn one_large_day_can_cross_several_milestones() {
        let history = History::from([(day(2024, 1, 1), 100_000)]);
        assert_eq!(achievements(&history, &goals(1), Weekday::Mon).milestones.len(), 3);
    }

    #[test]
    fn milestone_list_is_ascending() {
        let list: Vec<_> = milestones().collect();
        assert_eq!(list.len(), 19);
        assert!(list.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn periods_cover_the_expected_ranges() {
        let today = day(2024, 6, 5); // Wednesday
        let history = History::from([(day(2023, 2, 1), 1)]);
        let range = |period| period_range(period, &history, today, Weekday::Mon).0;
        assert_eq!(range(Period::Today), today);
        assert_eq!(range(Period::Week), day(2024, 6, 3));
        assert_eq!(range(Period::Month), day(2024, 6, 1));
        assert_eq!(range(Period::Last7), day(2024, 5, 30));
        assert_eq!(range(Period::Last30), day(2024, 5, 7));
        assert_eq!(range(Period::Year), day(2024, 1, 1));
        assert_eq!(range(Period::AllTime), day(2023, 2, 1));
    }

    #[test]
    fn summary_averages_over_calendar_days() {
        let history = History::from([(day(2024, 1, 1), 70)]);
        let summary = summarize(&history, day(2024, 1, 1), day(2024, 1, 7));
        assert_eq!((summary.total, summary.days, summary.average), (70, 7, 10));
    }

    #[test]
    fn buckets_include_empty_days_in_order() {
        let history = History::from([(day(2024, 1, 2), 3)]);
        let (resolution, bars) = buckets(&history, day(2024, 1, 1), day(2024, 1, 3));
        assert_eq!(resolution, Resolution::Day);
        assert_eq!(bars.iter().map(|bar| bar.steps).collect::<Vec<_>>(), [0, 3, 0]);
    }

    #[test]
    fn long_ranges_are_grouped() {
        let history = History::from([(day(2024, 1, 2), 3), (day(2024, 1, 3), 4)]);
        let (resolution, bars) = buckets(&history, day(2024, 1, 1), day(2024, 12, 31));
        assert_eq!((resolution, bars.len(), bars[0].steps), (Resolution::Month, 12, 7));
        let (resolution, bars) = buckets(&history, day(2020, 1, 1), day(2024, 12, 31));
        assert_eq!((resolution, bars.len()), (Resolution::Year, 5));
    }
}
