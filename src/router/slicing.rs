use super::fixed::{Fixed, PPM};

/// A single child order in an execution schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slice {
    pub index: usize,
    /// Offset from the start of the parent order, in seconds.
    pub scheduled_at_s: i64,
    /// Target quantity for this child order.
    pub quantity: Fixed,
    /// Fraction of the parent quantity targeted by this slice, in ppm.
    pub target_fraction_ppm: i64,
}

/// Target execution style for a parent order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionStyle {
    /// Equal-sized slices at fixed intervals.
    Twap,
    /// Slices sized to a historical intraday volume profile.
    Vwap { volume_curve: Vec<i64> },
}

/// A fully expanded slicing schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceSchedule {
    pub style: ExecutionStyle,
    pub total_quantity: Fixed,
    pub duration_s: i64,
    pub interval_s: i64,
    pub slices: Vec<Slice>,
}

impl SliceSchedule {
    /// Build a TWAP schedule distributing `total_quantity` evenly across
    /// `ceil(duration / interval)` child orders.
    pub fn twap(total_quantity: Fixed, duration_s: i64, interval_s: i64) -> Self {
        Self::build(
            ExecutionStyle::Twap,
            total_quantity,
            duration_s,
            interval_s,
            None,
        )
    }

    /// Build a VWAP schedule weighting slices by `volume_curve`. The curve is
    /// resampled to the number of intervals via linear interpolation.
    pub fn vwap(
        total_quantity: Fixed,
        duration_s: i64,
        interval_s: i64,
        volume_curve: Vec<i64>,
    ) -> Self {
        Self::build(
            ExecutionStyle::Vwap {
                volume_curve: volume_curve.clone(),
            },
            total_quantity,
            duration_s,
            interval_s,
            Some(volume_curve),
        )
    }

    fn build(
        style: ExecutionStyle,
        total_quantity: Fixed,
        duration_s: i64,
        interval_s: i64,
        volume_curve: Option<Vec<i64>>,
    ) -> Self {
        let interval_s = if interval_s > 0 { interval_s } else { 1 };
        let duration_s = duration_s.max(0);
        // Round to the nearest number of intervals (at least one).
        let n = (((duration_s + interval_s / 2) / interval_s).max(1)) as usize;
        let weights = match volume_curve {
            Some(curve) => resample(&curve, n),
            None => vec![1; n],
        };
        let weight_sum: i128 = weights.iter().map(|&w| w.max(0) as i128).sum();
        let weight_sum = if weight_sum > 0 { weight_sum } else { 1 };

        let mut slices = Vec::with_capacity(n);
        let mut allocated = Fixed::ZERO;
        for (index, weight) in weights.iter().enumerate() {
            let weight = (*weight).max(0);
            let fraction_ppm = (weight as i128 * PPM as i128 / weight_sum) as i64;
            // Give the final slice the exact remainder to avoid drift.
            let quantity = if index == n - 1 {
                total_quantity - allocated
            } else {
                Fixed::from_raw((total_quantity.raw() as i128 * weight as i128 / weight_sum) as i64)
            }
            .max(Fixed::ZERO);
            allocated += quantity;
            slices.push(Slice {
                index,
                scheduled_at_s: index as i64 * interval_s,
                quantity,
                target_fraction_ppm: fraction_ppm,
            });
        }

        SliceSchedule {
            style,
            total_quantity,
            duration_s,
            interval_s,
            slices,
        }
    }

    pub fn len(&self) -> usize {
        self.slices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slices.is_empty()
    }

    /// Sum of all slice quantities (should equal `total_quantity`).
    pub fn scheduled_quantity(&self) -> Fixed {
        self.slices
            .iter()
            .fold(Fixed::ZERO, |acc, slice| acc + slice.quantity)
    }
}

/// Resample an integer volume curve to exactly `n` buckets using linear
/// interpolation. Negative samples are clamped to zero.
pub fn resample(curve: &[i64], n: usize) -> Vec<i64> {
    if n == 0 {
        return Vec::new();
    }
    if curve.is_empty() {
        return vec![1; n];
    }
    if curve.len() == 1 {
        return vec![curve[0].max(0); n];
    }
    if curve.len() == n {
        return curve.iter().map(|&value| value.max(0)).collect();
    }

    let last = (curve.len() - 1) as i128;
    (0..n)
        .map(|i| {
            let position = if n == 1 {
                0
            } else {
                i as i128 * last / (n - 1) as i128
            };
            let lower = position as usize;
            let upper = (position + 1).min(last) as usize;
            if lower == upper {
                curve[lower].max(0)
            } else {
                // Reconstruct the fractional part without floats.
                let numerator = if n == 1 {
                    0
                } else {
                    (i as i128 * last) % (n - 1) as i128
                };
                let denominator = (n - 1) as i128;
                let value = (curve[lower] as i128 * (denominator - numerator)
                    + curve[upper] as i128 * numerator)
                    / denominator;
                value.max(0) as i64
            }
        })
        .collect()
}

/// A classic U-shaped intraday volume profile (heavier at the open and close),
/// scaled by ten.
pub fn u_shaped_volume_curve() -> Vec<i64> {
    vec![
        30, 22, 18, 15, 13, 12, 11, 10, 10, 10, 10, 11, 12, 14, 17, 21, 26, 32,
    ]
}

/// Format a fraction in ppm as a percentage for display.
pub fn ppm_to_percent(ppm: i64) -> f64 {
    ppm as f64 / PPM as f64 * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(value: f64) -> Fixed {
        Fixed::from_f64(value)
    }

    #[test]
    fn test_twap_equal_slices() {
        let schedule = SliceSchedule::twap(fixed(1_000.0), 300, 60);
        assert_eq!(schedule.len(), 5);
        for slice in &schedule.slices {
            assert_eq!(slice.quantity, fixed(200.0));
        }
        assert_eq!(schedule.scheduled_quantity(), fixed(1_000.0));
    }

    #[test]
    fn test_twap_remainder_is_exact() {
        let schedule = SliceSchedule::twap(fixed(100.0), 300, 120);
        assert_eq!(schedule.len(), 3);
        assert_eq!(schedule.scheduled_quantity(), fixed(100.0));
    }

    #[test]
    fn test_twap_timestamps() {
        let schedule = SliceSchedule::twap(fixed(90.0), 30, 10);
        let times: Vec<i64> = schedule.slices.iter().map(|s| s.scheduled_at_s).collect();
        assert_eq!(times, vec![0, 10, 20]);
    }

    #[test]
    fn test_vwap_weights_follow_curve() {
        let schedule = SliceSchedule::vwap(fixed(1_000.0), 40, 10, vec![1, 1, 1, 7]);
        assert_eq!(schedule.len(), 4);
        assert_eq!(schedule.slices[0].quantity, fixed(100.0));
        assert_eq!(schedule.slices[3].quantity, fixed(700.0));
        assert_eq!(schedule.scheduled_quantity(), fixed(1_000.0));
    }

    #[test]
    fn test_vwap_resamples_curve() {
        let schedule = SliceSchedule::vwap(fixed(1_000.0), 50, 10, u_shaped_volume_curve());
        assert_eq!(schedule.len(), 5);
        let first = schedule.slices.first().unwrap().quantity;
        let middle = schedule.slices[2].quantity;
        let last = schedule.slices.last().unwrap().quantity;
        assert!(first > middle);
        assert!(last > middle);
    }

    #[test]
    fn test_single_interval() {
        let schedule = SliceSchedule::twap(fixed(500.0), 0, 60);
        assert_eq!(schedule.len(), 1);
        assert_eq!(schedule.slices[0].quantity, fixed(500.0));
    }

    #[test]
    fn test_zero_duration_and_interval_are_safe() {
        let schedule = SliceSchedule::twap(fixed(100.0), -5, 0);
        assert_eq!(schedule.len(), 1);
    }

    #[test]
    fn test_resample_interpolates() {
        let r = resample(&[0, 10], 3);
        assert_eq!(r, vec![0, 5, 10]);
    }

    #[test]
    fn test_resample_handles_empties() {
        assert!(resample(&[], 0).is_empty());
        assert_eq!(resample(&[], 3), vec![1, 1, 1]);
        assert_eq!(resample(&[5], 2), vec![5, 5]);
        assert_eq!(resample(&[-1, -2], 2), vec![0, 0]);
    }

    #[test]
    fn test_target_fractions_sum_to_ppm() {
        let schedule = SliceSchedule::vwap(fixed(1_000.0), 50, 10, u_shaped_volume_curve());
        let sum: i64 = schedule
            .slices
            .iter()
            .map(|slice| slice.target_fraction_ppm)
            .sum();
        assert!((sum - PPM).abs() <= schedule.len() as i64);
    }
}
