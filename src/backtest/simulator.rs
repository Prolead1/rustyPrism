use super::BacktestConfig;
use crate::router::fixed::Fixed;
use crate::router::sor::SmartOrderRouter;
use crate::router::symbol::SymbolId;
use crate::router::topology::{build_book, Lcg};
use crate::router::venue::VenueBook;

/// Per-venue book shape captured from the topology so it can be regenerated at
/// each time step.
#[derive(Debug, Clone, Copy)]
struct VenueProfile {
    levels: usize,
    half_spread_bps: f64,
    touch_depth: f64,
}

/// Seeded market simulator.
///
/// Evolves a single consolidated mid price with a geometric random walk and
/// rebuilds every venue's book around it. The router's market-data view is
/// refreshed in place, so its routing decisions see a moving market without any
/// book-keeping on the caller's side. This runs at setup/backtest cadence, not
/// in the routing hot path, so floating point is acceptable here.
pub struct MarketSimulator {
    rng: Lcg,
    mid: f64,
    drift_per_step: f64,
    vol_per_step: f64,
    profiles: Vec<VenueProfile>,
    symbol: SymbolId,
}

impl MarketSimulator {
    /// Build a simulator from the router's current books for `symbol`.
    pub fn from_router(
        router: &SmartOrderRouter,
        symbol: SymbolId,
        config: &BacktestConfig,
    ) -> Self {
        let profiles = router
            .venues()
            .iter()
            .map(|venue| {
                venue
                    .book(symbol)
                    .map(profile_from_book)
                    .unwrap_or(VenueProfile {
                        levels: 5,
                        half_spread_bps: 0.5,
                        touch_depth: 1_000.0,
                    })
            })
            .collect();

        MarketSimulator {
            rng: Lcg::new(config.seed),
            mid: consolidated_mid(router, symbol).to_f64().max(1.0),
            drift_per_step: config.drift_bps_per_step / 10_000.0,
            vol_per_step: config.vol_bps_per_step / 10_000.0,
            profiles,
            symbol,
        }
    }

    /// Current simulated mid.
    pub fn mid(&self) -> Fixed {
        Fixed::from_f64(self.mid)
    }

    /// Advance the market one step and refresh every venue book.
    pub fn step(&mut self, router: &mut SmartOrderRouter) {
        self.mid *= 1.0 + self.drift_per_step + self.vol_per_step * self.rng.normal();
        let mid = self.mid;
        let symbol = self.symbol;

        // The rng is shared across venues so the depth jitter is correlated
        // with the price path and fully deterministic.
        for (index, profile) in self.profiles.iter().enumerate() {
            let book = build_book(
                mid,
                profile.half_spread_bps * 2.0,
                profile.touch_depth,
                profile.levels,
                &mut self.rng,
            );
            if let Some(venue) = router.venues_mut().get_mut(index) {
                venue.set_book(symbol, book);
            }
        }
    }
}

/// Touch-size weighted consolidated mid across all venues that quote `symbol`.
pub fn consolidated_mid(router: &SmartOrderRouter, symbol: SymbolId) -> Fixed {
    let mut numerator: i128 = 0;
    let mut denominator: i128 = 0;
    for venue in router.venues() {
        let Some(book) = venue.book(symbol) else {
            continue;
        };
        let Some(mid) = book.mid_price() else {
            continue;
        };
        let weight = book
            .asks
            .first()
            .map(|level| level.size)
            .unwrap_or(Fixed::ZERO)
            + book
                .bids
                .first()
                .map(|level| level.size)
                .unwrap_or(Fixed::ZERO);
        numerator += mid.raw() as i128 * weight.raw() as i128;
        denominator += weight.raw() as i128;
    }
    if denominator > 0 {
        Fixed::from_raw((numerator / denominator) as i64)
    } else {
        Fixed::ZERO
    }
}

fn profile_from_book(book: &VenueBook) -> VenueProfile {
    let levels = book.bids.len().min(book.asks.len()).max(1);
    let half_spread_bps = if book.spread_millibps() > 0 {
        book.spread_millibps() as f64 / 2_000.0
    } else {
        0.5
    };
    let touch_depth = book
        .asks
        .first()
        .map(|level| level.size.to_f64())
        .unwrap_or(1_000.0);
    VenueProfile {
        levels,
        half_spread_bps,
        touch_depth,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::sor::SmartOrderRouter;

    #[test]
    fn test_simulator_is_deterministic() {
        let config = BacktestConfig::default();
        let mut a = SmartOrderRouter::simulated(&["AAPL"], 100.0, 42);
        let mut b = SmartOrderRouter::simulated(&["AAPL"], 100.0, 42);
        let symbol = a.symbols().id("AAPL").unwrap();
        let mut sim_a = MarketSimulator::from_router(&a, symbol, &config);
        let mut sim_b = MarketSimulator::from_router(&b, symbol, &config);
        for _ in 0..20 {
            sim_a.step(&mut a);
            sim_b.step(&mut b);
            assert_eq!(consolidated_mid(&a, symbol), consolidated_mid(&b, symbol));
        }
    }

    #[test]
    fn test_books_stay_crossed_free() {
        let config = BacktestConfig::default();
        let mut router = SmartOrderRouter::simulated(&["AAPL"], 100.0, 42);
        let symbol = router.symbols().id("AAPL").unwrap();
        let mut sim = MarketSimulator::from_router(&router, symbol, &config);
        for _ in 0..50 {
            sim.step(&mut router);
            for venue in router.venues() {
                let book = venue.book(symbol).unwrap();
                assert!(book.best_bid().unwrap() < book.best_ask().unwrap());
            }
        }
    }
}
