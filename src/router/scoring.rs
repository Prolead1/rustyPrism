use super::fixed::{diff_millibps, quote_validity_ppm, Fixed, MILLI_BPS, PPM};
use super::impact::{ImpactParams, QueueModel};
use super::symbol::SymbolId;
use super::venue::{Venue, VenueId};
use crate::order::Side;

/// Relative importance of each cost component when scoring a destination.
///
/// All weights are integers where `MILLI_BPS` (1000) represents a multiplier of
/// `1.0`. The scorer converts every component into milli-basis-points of
/// expected cost and forms a weighted sum. A destination's `score` is the
/// negated cost, so higher is always better.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoreWeights {
    /// Weight on expected slippage versus the reference price.
    pub slippage: i64,
    /// Weight on net fees (taker is a cost, maker rebate is a benefit).
    pub fees: i64,
    /// Weight on expected market impact.
    pub impact: i64,
    /// Cost in milli-bps charged per microsecond of venue latency.
    pub latency: i64,
    /// Penalty in milli-bps applied when fill probability is zero.
    pub fill_probability: i64,
    /// Reward in milli-bps for covering the full order from displayed liquidity.
    pub liquidity: i64,
}

impl Default for ScoreWeights {
    fn default() -> Self {
        ScoreWeights {
            slippage: MILLI_BPS,
            fees: MILLI_BPS,
            impact: MILLI_BPS,
            // 1 milli-bps per microsecond == 1 bp per millisecond.
            latency: 1,
            fill_probability: 5 * MILLI_BPS,
            liquidity: MILLI_BPS / 2,
        }
    }
}

/// Static, immutable inputs shared by every venue score in a routing pass.
pub struct ScoringContext<'a> {
    pub weights: &'a ScoreWeights,
    pub impact: &'a ImpactParams,
    pub queue: &'a QueueModel,
    /// Band around the reference price that counts as "implied liquidity".
    pub liquidity_band_bps: i64,
    /// Latency at which a quote has a 1/e chance of being stale.
    pub latency_decay_us: i64,
}

/// Per-destination score for a single order. All cost values are signed
/// integer milli-basis-points where positive means a cost to the trader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VenueScore {
    pub venue_id: VenueId,
    pub requested_qty: Fixed,
    pub filled_qty: Fixed,
    pub expected_vwap: Fixed,
    pub slippage_millibps: i64,
    pub net_fee_millibps: i64,
    pub impact_millibps: i64,
    pub latency_us: u32,
    /// Probability that the order (or its marketable portion) fills, in ppm.
    pub fill_probability_ppm: i64,
    /// Probability of a passive fill, driven by queue position and trade rate.
    pub queue_fill_probability_ppm: i64,
    /// Total displayed liquidity available within the band.
    pub liquidity_available: Fixed,
    /// Fraction of the requested quantity covered by displayed liquidity, in ppm.
    pub liquidity_coverage_ppm: i64,
    /// Cached weighted score (higher is better).
    pub score: i64,
}

impl VenueScore {
    /// Weighted expected cost in milli-basis-points (lower is better).
    pub fn total_cost_millibps(&self, weights: &ScoreWeights) -> i64 {
        (self.slippage_millibps * weights.slippage) / MILLI_BPS
            + (self.net_fee_millibps * weights.fees) / MILLI_BPS
            + (self.impact_millibps * weights.impact) / MILLI_BPS
            + self.latency_us as i64 * weights.latency
            + (PPM - self.fill_probability_ppm) * weights.fill_probability / PPM
            - self.liquidity_coverage_ppm * weights.liquidity / PPM
    }
}

/// Score a single venue for an order.
///
/// For marketable orders the book is swept and slippage, impact and depth are
/// measured directly. For passive orders the expected fill is at the limit and
/// the fill probability is derived from queue dynamics.
#[allow(clippy::too_many_arguments)]
pub fn score_venue(
    venue: &Venue,
    symbol: SymbolId,
    side: Side,
    qty: Fixed,
    reference_price: Fixed,
    adv: Fixed,
    horizon_s: i64,
    limit_price: Option<Fixed>,
    context: &ScoringContext<'_>,
) -> VenueScore {
    let book = match venue.book(symbol) {
        Some(book) => book,
        None => {
            // No book: the venue cannot be scored. Return a maximally
            // unattractive score so it sorts last.
            return VenueScore {
                venue_id: venue.id,
                requested_qty: qty,
                filled_qty: Fixed::ZERO,
                expected_vwap: Fixed::ZERO,
                slippage_millibps: 0,
                net_fee_millibps: venue.effective_fees().taker_millibps,
                impact_millibps: 0,
                latency_us: venue.latency_us,
                fill_probability_ppm: 0,
                queue_fill_probability_ppm: 0,
                liquidity_available: Fixed::ZERO,
                liquidity_coverage_ppm: 0,
                score: i64::MIN,
            };
        }
    };

    let marketable = match limit_price {
        None => true,
        Some(limit) => match side {
            Side::Buy => book.best_ask().map(|ask| limit >= ask).unwrap_or(false),
            Side::Sell => book.best_bid().map(|bid| limit <= bid).unwrap_or(false),
        },
    };

    let liquidity_available =
        book.implied_liquidity(side, context.liquidity_band_bps, reference_price);
    let liquidity_coverage_ppm = if qty.raw() > 0 {
        liquidity_available.ratio_ppm(qty).clamp(0, PPM)
    } else {
        0
    };
    let validity_ppm = quote_validity_ppm(venue.latency_us, context.latency_decay_us);

    let mut score = if marketable {
        let walk = book.walk_with_limit(side, qty, limit_price);
        let slippage_millibps = walk.slippage_millibps(side, reference_price);
        let net_fee_millibps = venue.effective_fees().taker_millibps;
        let impact_millibps = context.impact.temporary_millibps(walk.filled_qty, adv);
        let depth_coverage_ppm = if qty.raw() > 0 {
            walk.filled_qty.ratio_ppm(qty).clamp(0, PPM)
        } else {
            PPM
        };
        let fill_probability_ppm =
            (venue.fill_probability_ppm * depth_coverage_ppm / PPM * validity_ppm / PPM)
                .clamp(0, PPM);

        VenueScore {
            venue_id: venue.id,
            requested_qty: qty,
            filled_qty: walk.filled_qty,
            expected_vwap: walk.vwap(),
            slippage_millibps,
            net_fee_millibps,
            impact_millibps,
            latency_us: venue.latency_us,
            fill_probability_ppm,
            queue_fill_probability_ppm: PPM,
            liquidity_available,
            liquidity_coverage_ppm,
            score: 0,
        }
    } else {
        let limit = limit_price.unwrap_or(reference_price);
        let raw = diff_millibps(limit, reference_price);
        let slippage_millibps = match side {
            Side::Buy => raw,
            Side::Sell => -raw,
        };
        let net_fee_millibps = venue.effective_fees().maker_millibps;
        let queue_ahead = book.queue_ahead_at_touch(side);
        let queue_fill_probability_ppm =
            (context
                .queue
                .fill_probability_ppm(queue_ahead, qty, horizon_s)
                * venue.fill_probability_ppm
                / PPM
                * validity_ppm
                / PPM)
                .clamp(0, PPM);

        VenueScore {
            venue_id: venue.id,
            requested_qty: qty,
            filled_qty: Fixed::ZERO,
            expected_vwap: limit,
            slippage_millibps,
            net_fee_millibps,
            // Passive orders do not pay a taker-style temporary impact.
            impact_millibps: 0,
            latency_us: venue.latency_us,
            fill_probability_ppm: queue_fill_probability_ppm,
            queue_fill_probability_ppm,
            liquidity_available,
            liquidity_coverage_ppm,
            score: 0,
        }
    };

    score.score = -score.total_cost_millibps(context.weights);
    score
}

/// Sort scores best-first: highest score, then lowest latency, then venue id.
pub fn rank_scores(scores: &mut [VenueScore]) {
    scores.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(a.latency_us.cmp(&b.latency_us))
            .then(a.venue_id.cmp(&b.venue_id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::fixed::Fixed;
    use crate::router::venue::{FeeTier, PriceLevel, Venue, VenueBook};

    fn fixed(value: f64) -> Fixed {
        Fixed::from_f64(value)
    }

    fn book(ask_off: f64, ask_size: f64) -> VenueBook {
        VenueBook::from_levels(
            vec![PriceLevel::from_f64(100.0 - ask_off - 0.01, ask_size)],
            vec![PriceLevel::from_f64(100.0 + ask_off, ask_size)],
        )
    }

    fn context<'a>(
        weights: &'a ScoreWeights,
        impact: &'a ImpactParams,
        queue: &'a QueueModel,
    ) -> ScoringContext<'a> {
        ScoringContext {
            weights,
            impact,
            queue,
            liquidity_band_bps: 20,
            latency_decay_us: 500,
        }
    }

    #[test]
    fn test_cheaper_venue_scores_higher() {
        let weights = ScoreWeights::default();
        let impact = ImpactParams::default();
        let queue = QueueModel::default();
        let ctx = context(&weights, &impact, &queue);

        let cheap = Venue::new(0, "CHEAP", 10, 0.99)
            .with_fees(0.0, 1.0)
            .with_book(0, book(0.01, 10_000.0));
        let expensive = Venue::new(1, "EXP", 10, 0.99)
            .with_fees(0.0, 5.0)
            .with_book(0, book(0.01, 10_000.0));

        let a = score_venue(
            &cheap,
            0,
            Side::Buy,
            fixed(100.0),
            fixed(100.0),
            fixed(1e6),
            30,
            None,
            &ctx,
        );
        let b = score_venue(
            &expensive,
            0,
            Side::Buy,
            fixed(100.0),
            fixed(100.0),
            fixed(1e6),
            30,
            None,
            &ctx,
        );
        assert!(a.score > b.score);
    }

    #[test]
    fn test_fill_probability_feeds_score() {
        let weights = ScoreWeights::default();
        let impact = ImpactParams::default();
        let queue = QueueModel::default();
        let ctx = context(&weights, &impact, &queue);

        let reliable = Venue::new(0, "A", 10, 1.0).with_book(0, book(0.01, 10_000.0));
        let flaky = Venue::new(1, "B", 10, 0.1).with_book(0, book(0.01, 10_000.0));

        let a = score_venue(
            &reliable,
            0,
            Side::Buy,
            fixed(100.0),
            fixed(100.0),
            fixed(1e6),
            30,
            None,
            &ctx,
        );
        let b = score_venue(
            &flaky,
            0,
            Side::Buy,
            fixed(100.0),
            fixed(100.0),
            fixed(1e6),
            30,
            None,
            &ctx,
        );
        assert!(a.fill_probability_ppm > b.fill_probability_ppm);
        assert!(a.score > b.score);
    }

    #[test]
    fn test_latency_penalised() {
        let weights = ScoreWeights::default();
        let impact = ImpactParams::default();
        let queue = QueueModel::default();
        let ctx = context(&weights, &impact, &queue);

        let fast = Venue::new(0, "FAST", 5, 1.0).with_book(0, book(0.01, 10_000.0));
        let slow = Venue::new(1, "SLOW", 5_000, 1.0).with_book(0, book(0.01, 10_000.0));

        let a = score_venue(
            &fast,
            0,
            Side::Buy,
            fixed(100.0),
            fixed(100.0),
            fixed(1e6),
            30,
            None,
            &ctx,
        );
        let b = score_venue(
            &slow,
            0,
            Side::Buy,
            fixed(100.0),
            fixed(100.0),
            fixed(1e6),
            30,
            None,
            &ctx,
        );
        assert!(a.score > b.score);
        assert!(b.fill_probability_ppm < a.fill_probability_ppm);
    }

    #[test]
    fn test_passive_uses_maker_fee_and_queue() {
        let weights = ScoreWeights::default();
        let impact = ImpactParams::default();
        let queue = QueueModel::from_shares_per_second(100.0);
        let ctx = context(&weights, &impact, &queue);

        let venue = Venue::new(0, "MAKER", 10, 1.0)
            .with_fees(-0.5, 3.0)
            .with_book(0, book(0.05, 5_000.0));
        // Buy limit below the ask -> passive.
        let s = score_venue(
            &venue,
            0,
            Side::Buy,
            fixed(100.0),
            fixed(100.0),
            fixed(1e6),
            5,
            Some(fixed(99.99)),
            &ctx,
        );
        assert_eq!(s.net_fee_millibps, -500);
        assert!(s.queue_fill_probability_ppm > 0);
        assert!(s.fill_probability_ppm < PPM);
    }

    #[test]
    fn test_missing_book_scores_min() {
        let weights = ScoreWeights::default();
        let impact = ImpactParams::default();
        let queue = QueueModel::default();
        let ctx = context(&weights, &impact, &queue);
        let venue = Venue::new(0, "EMPTY", 10, 1.0);
        let s = score_venue(
            &venue,
            0,
            Side::Buy,
            fixed(100.0),
            fixed(100.0),
            fixed(1e6),
            30,
            None,
            &ctx,
        );
        assert_eq!(s.score, i64::MIN);
    }

    #[test]
    fn test_fee_tier_changes_ranking() {
        let weights = ScoreWeights::default();
        let impact = ImpactParams::default();
        let queue = QueueModel::default();
        let ctx = context(&weights, &impact, &queue);
        let mut venue = Venue::new(0, "TIERED", 10, 1.0)
            .with_fees(0.0, 5.0)
            .with_fee_tiers(vec![FeeTier::from_bps(1_000_000.0, 0.0, 1.0)])
            .with_book(0, book(0.01, 10_000.0));
        let before = score_venue(
            &venue,
            0,
            Side::Buy,
            fixed(100.0),
            fixed(100.0),
            fixed(1e6),
            30,
            None,
            &ctx,
        );
        venue.monthly_volume = 2_000_000;
        let after = score_venue(
            &venue,
            0,
            Side::Buy,
            fixed(100.0),
            fixed(100.0),
            fixed(1e6),
            30,
            None,
            &ctx,
        );
        assert!(after.score > before.score);
        assert_eq!(after.net_fee_millibps, 1_000);
    }

    #[test]
    fn test_rank_scores_orders_by_score() {
        let mut scores = vec![
            VenueScore {
                score: 5,
                latency_us: 10,
                ..missing_score()
            },
            VenueScore {
                score: 9,
                latency_us: 50,
                ..missing_score()
            },
            VenueScore {
                score: 5,
                latency_us: 2,
                ..missing_score()
            },
        ];
        rank_scores(&mut scores);
        assert_eq!(scores[0].score, 9);
        assert_eq!(scores[1].latency_us, 2);
        assert_eq!(scores[2].latency_us, 10);
    }

    fn missing_score() -> VenueScore {
        VenueScore {
            venue_id: 0,
            requested_qty: Fixed::ZERO,
            filled_qty: Fixed::ZERO,
            expected_vwap: Fixed::ZERO,
            slippage_millibps: 0,
            net_fee_millibps: 0,
            impact_millibps: 0,
            latency_us: 0,
            fill_probability_ppm: 0,
            queue_fill_probability_ppm: 0,
            liquidity_available: Fixed::ZERO,
            liquidity_coverage_ppm: 0,
            score: 0,
        }
    }
}
