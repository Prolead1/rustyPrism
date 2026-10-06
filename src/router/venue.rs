use super::fixed::{diff_millibps, Fixed, BPS_IN_MILLI, MILLI_BPS, PPM, SCALE_SQUARED_I128};
use super::symbol::SymbolId;
use crate::order::Side;

/// Compact identifier for a venue within a simulated topology.
pub type VenueId = usize;

/// A single resting price level in a venue's simulated order book.
///
/// `queue_ahead` captures the microstructure state at the level: how many
/// shares are queued in front of a newly arriving passive order. It is used by
/// the queue model to estimate the probability of a passive fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriceLevel {
    pub price: Fixed,
    pub size: Fixed,
    pub queue_ahead: Fixed,
}

impl PriceLevel {
    pub fn new(price: Fixed, size: Fixed) -> Self {
        PriceLevel {
            price,
            size,
            queue_ahead: Fixed::ZERO,
        }
    }

    pub fn with_queue(price: Fixed, size: Fixed, queue_ahead: Fixed) -> Self {
        PriceLevel {
            price,
            size,
            queue_ahead,
        }
    }

    pub fn from_f64(price: f64, size: f64) -> Self {
        PriceLevel::new(Fixed::from_f64(price), Fixed::from_f64(size))
    }
}

/// Result of sweeping a book for a given quantity.
///
/// `notional` is stored with `SCALE^2` implied (the sum of `price_raw *
/// qty_raw`); use [`BookWalk::notional_value`] for real currency and
/// [`BookWalk::vwap`] for the volume-weighted price.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BookWalk {
    pub requested_qty: Fixed,
    pub filled_qty: Fixed,
    pub notional: i128,
    pub worst_price: Fixed,
    pub levels_consumed: usize,
}

impl BookWalk {
    /// Volume weighted average execution price.
    pub fn vwap(&self) -> Fixed {
        if self.filled_qty.raw() > 0 {
            Fixed::from_raw((self.notional / self.filled_qty.raw() as i128) as i64)
        } else {
            Fixed::ZERO
        }
    }

    /// Traded notional in currency units.
    pub fn notional_value(&self) -> i128 {
        self.notional / SCALE_SQUARED_I128
    }

    /// Quantity that could not be filled against displayed liquidity.
    pub fn unfilled_qty(&self) -> Fixed {
        (self.requested_qty - self.filled_qty).max(Fixed::ZERO)
    }

    /// Signed slippage versus a reference price, in milli-basis-points.
    ///
    /// Positive always means "worse than the reference" for the given side.
    pub fn slippage_millibps(&self, side: Side, reference_price: Fixed) -> i64 {
        if self.filled_qty.raw() <= 0 || reference_price.raw() <= 0 {
            return 0;
        }
        let raw = diff_millibps(self.vwap(), reference_price);
        match side {
            Side::Buy => raw,
            Side::Sell => -raw,
        }
    }
}

/// Maker/taker fees expressed in milli-basis-points.
///
/// Positive values are a cost to the trader, negative values are a rebate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FeeSchedule {
    pub maker_millibps: i64,
    pub taker_millibps: i64,
}

impl FeeSchedule {
    pub const fn from_millibps(maker_millibps: i64, taker_millibps: i64) -> Self {
        FeeSchedule {
            maker_millibps,
            taker_millibps,
        }
    }

    /// Build from basis points, e.g. `2.75` bps -> `2750` milli-bps.
    pub fn from_bps(maker_bps: f64, taker_bps: f64) -> Self {
        FeeSchedule {
            maker_millibps: (maker_bps * MILLI_BPS as f64).round() as i64,
            taker_millibps: (taker_bps * MILLI_BPS as f64).round() as i64,
        }
    }
}

/// One tier of a venue's volume-based fee schedule.
///
/// A tier applies while the routed monthly volume is at least
/// `min_monthly_volume`. Tiers are evaluated highest-threshold-first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeTier {
    pub min_monthly_volume: i128,
    pub fees: FeeSchedule,
}

impl FeeTier {
    pub const fn new(min_monthly_volume: i128, fees: FeeSchedule) -> Self {
        FeeTier {
            min_monthly_volume,
            fees,
        }
    }

    pub fn from_bps(min_monthly_volume: f64, maker_bps: f64, taker_bps: f64) -> Self {
        FeeTier {
            min_monthly_volume: min_monthly_volume.round() as i128,
            fees: FeeSchedule::from_bps(maker_bps, taker_bps),
        }
    }
}

/// A venue's simulated order book for a single symbol.
///
/// Bids are stored best (highest) first and asks best (lowest) first, so that
/// a sweep is a simple forward iteration.
#[derive(Debug, Clone, Default)]
pub struct VenueBook {
    pub bids: Vec<PriceLevel>,
    pub asks: Vec<PriceLevel>,
}

impl VenueBook {
    pub fn new() -> Self {
        VenueBook {
            bids: Vec::new(),
            asks: Vec::new(),
        }
    }

    /// Build a book from arbitrary level collections, sorting to best-first.
    pub fn from_levels(mut bids: Vec<PriceLevel>, mut asks: Vec<PriceLevel>) -> Self {
        bids.sort_by_key(|level| std::cmp::Reverse(level.price));
        asks.sort_by_key(|level| level.price);
        VenueBook { bids, asks }
    }

    pub fn best_bid(&self) -> Option<Fixed> {
        self.bids.first().map(|level| level.price)
    }

    pub fn best_ask(&self) -> Option<Fixed> {
        self.asks.first().map(|level| level.price)
    }

    pub fn mid_price(&self) -> Option<Fixed> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some(Fixed::from_raw(
                ((bid.raw() as i128 + ask.raw() as i128) / 2) as i64,
            )),
            (Some(bid), None) => Some(bid),
            (None, Some(ask)) => Some(ask),
            (None, None) => None,
        }
    }

    pub fn spread_millibps(&self) -> i64 {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) if bid.raw() > 0 => diff_millibps(ask, bid),
            _ => 0,
        }
    }

    /// Levels that an aggressive order of `side` would consume, best first.
    fn sweep_side(&self, side: Side) -> &[PriceLevel] {
        match side {
            Side::Buy => &self.asks,
            Side::Sell => &self.bids,
        }
    }

    /// Simulate (without mutating) sweeping `qty` from the book.
    pub fn walk(&self, side: Side, qty: Fixed) -> BookWalk {
        self.walk_with_limit(side, qty, None)
    }

    /// Simulate a sweep that respects an optional limit price.
    pub fn walk_with_limit(&self, side: Side, qty: Fixed, limit_price: Option<Fixed>) -> BookWalk {
        let mut result = BookWalk {
            requested_qty: qty,
            ..BookWalk::default()
        };
        let mut remaining = qty;
        for level in self.sweep_side(side) {
            if remaining.raw() <= 0 {
                break;
            }
            if let Some(limit) = limit_price {
                if !Self::is_marketable(side, level.price, limit) {
                    break;
                }
            }
            let take = remaining.min(level.size);
            result.filled_qty += take;
            result.notional += level.price.raw() as i128 * take.raw() as i128;
            result.worst_price = level.price;
            result.levels_consumed += 1;
            remaining -= take;
        }
        result
    }

    /// Mutating sweep used when simulating an actual execution. Consumes
    /// liquidity from the book, removing empty levels.
    pub fn consume(&mut self, side: Side, qty: Fixed, limit_price: Option<Fixed>) -> BookWalk {
        let mut result = BookWalk {
            requested_qty: qty,
            ..BookWalk::default()
        };
        let mut remaining = qty;
        let levels = match side {
            Side::Buy => &mut self.asks,
            Side::Sell => &mut self.bids,
        };
        let mut index = 0;
        while index < levels.len() && remaining.raw() > 0 {
            if let Some(limit) = limit_price {
                if !Self::is_marketable(side, levels[index].price, limit) {
                    break;
                }
            }
            let take = remaining.min(levels[index].size);
            result.filled_qty += take;
            result.notional += levels[index].price.raw() as i128 * take.raw() as i128;
            result.worst_price = levels[index].price;
            result.levels_consumed += 1;
            remaining -= take;
            levels[index].size -= take;
            if levels[index].size.raw() <= 0 {
                levels.remove(index);
            } else {
                index += 1;
            }
        }
        result
    }

    /// A buy is marketable against an ask when ask <= limit.
    /// A sell is marketable against a bid when bid >= limit.
    fn is_marketable(side: Side, level_price: Fixed, limit_price: Fixed) -> bool {
        match side {
            Side::Buy => level_price <= limit_price,
            Side::Sell => level_price >= limit_price,
        }
    }

    /// Total displayed liquidity within `band_bps` of `reference_price` on the
    /// side that an order of `side` would trade against. This is the
    /// "implied liquidity" available to the order.
    pub fn implied_liquidity(&self, side: Side, band_bps: i64, reference_price: Fixed) -> Fixed {
        if reference_price.raw() <= 0 {
            return Fixed::ZERO;
        }
        let offset = reference_price.apply_bps(band_bps);
        self.implied_liquidity_bounds(side, reference_price - offset, reference_price + offset)
    }

    /// Implied liquidity using pre-computed band bounds.
    ///
    /// The bounds are constant for a whole order, so callers can compute them
    /// once instead of running a basis-point division per venue.
    #[inline]
    pub fn implied_liquidity_bounds(&self, side: Side, lower: Fixed, upper: Fixed) -> Fixed {
        match side {
            Side::Buy => self
                .asks
                .iter()
                .filter(|level| level.price <= upper)
                .fold(Fixed::ZERO, |acc, level| acc + level.size),
            Side::Sell => self
                .bids
                .iter()
                .filter(|level| level.price >= lower)
                .fold(Fixed::ZERO, |acc, level| acc + level.size),
        }
    }

    /// Aggregate queue depth resting ahead at the touch on the given side.
    pub fn queue_ahead_at_touch(&self, side: Side) -> Fixed {
        match self.sweep_side(side).first() {
            Some(level) => level.queue_ahead + level.size,
            None => Fixed::ZERO,
        }
    }
}

/// A venue in the simulated multi-venue topology.
#[derive(Debug, Clone)]
pub struct Venue {
    pub id: VenueId,
    pub name: String,
    /// Base fee schedule used when no volume tier is reached.
    pub base_fees: FeeSchedule,
    /// Volume-based fee tiers, in ascending threshold order.
    pub fee_tiers: Vec<FeeTier>,
    /// One-way latency in microseconds.
    pub latency_us: u32,
    /// Historical fill probability in parts-per-million.
    pub fill_probability_ppm: i64,
    /// Rolling routed monthly volume used to select a fee tier.
    pub monthly_volume: i128,
    /// Simulated books indexed by interned [`SymbolId`].
    pub books: Vec<Option<VenueBook>>,
}

impl Venue {
    pub fn new(id: VenueId, name: &str, latency_us: u32, fill_probability: f64) -> Self {
        Venue {
            id,
            name: name.to_string(),
            base_fees: FeeSchedule::default(),
            fee_tiers: Vec::new(),
            latency_us,
            fill_probability_ppm: ((fill_probability.clamp(0.0, 1.0)) * PPM as f64).round() as i64,
            monthly_volume: 0,
            books: Vec::new(),
        }
    }

    pub fn with_fees(mut self, maker_bps: f64, taker_bps: f64) -> Self {
        self.base_fees = FeeSchedule::from_bps(maker_bps, taker_bps);
        self
    }

    pub fn with_fee_schedule(mut self, fees: FeeSchedule) -> Self {
        self.base_fees = fees;
        self
    }

    pub fn with_fee_tiers(mut self, mut tiers: Vec<FeeTier>) -> Self {
        tiers.sort_by_key(|a| a.min_monthly_volume);
        self.fee_tiers = tiers;
        self
    }

    fn ensure_books(&mut self, symbol: SymbolId) {
        let index = symbol as usize;
        if self.books.len() <= index {
            self.books.resize_with(index + 1, || None);
        }
    }

    pub fn with_book(mut self, symbol: SymbolId, book: VenueBook) -> Self {
        self.set_book(symbol, book);
        self
    }

    /// Replace or insert a symbol's book in place.
    pub fn set_book(&mut self, symbol: SymbolId, book: VenueBook) {
        self.ensure_books(symbol);
        self.books[symbol as usize] = Some(book);
    }

    pub fn book(&self, symbol: SymbolId) -> Option<&VenueBook> {
        self.books
            .get(symbol as usize)
            .and_then(|book| book.as_ref())
    }

    pub fn book_mut(&mut self, symbol: SymbolId) -> Option<&mut VenueBook> {
        self.books
            .get_mut(symbol as usize)
            .and_then(|book| book.as_mut())
    }

    /// Select the fee schedule for the venue's current routed volume.
    pub fn effective_fees(&self) -> FeeSchedule {
        self.effective_fees_at(self.monthly_volume)
    }

    pub fn effective_fees_at(&self, monthly_volume: i128) -> FeeSchedule {
        let mut chosen = self.base_fees;
        for tier in &self.fee_tiers {
            if monthly_volume >= tier.min_monthly_volume {
                chosen = tier.fees;
            } else {
                break;
            }
        }
        chosen
    }
}

/// Fee amount in currency units for `notional` at `fee_millibps`.
pub fn fee_amount(notional: i128, fee_millibps: i64) -> i128 {
    notional * fee_millibps as i128 / BPS_IN_MILLI as i128
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_book() -> VenueBook {
        VenueBook::from_levels(
            vec![
                PriceLevel::from_f64(99.99, 500.0),
                PriceLevel::from_f64(99.98, 800.0),
                PriceLevel::from_f64(99.95, 2_000.0),
            ],
            vec![
                PriceLevel::from_f64(100.01, 400.0),
                PriceLevel::from_f64(100.02, 700.0),
                PriceLevel::from_f64(100.05, 1_500.0),
            ],
        )
    }

    #[test]
    fn test_best_prices_and_mid() {
        let book = sample_book();
        assert_eq!(book.best_bid(), Some(Fixed::from_f64(99.99)));
        assert_eq!(book.best_ask(), Some(Fixed::from_f64(100.01)));
        assert_eq!(book.mid_price(), Some(Fixed::from_f64(100.0)));
        assert!(book.spread_millibps() > 0);
    }

    #[test]
    fn test_walk_buy_vwap() {
        let book = sample_book();
        // 400 @ 100.01 + 600 @ 100.02 = 100,016 / 1,000 = 100.016
        let walk = book.walk(Side::Buy, Fixed::from_f64(1_000.0));
        assert_eq!(walk.filled_qty, Fixed::from_f64(1_000.0));
        assert_eq!(walk.vwap(), Fixed::from_f64(100.016));
        assert_eq!(walk.worst_price, Fixed::from_f64(100.02));
        assert_eq!(walk.levels_consumed, 2);
    }

    #[test]
    fn test_notional_value_units() {
        let book = sample_book();
        let walk = book.walk(Side::Buy, Fixed::from_f64(1_000.0));
        // 400 @ 100.01 + 600 @ 100.02 = 100,016.0 in currency units.
        assert_eq!(walk.notional_value(), 100_016);
    }

    #[test]
    fn test_walk_sell_slippage_sign() {
        let book = sample_book();
        let walk = book.walk(Side::Sell, Fixed::from_f64(800.0));
        assert!(walk.vwap() < Fixed::from_f64(100.0));
        assert!(walk.slippage_millibps(Side::Sell, Fixed::from_f64(100.0)) > 0);
    }

    #[test]
    fn test_walk_partial_fill() {
        let book = sample_book();
        let walk = book.walk(Side::Buy, Fixed::from_f64(10_000.0));
        let depth: Fixed = book.asks.iter().fold(Fixed::ZERO, |acc, l| acc + l.size);
        assert_eq!(walk.filled_qty, depth);
        assert!(walk.unfilled_qty().is_positive());
    }

    #[test]
    fn test_walk_respects_limit() {
        let book = sample_book();
        let walk = book.walk_with_limit(
            Side::Buy,
            Fixed::from_f64(1_000.0),
            Some(Fixed::from_f64(100.01)),
        );
        assert_eq!(walk.filled_qty, Fixed::from_f64(400.0));
        assert_eq!(walk.levels_consumed, 1);
    }

    #[test]
    fn test_consume_removes_liquidity() {
        let mut book = sample_book();
        let before: Fixed = book.asks.iter().fold(Fixed::ZERO, |acc, l| acc + l.size);
        let walk = book.consume(Side::Buy, Fixed::from_f64(500.0), None);
        assert_eq!(walk.filled_qty, Fixed::from_f64(500.0));
        let after: Fixed = book.asks.iter().fold(Fixed::ZERO, |acc, l| acc + l.size);
        assert_eq!(before - after, Fixed::from_f64(500.0));
        assert_eq!(book.best_ask(), Some(Fixed::from_f64(100.02)));
    }

    #[test]
    fn test_implied_liquidity_band() {
        let book = sample_book();
        let liq = book.implied_liquidity(Side::Buy, 1, Fixed::from_f64(100.0));
        assert_eq!(liq, Fixed::from_f64(400.0));
        let wide = book.implied_liquidity(Side::Buy, 10, Fixed::from_f64(100.0));
        assert!(wide > liq);
    }

    #[test]
    fn test_fee_tier_selection() {
        let venue = Venue::new(0, "XNAS", 20, 0.95).with_fee_tiers(vec![
            FeeTier::from_bps(0.0, 1.0, 3.0),
            FeeTier::from_bps(1_000_000.0, 0.5, 2.5),
            FeeTier::from_bps(5_000_000.0, 0.0, 2.0),
        ]);
        assert_eq!(venue.effective_fees_at(0).taker_millibps, 3_000);
        assert_eq!(venue.effective_fees_at(2_000_000).taker_millibps, 2_500);
        assert_eq!(venue.effective_fees_at(9_000_000).taker_millibps, 2_000);
        assert_eq!(venue.effective_fees_at(9_000_000).maker_millibps, 0);
    }

    #[test]
    fn test_fee_amount() {
        // 1,000,000 notional at 2.5 bps = 250.
        assert_eq!(fee_amount(1_000_000, 2_500), 250);
    }

    #[test]
    fn test_books_indexed_by_symbol() {
        let venue = Venue::new(0, "X", 10, 1.0).with_book(3, sample_book());
        assert!(venue.book(3).is_some());
        assert!(venue.book(0).is_none());
        assert!(venue.book(99).is_none());
    }
}
