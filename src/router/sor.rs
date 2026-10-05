use super::fixed::{diff_millibps, Fixed, PPM};
use super::impact::{ImpactParams, QueueModel};
use super::scoring::{rank_scores, score_venue, ScoreWeights, ScoringContext, VenueScore};
use super::slicing::SliceSchedule;
use super::symbol::{SymbolId, SymbolRegistry};
use super::topology::simulated_topology;
use super::venue::{fee_amount, BookWalk, Venue, VenueId};
use crate::order::Side;
use std::time::Instant;

/// An order submitted to the router.
///
/// Prices and quantities are stored as [`Fixed`] so the hot path never touches
/// floating point. The convenience constructors accept `f64` and convert once
/// at the API boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderRequest {
    pub symbol: String,
    pub side: Side,
    pub quantity: Fixed,
    /// Optional limit price. `None` is a market order.
    pub limit_price: Option<Fixed>,
    /// Arrival / decision price used as the slippage reference.
    pub arrival_price: Fixed,
    /// Average daily volume for the symbol.
    pub adv: Fixed,
    /// Horizon over which the order may execute, in seconds.
    pub horizon_s: i64,
}

impl OrderRequest {
    pub fn market(symbol: &str, side: Side, quantity: f64, arrival_price: f64) -> Self {
        OrderRequest {
            symbol: symbol.to_string(),
            side,
            quantity: Fixed::from_f64(quantity),
            limit_price: None,
            arrival_price: Fixed::from_f64(arrival_price),
            adv: Fixed::from_f64(1_000_000.0),
            horizon_s: 60,
        }
    }

    pub fn with_limit(mut self, limit_price: f64) -> Self {
        self.limit_price = Some(Fixed::from_f64(limit_price));
        self
    }

    pub fn with_adv(mut self, adv: f64) -> Self {
        self.adv = Fixed::from_f64(adv);
        self
    }

    pub fn with_horizon_s(mut self, horizon_s: i64) -> Self {
        self.horizon_s = horizon_s;
        self
    }
}

/// A single child allocation produced by the router.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteAllocation {
    pub venue_id: VenueId,
    pub quantity: Fixed,
    pub expected_vwap: Fixed,
    pub expected_cost_millibps: i64,
    pub fill_probability_ppm: i64,
    pub score: i64,
}

/// The full set of child orders chosen for a parent order.
///
/// Vectors are reused across calls via [`RoutePlan::clear`], so steady-state
/// routing allocates nothing after the first pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoutePlan {
    pub allocations: Vec<RouteAllocation>,
    pub requested_qty: Fixed,
    pub allocated_qty: Fixed,
    pub unallocated_qty: Fixed,
    /// Time spent making the routing decision, in nanoseconds.
    pub decision_latency_ns: u64,
    /// Ranked scores for every venue considered.
    pub scores: Vec<VenueScore>,
}

impl RoutePlan {
    /// Reset the plan while retaining the capacity of its vectors.
    pub fn clear(&mut self) {
        self.allocations.clear();
        self.scores.clear();
        self.requested_qty = Fixed::ZERO;
        self.allocated_qty = Fixed::ZERO;
        self.unallocated_qty = Fixed::ZERO;
        self.decision_latency_ns = 0;
    }
}

/// A realized execution against a venue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fill {
    pub venue_id: VenueId,
    pub quantity: Fixed,
    pub price: Fixed,
    /// Signed fee in milli-bps (negative = rebate).
    pub fee_millibps: i64,
    pub notional: i128,
}

/// Outcome of routing and executing a single parent order.
///
/// Reuse across calls with [`ExecutionReport::clear`] for an allocation-free
/// steady state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionReport {
    pub plan: RoutePlan,
    pub fills: Vec<Fill>,
    pub requested_qty: Fixed,
    pub filled_qty: Fixed,
    pub notional_value: i128,
    pub avg_price: Fixed,
    pub arrival_price: Fixed,
    /// Realized slippage versus the arrival price, in milli-bps (+ = worse).
    pub realized_slippage_millibps: i64,
    pub total_fees: i128,
    pub decision_latency_ns: u64,
    pub execution_latency_ns: u64,
}

impl ExecutionReport {
    pub fn unfilled_qty(&self) -> Fixed {
        (self.requested_qty - self.filled_qty).max(Fixed::ZERO)
    }

    pub fn avg_price_f64(&self) -> f64 {
        self.avg_price.to_f64()
    }

    pub fn realized_slippage_bps(&self) -> f64 {
        self.realized_slippage_millibps as f64 / super::fixed::MILLI_BPS as f64
    }

    /// Reset while retaining vector capacity.
    pub fn clear(&mut self) {
        self.plan.clear();
        self.fills.clear();
        self.requested_qty = Fixed::ZERO;
        self.filled_qty = Fixed::ZERO;
        self.notional_value = 0;
        self.avg_price = Fixed::ZERO;
        self.arrival_price = Fixed::ZERO;
        self.realized_slippage_millibps = 0;
        self.total_fees = 0;
        self.decision_latency_ns = 0;
        self.execution_latency_ns = 0;
    }
}

/// Aggregated outcome of running a slicing schedule.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScheduleReport {
    pub slices: Vec<ExecutionReport>,
    pub total_requested: Fixed,
    pub total_filled: Fixed,
    pub notional_value: i128,
    pub avg_price: Fixed,
    pub realized_slippage_millibps: i64,
    pub total_fees: i128,
    pub avg_decision_latency_ns: u64,
    pub max_decision_latency_ns: u64,
}

impl ScheduleReport {
    pub fn unfilled_qty(&self) -> Fixed {
        (self.total_requested - self.total_filled).max(Fixed::ZERO)
    }

    pub fn num_slices(&self) -> usize {
        self.slices.len()
    }

    pub fn realized_slippage_bps(&self) -> f64 {
        self.realized_slippage_millibps as f64 / super::fixed::MILLI_BPS as f64
    }
}

/// Tunable router configuration. Fractions are expressed in ppm and cost
/// weights in milli-bps to keep the configuration integer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterConfig {
    pub weights: ScoreWeights,
    pub impact: ImpactParams,
    pub queue: QueueModel,
    /// Band around the reference price treated as implied liquidity, in bps.
    pub liquidity_band_bps: i64,
    /// Latency at which a quote has a 1/e chance of being stale.
    pub latency_decay_us: i64,
    /// Maximum fraction of ADV routed to a single venue per order, in ppm.
    pub max_participation_ppm: i64,
    /// Adapt child slice size to displayed liquidity while scheduling.
    pub adaptive_slicing: bool,
    /// Floor on adaptive slice release, in ppm.
    pub min_slice_fraction_ppm: i64,
}

impl Default for RouterConfig {
    fn default() -> Self {
        RouterConfig {
            weights: ScoreWeights::default(),
            impact: ImpactParams::default(),
            queue: QueueModel::default(),
            liquidity_band_bps: 10,
            latency_decay_us: 500,
            max_participation_ppm: 50_000,
            adaptive_slicing: true,
            min_slice_fraction_ppm: 400_000,
        }
    }
}

/// Rolling router statistics, including decision-latency tracking.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouterStats {
    pub orders: u64,
    pub decisions: u64,
    pub total_requested_qty: Fixed,
    pub total_routed_qty: Fixed,
    pub total_filled_qty: Fixed,
    pub total_notional: i128,
    pub total_fees: i128,
    pub total_decision_ns: u128,
    pub min_decision_ns: u64,
    pub max_decision_ns: u64,
    /// Routed quantity per venue, indexed by [`VenueId`].
    pub per_venue_qty: Vec<Fixed>,
    /// Number of child orders per venue, indexed by [`VenueId`].
    pub per_venue_orders: Vec<u64>,
}

impl RouterStats {
    pub fn avg_decision_ns(&self) -> f64 {
        if self.decisions > 0 {
            self.total_decision_ns as f64 / self.decisions as f64
        } else {
            0.0
        }
    }

    pub fn avg_fill_price(&self) -> Fixed {
        if self.total_filled_qty.raw() > 0 {
            Fixed::from_raw(
                (self.total_notional * super::fixed::SCALE_SQUARED_I128
                    / self.total_filled_qty.raw() as i128) as i64,
            )
        } else {
            Fixed::ZERO
        }
    }

    fn ensure_venues(&mut self, count: usize) {
        if self.per_venue_qty.len() < count {
            self.per_venue_qty.resize(count, Fixed::ZERO);
        }
        if self.per_venue_orders.len() < count {
            self.per_venue_orders.resize(count, 0);
        }
    }
}

/// Smart Order Router over a simulated multi-venue topology.
pub struct SmartOrderRouter {
    venues: Vec<Venue>,
    symbols: SymbolRegistry,
    config: RouterConfig,
    stats: RouterStats,
}

impl SmartOrderRouter {
    pub fn new(venues: Vec<Venue>, symbols: SymbolRegistry) -> Self {
        let mut stats = RouterStats::default();
        stats.ensure_venues(venues.len());
        SmartOrderRouter {
            venues,
            symbols,
            config: RouterConfig::default(),
            stats,
        }
    }

    /// Build a router directly from the seeded topology generator.
    pub fn simulated(symbols: &[&str], reference_price: f64, seed: u64) -> Self {
        let mut registry = SymbolRegistry::new();
        let venues = simulated_topology(&mut registry, symbols, reference_price, seed);
        SmartOrderRouter::new(venues, registry)
    }

    pub fn with_config(mut self, config: RouterConfig) -> Self {
        self.config = config;
        self
    }

    pub fn venues(&self) -> &[Venue] {
        &self.venues
    }

    pub fn venues_mut(&mut self) -> &mut [Venue] {
        &mut self.venues
    }

    pub fn symbols(&self) -> &SymbolRegistry {
        &self.symbols
    }

    pub fn config(&self) -> &RouterConfig {
        &self.config
    }

    pub fn stats(&self) -> &RouterStats {
        &self.stats
    }

    pub fn reset_stats(&mut self) {
        let count = self.venues.len();
        self.stats = RouterStats::default();
        self.stats.ensure_venues(count);
    }

    /// Upper bound on quantity routed to a single venue for this order.
    fn participation_cap(&self, req: &OrderRequest) -> Fixed {
        if req.adv.raw() > 0 {
            Fixed::from_raw(
                (req.adv.raw() as i128 * self.config.max_participation_ppm as i128 / PPM as i128)
                    as i64,
            )
        } else {
            Fixed::from_raw(i64::MAX)
        }
    }

    /// Score and rank every venue into `out` without allocating.
    pub fn score_destinations_into(
        &self,
        req: &OrderRequest,
        symbol: SymbolId,
        out: &mut Vec<VenueScore>,
    ) {
        out.clear();
        let context = ScoringContext {
            weights: &self.config.weights,
            impact: &self.config.impact,
            queue: &self.config.queue,
            liquidity_band_bps: self.config.liquidity_band_bps,
            latency_decay_us: self.config.latency_decay_us,
        };
        for venue in &self.venues {
            out.push(score_venue(
                venue,
                symbol,
                req.side,
                req.quantity,
                req.arrival_price,
                req.adv,
                req.horizon_s,
                req.limit_price,
                &context,
            ));
        }
        rank_scores(out);
    }

    /// Convenience wrapper that allocates a fresh score vector.
    pub fn score_destinations(&self, req: &OrderRequest) -> Vec<VenueScore> {
        let mut scores = Vec::with_capacity(self.venues.len());
        let symbol = self.symbols.id(&req.symbol).unwrap_or(u32::MAX);
        self.score_destinations_into(req, symbol, &mut scores);
        scores
    }

    /// Produce a cost-minimising allocation into a caller-owned plan.
    ///
    /// Venues are visited best-score-first and each receives as much as its
    /// displayed implied liquidity (capped by the participation limit) allows.
    /// This is a greedy water-filling allocation: the marginal cost of the
    /// next share is monotonically non-decreasing as we move down the ranking.
    pub fn route_into(&mut self, req: &OrderRequest, symbol: SymbolId, plan: &mut RoutePlan) {
        let start = Instant::now();
        plan.clear();
        plan.requested_qty = req.quantity.max(Fixed::ZERO);

        self.score_destinations_into(req, symbol, &mut plan.scores);
        let cap = self.participation_cap(req);
        let mut remaining = plan.requested_qty;

        for score in &plan.scores {
            if remaining.raw() <= 0 {
                break;
            }
            if score.score == i64::MIN {
                continue;
            }
            let available = score.liquidity_available.min(cap).max(Fixed::ZERO);
            let quantity = remaining.min(available);
            if quantity.raw() <= 0 {
                continue;
            }
            plan.allocations.push(RouteAllocation {
                venue_id: score.venue_id,
                quantity,
                expected_vwap: score.expected_vwap,
                expected_cost_millibps: score.total_cost_millibps(&self.config.weights),
                fill_probability_ppm: score.fill_probability_ppm,
                score: score.score,
            });
            remaining -= quantity;
        }

        plan.allocated_qty = (plan.requested_qty - remaining).max(Fixed::ZERO);
        plan.unallocated_qty = remaining;
        let decision_latency_ns = nanos(start.elapsed());
        plan.decision_latency_ns = decision_latency_ns;

        self.stats.orders += 1;
        self.stats.decisions += 1;
        self.stats.total_requested_qty += plan.requested_qty;
        self.stats.total_routed_qty += plan.allocated_qty;
        self.stats.total_decision_ns += decision_latency_ns as u128;
        self.stats.max_decision_ns = self.stats.max_decision_ns.max(decision_latency_ns);
        if self.stats.min_decision_ns == 0 || decision_latency_ns < self.stats.min_decision_ns {
            self.stats.min_decision_ns = decision_latency_ns;
        }
        for allocation in &plan.allocations {
            let id = allocation.venue_id;
            self.stats.per_venue_qty[id] += allocation.quantity;
            self.stats.per_venue_orders[id] += 1;
        }
    }

    /// Convenience wrapper that allocates a fresh plan.
    pub fn route(&mut self, req: &OrderRequest) -> RoutePlan {
        let mut plan = RoutePlan::default();
        let symbol = self.symbols.id(&req.symbol).unwrap_or(u32::MAX);
        self.route_into(req, symbol, &mut plan);
        plan
    }

    /// Route and simulate execution into a caller-owned report.
    pub fn execute_into(
        &mut self,
        req: &OrderRequest,
        symbol: SymbolId,
        report: &mut ExecutionReport,
    ) {
        let start = Instant::now();
        self.route_into(req, symbol, &mut report.plan);
        let decision_latency_ns = report.plan.decision_latency_ns;

        report.fills.clear();
        report.requested_qty = req.quantity.max(Fixed::ZERO);
        report.arrival_price = req.arrival_price;
        report.notional_value = 0;
        report.total_fees = 0;
        report.filled_qty = Fixed::ZERO;
        report.avg_price = Fixed::ZERO;
        report.realized_slippage_millibps = 0;
        report.decision_latency_ns = decision_latency_ns;

        for index in 0..report.plan.allocations.len() {
            let allocation = report.plan.allocations[index];
            let venue = match self
                .venues
                .iter_mut()
                .find(|venue| venue.id == allocation.venue_id)
            {
                Some(venue) => venue,
                None => continue,
            };

            let fee_millibps = venue.effective_fees().taker_millibps;
            let walk = match venue.book_mut(symbol) {
                Some(book) => book.consume(req.side, allocation.quantity, req.limit_price),
                None => BookWalk::default(),
            };
            if walk.filled_qty.raw() <= 0 {
                continue;
            }

            let notional = walk.notional_value();
            let fee = fee_amount(notional, fee_millibps);
            venue.monthly_volume += notional;

            report.fills.push(Fill {
                venue_id: allocation.venue_id,
                quantity: walk.filled_qty,
                price: walk.vwap(),
                fee_millibps,
                notional,
            });
            report.filled_qty += walk.filled_qty;
            report.notional_value += notional;
            report.total_fees += fee;
        }

        if report.filled_qty.raw() > 0 {
            report.avg_price = Fixed::from_raw(
                (report.notional_value * super::fixed::SCALE_SQUARED_I128
                    / report.filled_qty.raw() as i128) as i64,
            );
            let raw = diff_millibps(report.avg_price, req.arrival_price);
            report.realized_slippage_millibps = match req.side {
                Side::Buy => raw,
                Side::Sell => -raw,
            };
        }
        report.execution_latency_ns = nanos(start.elapsed());

        self.stats.total_filled_qty += report.filled_qty;
        self.stats.total_notional += report.notional_value;
        self.stats.total_fees += report.total_fees;
    }

    /// Convenience wrapper that allocates a fresh report.
    pub fn execute(&mut self, req: &OrderRequest) -> ExecutionReport {
        let mut report = ExecutionReport::default();
        let symbol = self.symbols.id(&req.symbol).unwrap_or(u32::MAX);
        self.execute_into(req, symbol, &mut report);
        report
    }

    /// Execute a slicing schedule slice-by-slice into a caller-owned report.
    ///
    /// Each child order interacts with the (mutating) simulated books, so later
    /// slices see the impact of earlier ones. When `adaptive_slicing` is set,
    /// thin displayed liquidity reduces the child release and the remainder is
    /// carried forward.
    pub fn execute_schedule_into(
        &mut self,
        req: &OrderRequest,
        schedule: &SliceSchedule,
        report: &mut ScheduleReport,
    ) {
        let symbol = self.symbols.id(&req.symbol).unwrap_or(u32::MAX);
        let expected = schedule.slices.len();
        while report.slices.len() < expected {
            report.slices.push(ExecutionReport::default());
        }

        let mut used = 0usize;
        let mut carryover = Fixed::ZERO;
        for slice in &schedule.slices {
            let base = slice.quantity + carryover;
            if base.raw() <= 0 {
                continue;
            }

            let release = if self.config.adaptive_slicing {
                self.adaptive_child_qty(req, symbol, base)
            } else {
                base
            };
            let release = release.clamp(Fixed::ZERO, base);
            carryover = base - release;
            if release.raw() <= 0 {
                continue;
            }

            let mut child = req.clone();
            child.quantity = release;
            child.horizon_s = schedule.interval_s.max(1);

            self.execute_into(&child, symbol, &mut report.slices[used]);
            used += 1;
        }

        report.slices.truncate(used);
        report.total_requested = Fixed::ZERO;
        report.total_filled = Fixed::ZERO;
        report.notional_value = 0;
        report.total_fees = 0;
        let mut decision_total: u128 = 0;
        let mut decision_max: u64 = 0;
        for slice in &report.slices {
            report.total_requested += slice.requested_qty;
            report.total_filled += slice.filled_qty;
            report.notional_value += slice.notional_value;
            report.total_fees += slice.total_fees;
            decision_total += slice.decision_latency_ns as u128;
            decision_max = decision_max.max(slice.decision_latency_ns);
        }
        report.avg_price = if report.total_filled.raw() > 0 {
            Fixed::from_raw(
                (report.notional_value * super::fixed::SCALE_SQUARED_I128
                    / report.total_filled.raw() as i128) as i64,
            )
        } else {
            Fixed::ZERO
        };
        let raw = diff_millibps(report.avg_price, req.arrival_price);
        report.realized_slippage_millibps = if report.total_filled.raw() > 0 {
            match req.side {
                Side::Buy => raw,
                Side::Sell => -raw,
            }
        } else {
            0
        };
        report.max_decision_latency_ns = decision_max;
        report.avg_decision_latency_ns = if report.slices.is_empty() {
            0
        } else {
            (decision_total / report.slices.len() as u128) as u64
        };
    }

    /// Convenience wrapper that allocates a fresh schedule report.
    pub fn execute_schedule(
        &mut self,
        req: &OrderRequest,
        schedule: &SliceSchedule,
    ) -> ScheduleReport {
        let mut report = ScheduleReport::default();
        self.execute_schedule_into(req, schedule, &mut report);
        report
    }

    /// Microstructure-aware child slice sizing.
    ///
    /// Full size is released when displayed liquidity covers the slice; the
    /// release is scaled down toward `min_slice_fraction_ppm` when the book is
    /// thin, deferring the remainder to a later slice.
    fn adaptive_child_qty(&self, req: &OrderRequest, symbol: SymbolId, base: Fixed) -> Fixed {
        if base.raw() <= 0 {
            return Fixed::ZERO;
        }
        let total_liquidity = self
            .venues
            .iter()
            .filter_map(|venue| venue.book(symbol))
            .fold(Fixed::ZERO, |acc, book| {
                acc + book.implied_liquidity(
                    req.side,
                    self.config.liquidity_band_bps,
                    req.arrival_price,
                )
            });
        let coverage_ppm = total_liquidity.ratio_ppm(base).clamp(0, PPM);
        let min_fraction = self.config.min_slice_fraction_ppm.clamp(0, PPM);
        let fraction_ppm = min_fraction + (PPM - min_fraction) * coverage_ppm / PPM;
        Fixed::from_raw((base.raw() as i128 * fraction_ppm as i128 / PPM as i128) as i64)
    }
}

/// Signed realized slippage in milli-bps. Positive means worse than arrival.
pub fn realized_slippage_millibps(
    side: Side,
    avg_price: Fixed,
    arrival_price: Fixed,
    filled_qty: Fixed,
) -> i64 {
    if filled_qty.raw() <= 0 || arrival_price.raw() <= 0 || avg_price.raw() <= 0 {
        return 0;
    }
    let raw = diff_millibps(avg_price, arrival_price);
    match side {
        Side::Buy => raw,
        Side::Sell => -raw,
    }
}

fn nanos(duration: std::time::Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::fixed::MILLI_BPS;
    use crate::router::slicing::{u_shaped_volume_curve, SliceSchedule};
    use crate::router::venue::{FeeTier, PriceLevel, VenueBook};

    fn fixed(value: f64) -> Fixed {
        Fixed::from_f64(value)
    }

    fn book(mid: f64, spread: f64, depth: f64) -> VenueBook {
        VenueBook::from_levels(
            vec![
                PriceLevel::with_queue(fixed(mid - spread / 2.0), fixed(depth), fixed(100.0)),
                PriceLevel::with_queue(fixed(mid - spread / 2.0 - 0.01), fixed(depth), Fixed::ZERO),
            ],
            vec![
                PriceLevel::with_queue(fixed(mid + spread / 2.0), fixed(depth), fixed(100.0)),
                PriceLevel::with_queue(fixed(mid + spread / 2.0 + 0.01), fixed(depth), Fixed::ZERO),
            ],
        )
    }

    fn topology() -> (SmartOrderRouter, SymbolId) {
        let mut symbols = SymbolRegistry::new();
        let aapl = symbols.intern("AAPL");
        let venues = vec![
            Venue::new(0, "XNAS", 20, 0.99)
                .with_fees(0.0, 1.0)
                .with_fee_tiers(vec![FeeTier::from_bps(1_000_000.0, 0.0, 0.5)])
                .with_book(aapl, book(100.0, 0.01, 2_000.0)),
            Venue::new(1, "ARCA", 40, 0.97)
                .with_fees(-0.2, 2.5)
                .with_book(aapl, book(100.0, 0.02, 3_000.0)),
            Venue::new(2, "BATS", 15, 0.95)
                .with_fees(0.0, 3.0)
                .with_book(aapl, book(100.0, 0.03, 5_000.0)),
        ];
        (SmartOrderRouter::new(venues, symbols), aapl)
    }

    #[test]
    fn test_scores_ranked_best_first() {
        let (router, symbol) = topology();
        let req = OrderRequest::market("AAPL", Side::Buy, 500.0, 100.0);
        let mut scores = Vec::new();
        router.score_destinations_into(&req, symbol, &mut scores);
        assert_eq!(scores.len(), 3);
        for pair in scores.windows(2) {
            assert!(pair[0].score >= pair[1].score);
        }
    }

    #[test]
    fn test_route_concentrates_in_best_venue() {
        let (mut router, _) = topology();
        let req = OrderRequest::market("AAPL", Side::Buy, 1_000.0, 100.0);
        let plan = router.route(&req);
        assert!(!plan.allocations.is_empty());
        assert_eq!(plan.allocations[0].venue_id, 0);
        assert_eq!(plan.allocated_qty, fixed(1_000.0));
        assert!(plan.decision_latency_ns < 5_000_000);
    }

    #[test]
    fn test_route_splits_when_best_venue_thin() {
        let mut symbols = SymbolRegistry::new();
        let aapl = symbols.intern("AAPL");
        let venues = vec![
            Venue::new(0, "THIN", 10, 1.0)
                .with_fees(0.0, 0.5)
                .with_book(aapl, book(100.0, 0.01, 100.0)),
            Venue::new(1, "DEEP", 10, 1.0)
                .with_fees(0.0, 2.0)
                .with_book(aapl, book(100.0, 0.02, 10_000.0)),
        ];
        let mut router = SmartOrderRouter::new(venues, symbols);
        let req = OrderRequest::market("AAPL", Side::Buy, 30_000.0, 100.0).with_adv(1e9);
        let plan = router.route(&req);
        assert_eq!(plan.allocations.len(), 2);
        let venues_used: Vec<_> = plan.allocations.iter().map(|a| a.venue_id).collect();
        assert!(venues_used.contains(&0));
        assert!(venues_used.contains(&1));
        assert_eq!(plan.allocated_qty, fixed(20_200.0));
        assert!(plan.unallocated_qty.is_positive());
    }

    #[test]
    fn test_unallocated_when_liquidity_insufficient() {
        let (mut router, _) = topology();
        let req = OrderRequest::market("AAPL", Side::Buy, 1_000_000.0, 100.0).with_adv(1e9);
        let plan = router.route(&req);
        assert!(plan.unallocated_qty.is_positive());
    }

    #[test]
    fn test_participation_cap_limits_venue() {
        let (mut router, _) = topology();
        let req = OrderRequest::market("AAPL", Side::Buy, 1_000.0, 100.0).with_adv(1_000.0);
        let plan = router.route(&req);
        for allocation in &plan.allocations {
            assert!(allocation.quantity <= fixed(50.0));
        }
    }

    #[test]
    fn test_execute_consumes_liquidity() {
        let (mut router, symbol) = topology();
        let req = OrderRequest::market("AAPL", Side::Buy, 1_000.0, 100.0);
        let before = router.venues()[0]
            .book(symbol)
            .unwrap()
            .asks
            .iter()
            .fold(Fixed::ZERO, |acc, level| acc + level.size);
        let report = router.execute(&req);
        let after = router.venues()[0]
            .book(symbol)
            .unwrap()
            .asks
            .iter()
            .fold(Fixed::ZERO, |acc, level| acc + level.size);
        assert_eq!(report.filled_qty, fixed(1_000.0));
        assert!(after < before);
        assert!(report.avg_price > fixed(100.0));
        assert!(report.realized_slippage_millibps > 0);
    }

    #[test]
    fn test_execute_sell_slippage_positive() {
        let (mut router, _) = topology();
        let req = OrderRequest::market("AAPL", Side::Sell, 1_000.0, 100.0);
        let report = router.execute(&req);
        assert_eq!(report.filled_qty, fixed(1_000.0));
        assert!(report.avg_price < fixed(100.0));
        assert!(report.realized_slippage_millibps > 0);
    }

    #[test]
    fn test_fee_tier_evolves_with_volume() {
        let (mut router, _) = topology();
        let req = OrderRequest::market("AAPL", Side::Buy, 1_000.0, 100.0);
        router.execute(&req);
        assert!(router.venues()[0].monthly_volume > 0);
        assert!(router.stats().total_fees > 0);
        assert!(router.stats().total_filled_qty.is_positive());
    }

    #[test]
    fn test_twap_schedule_execution() {
        let (mut router, _) = topology();
        let req = OrderRequest::market("AAPL", Side::Buy, 1_000.0, 100.0).with_adv(1e9);
        let schedule = SliceSchedule::twap(fixed(1_000.0), 50, 10);
        let report = router.execute_schedule(&req, &schedule);
        assert_eq!(report.num_slices(), 5);
        assert_eq!(report.total_filled, fixed(1_000.0));
        for slice in &report.slices {
            assert!(slice.avg_price >= fixed(100.0));
        }
    }

    #[test]
    fn test_vwap_schedule_weights() {
        let (mut router, _) = topology();
        let req = OrderRequest::market("AAPL", Side::Buy, 1_000.0, 100.0).with_adv(1e9);
        let schedule = SliceSchedule::vwap(fixed(1_000.0), 50, 10, u_shaped_volume_curve());
        let report = router.execute_schedule(&req, &schedule);
        assert_eq!(report.num_slices(), 5);
        assert!(report.total_filled.is_positive());
    }

    #[test]
    fn test_adaptive_scales_with_liquidity() {
        let mut symbols = SymbolRegistry::new();
        let aapl = symbols.intern("AAPL");
        let thin = vec![Venue::new(0, "THIN", 10, 1.0)
            .with_fees(0.0, 1.0)
            .with_book(aapl, book(100.0, 0.01, 5.0))];
        let config = RouterConfig {
            max_participation_ppm: PPM,
            min_slice_fraction_ppm: 500_000,
            ..RouterConfig::default()
        };
        let router = SmartOrderRouter::new(thin, symbols).with_config(config);
        let req = OrderRequest::market("AAPL", Side::Buy, 1_000.0, 100.0).with_adv(1e9);
        // Displayed liquidity = 2 levels * 5 = 10; base slice 200 => coverage
        // 0.05, so the release is 0.5 + 0.5 * 0.05 of the base = 105.
        let release = router.adaptive_child_qty(&req, aapl, fixed(200.0));
        assert!(release >= fixed(100.0) && release < fixed(200.0));
        assert_eq!(release, fixed(105.0));

        let mut symbols = SymbolRegistry::new();
        let aapl = symbols.intern("AAPL");
        let deep =
            vec![Venue::new(0, "DEEP", 10, 1.0).with_book(aapl, book(100.0, 0.01, 100_000.0))];
        let router = SmartOrderRouter::new(deep, symbols).with_config(RouterConfig {
            min_slice_fraction_ppm: 400_000,
            ..RouterConfig::default()
        });
        assert_eq!(
            router.adaptive_child_qty(&req, aapl, fixed(200.0)),
            fixed(200.0)
        );
    }

    #[test]
    fn test_non_adaptive_releases_full_slice() {
        let deep = vec![Venue::new(0, "DEEP", 10, 1.0)
            .with_fees(0.0, 1.0)
            .with_book(0, book(100.0, 0.01, 100_000.0))];
        let config = RouterConfig {
            max_participation_ppm: PPM,
            adaptive_slicing: false,
            ..RouterConfig::default()
        };
        let mut router = SmartOrderRouter::new(deep, SymbolRegistry::default()).with_config(config);
        // Manually intern AAPL at id 0 for this venue.
        router.symbols = {
            let mut registry = SymbolRegistry::new();
            registry.intern("AAPL");
            registry
        };
        let req = OrderRequest::market("AAPL", Side::Buy, 1_000.0, 100.0).with_adv(1e9);
        let schedule = SliceSchedule::twap(fixed(1_000.0), 50, 10);
        let report = router.execute_schedule(&req, &schedule);
        assert_eq!(report.slices[0].filled_qty, fixed(200.0));
        assert_eq!(report.total_filled, fixed(1_000.0));
    }

    #[test]
    fn test_stats_track_latency_and_venues() {
        let (mut router, _) = topology();
        for _ in 0..20 {
            let req = OrderRequest::market("AAPL", Side::Buy, 100.0, 100.0);
            router.execute(&req);
        }
        let stats = router.stats();
        assert_eq!(stats.orders, 20);
        assert!(stats.max_decision_ns >= stats.min_decision_ns);
        assert!(stats.per_venue_qty.iter().any(|qty| qty.is_positive()));
        assert!(stats.avg_decision_ns() < 1_000_000.0);
    }

    #[test]
    fn test_no_book_venue_skipped() {
        let venues = vec![
            Venue::new(0, "NOBOOK", 10, 1.0).with_fees(0.0, 0.1),
            Venue::new(1, "HAS", 10, 1.0)
                .with_fees(0.0, 3.0)
                .with_book(1, book(100.0, 0.02, 5_000.0)),
        ];
        let mut symbols = SymbolRegistry::new();
        symbols.intern("DUMMY");
        symbols.intern("AAPL");
        let mut router = SmartOrderRouter::new(venues, symbols);
        let req = OrderRequest::market("AAPL", Side::Buy, 500.0, 100.0).with_adv(1e9);
        let plan = router.route(&req);
        assert!(plan
            .allocations
            .iter()
            .all(|allocation| allocation.venue_id == 1));
    }

    #[test]
    fn test_limit_order_is_passive() {
        let (router, _) = topology();
        let req = OrderRequest::market("AAPL", Side::Buy, 500.0, 100.0).with_limit(99.0);
        let scores = router.score_destinations(&req);
        assert!(scores
            .iter()
            .all(|score| score.expected_vwap == fixed(99.0)));
        assert!(scores
            .iter()
            .all(|score| score.queue_fill_probability_ppm < PPM));
    }

    #[test]
    fn test_reused_buffers_report_capacity_retained() {
        let (mut router, _) = topology();
        let req = OrderRequest::market("AAPL", Side::Buy, 500.0, 100.0);
        let mut report = ExecutionReport::default();
        router.execute_into(&req, 0, &mut report);
        let score_capacity = report.plan.scores.capacity();
        let fill_capacity = report.fills.capacity();
        router.execute_into(&req, 0, &mut report);
        assert_eq!(report.plan.scores.capacity(), score_capacity);
        assert_eq!(report.fills.capacity(), fill_capacity);
        assert_eq!(report.filled_qty, fixed(500.0));
    }

    #[test]
    fn test_realized_slippage_helper_matches_report() {
        let (mut router, _) = topology();
        let req = OrderRequest::market("AAPL", Side::Buy, 1_000.0, 100.0);
        let report = router.execute(&req);
        let expected = realized_slippage_millibps(
            Side::Buy,
            report.avg_price,
            req.arrival_price,
            report.filled_qty,
        );
        assert_eq!(report.realized_slippage_millibps, expected);
        assert!(report.realized_slippage_bps() * MILLI_BPS as f64 > 0.0);
    }
}
