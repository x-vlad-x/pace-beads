// SPDX-License-Identifier: GPL-3.0-only
//! Turns cumulative hardware counter readings into step deltas.

/// Hardware counter position persisted together with the daily totals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CounterBase {
    pub boot_id: String,
    pub value: u32,
}

/// Upper bound on sustained walking or running cadence, in steps per second.
const MAX_STEPS_PER_SECOND: u64 = 5;
/// Slack added to the elapsed time so that short intervals are not rejected.
const PLAUSIBILITY_SLACK_SECONDS: u64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observation {
    /// No earlier position exists; the first reading only sets the base.
    FirstReading,
    /// Normal progress within one boot.
    Advanced,
    /// A new boot: the reading is the number of steps since activation.
    NewBoot,
    /// The value went down within one boot; only the base moves.
    CounterReset,
    /// The jump is too large for the elapsed time; only the base moves.
    Implausible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advance {
    pub steps: u64,
    pub observation: Observation,
}

fn plausible(steps: u64, elapsed_seconds: u64) -> bool {
    steps <= elapsed_seconds.saturating_add(PLAUSIBILITY_SLACK_SECONDS) * MAX_STEPS_PER_SECOND
}

/// Compares a reading with the stored base.
///
/// `elapsed_seconds` is the time since the base was written, or the uptime
/// when the boot changed.
pub fn advance(base: Option<&CounterBase>, boot_id: &str, value: u32, elapsed_seconds: u64) -> Advance {
    let (steps, observation) = match base {
        None => (0, Observation::FirstReading),
        Some(base) if base.boot_id != boot_id => (u64::from(value), Observation::NewBoot),
        Some(base) if value < base.value => (0, Observation::CounterReset),
        Some(base) => (u64::from(value - base.value), Observation::Advanced),
    };
    if plausible(steps, elapsed_seconds) {
        Advance { steps, observation }
    } else {
        Advance { steps: 0, observation: Observation::Implausible }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(boot: &str, value: u32) -> CounterBase {
        CounterBase { boot_id: boot.into(), value }
    }

    #[test]
    fn first_reading_counts_nothing() {
        let a = advance(None, "a", 500, 10);
        assert_eq!((a.steps, a.observation), (0, Observation::FirstReading));
    }

    #[test]
    fn progress_within_a_boot_is_the_difference() {
        let a = advance(Some(&base("a", 100)), "a", 130, 30);
        assert_eq!((a.steps, a.observation), (30, Observation::Advanced));
    }

    #[test]
    fn repeated_value_adds_nothing() {
        assert_eq!(advance(Some(&base("a", 100)), "a", 100, 1).steps, 0);
    }

    #[test]
    fn new_boot_counts_the_whole_reading() {
        let a = advance(Some(&base("a", 9000)), "b", 40, 600);
        assert_eq!((a.steps, a.observation), (40, Observation::NewBoot));
    }

    #[test]
    fn new_boot_is_detected_even_when_the_value_is_higher() {
        let a = advance(Some(&base("a", 10)), "b", 40, 600);
        assert_eq!((a.steps, a.observation), (40, Observation::NewBoot));
    }

    #[test]
    fn decrease_within_a_boot_only_moves_the_base() {
        let a = advance(Some(&base("a", 100)), "a", 5, 10);
        assert_eq!((a.steps, a.observation), (0, Observation::CounterReset));
    }

    #[test]
    fn impossible_jump_is_rejected() {
        let a = advance(Some(&base("a", 5)), "a", 100_000, 10);
        assert_eq!((a.steps, a.observation), (0, Observation::Implausible));
    }

    #[test]
    fn long_gap_allows_a_large_delta() {
        assert_eq!(advance(Some(&base("a", 0)), "a", 20_000, 3600 * 3).steps, 20_000);
    }

    #[test]
    fn counter_near_the_top_of_the_range_does_not_overflow() {
        assert_eq!(advance(Some(&base("a", u32::MAX - 1)), "a", u32::MAX, 1).steps, 1);
    }
}
