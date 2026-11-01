// SPDX-License-Identifier: GPL-3.0-only
//! Text formatting shared by the pages.

use chrono::NaiveDate;
use gettextrs::{gettext, ngettext, pgettext};
use pace_core::settings::{format_date, Settings};
use pace_core::units::{group_digits, km_to_miles, plural_count};

pub fn steps(value: u64) -> String {
    group_digits(value)
}

/// "1 step", "2 steps": the number with its unit, in the right plural form.
pub fn step_count(value: u64) -> String {
    fill(&ngettext("{} step", "{} steps", plural_count(value)), &[&steps(value)])
}

pub fn day_count(value: u64) -> String {
    fill(&ngettext("{} day", "{} days", plural_count(value)), &[&steps(value)])
}

pub fn period_count(value: u64) -> String {
    fill(&ngettext("{} period", "{} periods", plural_count(value)), &[&value.to_string()])
}

fn two_decimals(value: f64, separator: &str) -> String {
    format!("{value:.2}").replacen('.', separator, 1)
}

pub fn distance(settings: &Settings, step_count: u64) -> String {
    let km = settings.profile.distance_km(step_count);
    let separator = pgettext("decimal separator", ".");
    if settings.imperial {
        format!("{} {}", two_decimals(km_to_miles(km), &separator), gettext("mi"))
    } else {
        format!("{} {}", two_decimals(km, &separator), gettext("km"))
    }
}

pub fn energy(settings: &Settings, step_count: u64) -> String {
    format!("{} {}", steps(settings.profile.kilocalories(step_count)), gettext("kcal"))
}

pub fn date(settings: &Settings, value: NaiveDate) -> String {
    format_date(value, settings.date_format)
}

pub fn fill(template: &str, values: &[&str]) -> String {
    values.iter().fold(template.to_owned(), |text, value| text.replacen("{}", value, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_use_the_given_separator() {
        assert_eq!(two_decimals(3.756, "."), "3.76");
        assert_eq!(two_decimals(3.756, ","), "3,76");
        assert_eq!(two_decimals(0.0, ","), "0,00");
    }

    #[test]
    fn templates_fill_in_order() {
        assert_eq!(fill("{} of {}", &["1", "2"]), "1 of 2");
    }
}
