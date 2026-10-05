use super::fixed::{fill_probability_ppm, sqrt_fixed, Fixed, MILLI_BPS, PPM, SCALE};

/// Microstructure-aware market impact model.
///
/// The temporary component follows the common square-root law: impact grows
/// with the square root of participation (order size relative to average daily
/// volume) and scales with volatility. The permanent component is linear in
/// participation. All coefficients are integer (milli-bps) and all arithmetic
/// is fixed-point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImpactParams {
    /// Temporary impact coefficient, in milli-bps per unit of sqrt(participation).
    pub temporary_coeff_millibps: i64,
    /// Permanent impact coefficient, in milli-bps per unit of participation.
    pub permanent_coeff_millibps: i64,
    /// Reference daily volatility in basis points.
    pub daily_vol_bps: i64,
    /// Assumed length of a trading day, used to annualise participation.
    pub trading_day_s: i64,
}

impl Default for ImpactParams {
    fn default() -> Self {
        ImpactParams {
            // 10.0 and 2.0 dimensionless coefficients.
            temporary_coeff_millibps: 10 * MILLI_BPS,
            permanent_coeff_millibps: 2 * MILLI_BPS,
            daily_vol_bps: 150,
            trading_day_s: 23_400,
        }
    }
}

impl ImpactParams {
    /// Fraction of average daily volume traded by `qty`, as a `Fixed`.
    pub fn participation(&self, qty: Fixed, adv: Fixed) -> Fixed {
        if adv.raw() > 0 {
            qty.div(adv).max(Fixed::ZERO)
        } else {
            Fixed::ZERO
        }
    }

    /// Expected one-way temporary impact in milli-basis-points.
    pub fn temporary_millibps(&self, qty: Fixed, adv: Fixed) -> i64 {
        let participation = self.participation(qty, adv);
        let sqrt_participation = sqrt_fixed(participation);
        // coeff * vol * sqrt(participation) / 100
        (self.temporary_coeff_millibps as i64
            * self.daily_vol_bps as i64
            * sqrt_participation.raw())
            / (100 * SCALE)
    }

    /// Expected permanent impact in milli-basis-points.
    pub fn permanent_millibps(&self, qty: Fixed, adv: Fixed) -> i64 {
        let participation = self.participation(qty, adv);
        (self.permanent_coeff_millibps as i64 * self.daily_vol_bps as i64 * participation.raw())
            / (100 * SCALE)
    }

    /// Combined temporary + permanent impact.
    pub fn total_millibps(&self, qty: Fixed, adv: Fixed) -> i64 {
        self.temporary_millibps(qty, adv) + self.permanent_millibps(qty, adv)
    }
}

/// Passive queue dynamics model.
///
/// A passive order only fills after the queue ahead of it has traded. Assuming
/// fills at a level arrive as a Poisson process with rate `trade_rate`, the
/// probability that an order of `our_size` placed behind `queue_ahead` fills
/// within `horizon_s` seconds is approximated by the saturating rational
/// function in [`fill_probability_ppm`], keeping the model integer-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueModel {
    /// Shares per second expected to trade through the level being joined.
    pub trade_rate: Fixed,
}

impl Default for QueueModel {
    fn default() -> Self {
        QueueModel {
            trade_rate: Fixed::from_raw(250 * SCALE),
        }
    }
}

impl QueueModel {
    pub fn from_shares_per_second(trade_rate: f64) -> Self {
        QueueModel {
            trade_rate: Fixed::from_f64(trade_rate),
        }
    }

    /// Probability of a passive fill within `horizon_s` seconds, in ppm.
    pub fn fill_probability_ppm(&self, queue_ahead: Fixed, our_size: Fixed, horizon_s: i64) -> i64 {
        let effective_queue = queue_ahead.max(Fixed::ZERO) + our_size.max(Fixed::ZERO);
        if effective_queue.raw() <= 0 {
            return PPM;
        }
        let horizon = Fixed::from_raw(horizon_s.max(0) * SCALE);
        let traded = self.trade_rate.mul(horizon);
        let intensity = traded.div(effective_queue);
        fill_probability_ppm(intensity)
    }

    /// Expected time in seconds to fill an order of `our_size` at a level.
    pub fn expected_fill_time_s(&self, queue_ahead: Fixed, our_size: Fixed) -> Fixed {
        if self.trade_rate.raw() <= 0 {
            return Fixed::from_raw(i64::MAX);
        }
        (queue_ahead.max(Fixed::ZERO) + our_size.max(Fixed::ZERO)).div(self.trade_rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(value: f64) -> Fixed {
        Fixed::from_f64(value)
    }

    #[test]
    fn test_temporary_impact_increases_with_participation() {
        let params = ImpactParams::default();
        let small = params.temporary_millibps(fixed(1_000.0), fixed(1_000_000.0));
        let large = params.temporary_millibps(fixed(100_000.0), fixed(1_000_000.0));
        assert!(large > small);
        assert!(small >= 0);
    }

    #[test]
    fn test_temporary_impact_matches_analytic() {
        // C=10, vol=150bps => vol/100=1.5; participation 0.0001 => sqrt=0.01
        // impact = 10 * 1.5 * 0.01 = 0.15 bps = 150 milli-bps.
        let params = ImpactParams::default();
        let impact = params.temporary_millibps(fixed(100.0), fixed(1_000_000.0));
        assert!((impact - 150).abs() <= 1, "impact = {impact}");
    }

    #[test]
    fn test_zero_adv_is_safe() {
        let params = ImpactParams::default();
        assert_eq!(
            params.participation(fixed(1_000.0), Fixed::ZERO),
            Fixed::ZERO
        );
        assert_eq!(params.temporary_millibps(fixed(1_000.0), Fixed::ZERO), 0);
    }

    #[test]
    fn test_permanent_is_linear_in_participation() {
        let params = ImpactParams::default();
        let a = params.permanent_millibps(fixed(10_000.0), fixed(1_000_000.0));
        let b = params.permanent_millibps(fixed(20_000.0), fixed(1_000_000.0));
        assert!((b - 2 * a).abs() <= 1);
    }

    #[test]
    fn test_queue_fill_probability_bounds() {
        let queue = QueueModel::from_shares_per_second(100.0);
        assert_eq!(
            queue.fill_probability_ppm(Fixed::ZERO, Fixed::ZERO, 10),
            PPM
        );
        assert!(queue.fill_probability_ppm(fixed(10_000.0), fixed(1_000.0), 1) < PPM / 10);
        assert!(queue.fill_probability_ppm(fixed(100.0), fixed(100.0), 3_600) > 900_000);
    }

    #[test]
    fn test_expected_fill_time() {
        let queue = QueueModel::from_shares_per_second(100.0);
        assert!(
            (queue
                .expected_fill_time_s(fixed(500.0), fixed(500.0))
                .to_f64()
                - 10.0)
                .abs()
                < 1e-6
        );
    }
}
