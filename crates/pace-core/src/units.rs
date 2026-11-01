// SPDX-License-Identifier: GPL-3.0-only
//! Distance and energy estimates. Values are stored in metric units.

pub const DEFAULT_HEIGHT_CM: f64 = 180.0;
pub const DEFAULT_WEIGHT_KG: f64 = 70.0;
pub const STEP_LENGTH_FACTOR: f64 = 0.415;
pub const KM_TO_MILES: f64 = 0.621_371;
pub const KG_TO_LBS: f64 = 2.204_62;
pub const CM_PER_INCH: f64 = 2.54;

/// Groups digits with a narrow no-break space.
pub fn group_digits(value: u64) -> String {
    let digits = value.to_string();
    let mut text = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            text.push('\u{202f}');
        }
        text.push(digit);
    }
    text
}

/// Count for gettext plural selection, which takes 32 bits. Larger values
/// keep the last digits that plural rules look at.
pub fn plural_count(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(1000 + (value % 1000) as u32)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Profile {
    pub height_cm: f64,
    pub weight_kg: f64,
    /// Explicit step length; derived from the height when absent.
    pub step_length_cm: Option<f64>,
}

impl Default for Profile {
    fn default() -> Self {
        Self { height_cm: DEFAULT_HEIGHT_CM, weight_kg: DEFAULT_WEIGHT_KG, step_length_cm: None }
    }
}

impl Profile {
    pub fn effective_step_length_cm(&self) -> f64 {
        self.step_length_cm.unwrap_or(self.height_cm * STEP_LENGTH_FACTOR)
    }

    pub fn distance_km(&self, steps: u64) -> f64 {
        steps as f64 * self.effective_step_length_cm() / 100_000.0
    }

    pub fn kilocalories(&self, steps: u64) -> u64 {
        (steps as f64 * self.weight_kg * 0.0005) as u64
    }
}

pub fn km_to_miles(km: f64) -> f64 {
    km * KM_TO_MILES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_are_grouped() {
        assert_eq!(group_digits(7), "7");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1\u{202f}000");
        assert_eq!(group_digits(1_234_567), "1\u{202f}234\u{202f}567");
    }

    #[test]
    fn plural_count_keeps_the_last_digits() {
        assert_eq!(plural_count(21), 21);
        assert_eq!(plural_count(u64::from(u32::MAX)), u32::MAX);
        assert_eq!(plural_count(5_000_000_021), 1021);
    }

    #[test]
    fn default_step_length_comes_from_the_height() {
        assert!((Profile::default().effective_step_length_cm() - 74.7).abs() < 1e-9);
    }

    #[test]
    fn explicit_step_length_wins() {
        let p = Profile { step_length_cm: Some(80.0), ..Profile::default() };
        assert!((p.distance_km(10_000) - 8.0).abs() < 1e-9);
    }

    #[test]
    fn calories_are_truncated() {
        let p = Profile::default();
        assert_eq!(p.kilocalories(10_000), 350);
        assert_eq!(p.kilocalories(28), 0);
        assert_eq!(p.kilocalories(29), 1);
    }

    #[test]
    fn miles_conversion_matches_the_factor() {
        assert!((km_to_miles(10.0) - 6.21371).abs() < 1e-9);
    }
}
