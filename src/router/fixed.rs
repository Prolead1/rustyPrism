//! Fixed-point arithmetic for the router hot path.
//!
//! All prices, quantities and notional values are represented as scaled `i64`
//! (or `i128` for accumulations). This keeps the innermost loop — sweeping book
//! levels and summing notional — free of floating point, which makes decisions
//! deterministic and bit-for-bit reproducible across runs and platforms.
//!
//! Scales used throughout:
//!
//! | quantity | scale | meaning |
//! | --- | --- | --- |
//! | [`SCALE`] | 10,000 | price and quantity, 4 decimal places |
//! | [`BPS`] | 10,000 | basis points in a whole |
//! | [`MILLI_BPS`] | 1,000 | milli-basis-points per basis point |
//! | [`PPM`] | 1,000,000 | parts-per-million for probabilities / coverage |

/// Price and quantity scale (4 decimal places).
pub const SCALE: i64 = 10_000;
/// `SCALE` as an `i128` for products that need it.
pub const SCALE_I128: i128 = SCALE as i128;
/// `SCALE^2`, used to convert a price*qty product back to currency units.
pub const SCALE_SQUARED_I128: i128 = SCALE_I128 * SCALE_I128;
/// Basis points in a whole.
pub const BPS: i64 = 10_000;
/// Milli-basis-points per basis point (3 extra decimal places of precision).
pub const MILLI_BPS: i64 = 1_000;
/// Parts per million (used for probabilities and coverage).
pub const PPM: i64 = 1_000_000;
/// One basis point expressed in milli-basis-points.
pub const BPS_IN_MILLI: i64 = BPS * MILLI_BPS;

/// A fixed-point number with [`SCALE`] decimal places.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Fixed(i64);

impl Fixed {
    pub const ZERO: Fixed = Fixed(0);
    pub const ONE: Fixed = Fixed(SCALE);

    pub const fn from_raw(raw: i64) -> Self {
        Fixed(raw)
    }

    pub const fn raw(self) -> i64 {
        self.0
    }

    pub fn from_f64(value: f64) -> Self {
        Fixed((value * SCALE as f64).round() as i64)
    }

    pub fn to_f64(self) -> f64 {
        self.0 as f64 / SCALE as f64
    }

    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    pub const fn min(self, other: Self) -> Self {
        if self.0 < other.0 {
            self
        } else {
            other
        }
    }

    pub const fn max(self, other: Self) -> Self {
        if self.0 > other.0 {
            self
        } else {
            other
        }
    }

    /// Fixed-point multiply: `(a * b) / SCALE`.
    pub fn mul(self, rhs: Self) -> Self {
        Fixed(((self.0 as i128 * rhs.0 as i128) / SCALE_I128) as i64)
    }

    /// Fixed-point divide: `(a * SCALE) / b`.
    pub fn div(self, rhs: Self) -> Self {
        if rhs.0 == 0 {
            return Fixed::ZERO;
        }
        Fixed((self.0 as i128 * SCALE_I128 / rhs.0 as i128) as i64)
    }

    /// Ratio `self / rhs` expressed in parts-per-million.
    pub fn ratio_ppm(self, rhs: Self) -> i64 {
        if rhs.0 == 0 {
            return 0;
        }
        (self.0 * PPM) / rhs.0
    }

    /// Apply a basis-point adjustment: `self * bps / 10_000`.
    pub fn apply_bps(self, bps: i64) -> Self {
        Fixed((self.0 * bps) / BPS)
    }

    /// Clamp to the inclusive `[lo, hi]` range.
    pub fn clamp(self, lo: Fixed, hi: Fixed) -> Self {
        if self.0 < lo.0 {
            lo
        } else if self.0 > hi.0 {
            hi
        } else {
            self
        }
    }
}

impl std::ops::Add for Fixed {
    type Output = Fixed;
    fn add(self, rhs: Fixed) -> Fixed {
        Fixed(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Fixed {
    type Output = Fixed;
    fn sub(self, rhs: Fixed) -> Fixed {
        Fixed(self.0 - rhs.0)
    }
}

impl std::ops::Neg for Fixed {
    type Output = Fixed;
    fn neg(self) -> Fixed {
        Fixed(-self.0)
    }
}

impl std::ops::AddAssign for Fixed {
    fn add_assign(&mut self, rhs: Fixed) {
        self.0 += rhs.0;
    }
}

impl std::ops::SubAssign for Fixed {
    fn sub_assign(&mut self, rhs: Fixed) {
        self.0 -= rhs.0;
    }
}

impl std::fmt::Display for Fixed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.4}", self.to_f64())
    }
}

/// Signed difference `value - reference` expressed in milli-basis-points.
pub fn diff_millibps(value: Fixed, reference: Fixed) -> i64 {
    if reference.0 == 0 {
        return 0;
    }
    ((value.0 - reference.0) * BPS_IN_MILLI) / reference.0
}

/// Fixed-point square root of a non-negative dimensionless `Fixed`.
pub fn sqrt_fixed(value: Fixed) -> Fixed {
    if value.0 <= 0 {
        return Fixed::ZERO;
    }
    Fixed(isqrt_u64(value.0 as u64 * SCALE as u64) as i64)
}

/// Integer square root for `u64`.
pub fn isqrt_u64(value: u64) -> u64 {
    if value < 2 {
        return value;
    }
    let mut x = value;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + value / x) / 2;
    }
    x
}

/// Saturating Poisson-style fill probability in parts-per-million.
///
/// Uses the monotone rational approximation `P = x / (1 + x)` for `1 - e^-x`
/// so the model stays entirely in integer arithmetic. It matches the
/// exponential for small `x` and saturates to one as `x` grows.
pub fn fill_probability_ppm(intensity: Fixed) -> i64 {
    let intensity = intensity.raw().max(0) as i128;
    ((intensity * PPM as i128) / (SCALE_I128 + intensity)) as i64
}

/// Probability that a quote is still valid after `latency_us` of travel, in ppm.
///
/// Same rational approximation as [`fill_probability_ppm`] applied to
/// `1 / (1 + latency / decay)`.
pub fn quote_validity_ppm(latency_us: u32, decay_us: i64) -> i64 {
    if decay_us <= 0 {
        return PPM;
    }
    let decay = decay_us as i128;
    let latency = latency_us as i128;
    ((decay * PPM as i128) / (decay + latency)) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_to_f64_roundtrip() {
        for value in [0.0, 1.0, 99.99, 100.005, 1.2345, 1_000_000.0] {
            let round_trip = Fixed::from_f64(value).to_f64();
            assert!((round_trip - value).abs() < 1e-9, "{value} -> {round_trip}");
        }
    }

    #[test]
    fn test_add_sub_neg() {
        let a = Fixed::from_f64(10.5);
        let b = Fixed::from_f64(2.25);
        assert_eq!((a + b).to_f64(), 12.75);
        assert_eq!((a - b).to_f64(), 8.25);
        assert_eq!((-b).to_f64(), -2.25);
    }

    #[test]
    fn test_mul_div() {
        let price = Fixed::from_f64(100.0);
        let qty = Fixed::from_f64(3.5);
        assert_eq!(price.mul(qty).to_f64(), 350.0);
        assert!((price.div(qty).to_f64() - 100.0 / 3.5).abs() < 1e-4);
    }

    #[test]
    fn test_ratio_ppm() {
        assert_eq!(
            Fixed::from_f64(200.0).ratio_ppm(Fixed::from_f64(100.0)),
            2 * PPM
        );
        assert_eq!(
            Fixed::from_f64(50.0).ratio_ppm(Fixed::from_f64(100.0)),
            PPM / 2
        );
        assert_eq!(Fixed::from_f64(1.0).ratio_ppm(Fixed::ZERO), 0);
    }

    #[test]
    fn test_apply_bps() {
        assert_eq!(Fixed::from_f64(100.0).apply_bps(10).to_f64(), 0.1);
        assert_eq!(Fixed::from_f64(100.0).apply_bps(10_000).to_f64(), 100.0);
    }

    #[test]
    fn test_diff_millibps() {
        // 1 bps move on 100.00 is 1000 milli-bps.
        let value = Fixed::from_f64(100.01);
        let reference = Fixed::from_f64(100.0);
        assert!((diff_millibps(value, reference) - MILLI_BPS).abs() <= 1);
        let forward = diff_millibps(value, reference);
        let backward = diff_millibps(reference, value);
        assert!((forward + backward).abs() <= 1);
    }

    #[test]
    fn test_sqrt_fixed() {
        assert_eq!(sqrt_fixed(Fixed::ZERO), Fixed::ZERO);
        assert_eq!(sqrt_fixed(Fixed::from_f64(4.0)), Fixed::from_f64(2.0));
        // sqrt(0.0004) = 0.02
        assert_eq!(sqrt_fixed(Fixed::from_f64(0.0004)), Fixed::from_f64(0.02));
    }

    #[test]
    fn test_isqrt() {
        assert_eq!(isqrt_u64(0), 0);
        assert_eq!(isqrt_u64(1), 1);
        assert_eq!(isqrt_u64(15), 3);
        assert_eq!(isqrt_u64(16), 4);
        assert_eq!(isqrt_u64(1_000_000), 1_000);
    }

    #[test]
    fn test_fill_probability_monotone_and_bounded() {
        assert_eq!(fill_probability_ppm(Fixed::ZERO), 0);
        let mut previous = 0;
        for raw in [10, 100, 1_000, 10_000, 100_000, 1_000_000] {
            let p = fill_probability_ppm(Fixed::from_raw(raw));
            assert!(p >= previous && p <= PPM);
            previous = p;
        }
        // Large intensity saturates near 1.
        assert!(fill_probability_ppm(Fixed::from_raw(1_000_000_000)) > 999_000);
    }

    #[test]
    fn test_quote_validity_monotone() {
        assert_eq!(quote_validity_ppm(0, 500), PPM);
        let fast = quote_validity_ppm(100, 500);
        let slow = quote_validity_ppm(1_000, 500);
        assert!(fast < PPM && slow < fast && slow > 0);
    }
}
