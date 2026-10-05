use super::fixed::Fixed;
use super::symbol::SymbolRegistry;
use super::venue::{FeeTier, PriceLevel, Venue, VenueBook};

/// Tiny deterministic linear congruential generator.
///
/// Keeping the topology generator dependency-free and seedable makes routing
/// simulations reproducible in tests and benchmarks. This runs only at setup
/// time, so floating point is acceptable here.
#[derive(Debug, Clone, Copy)]
pub struct Lcg {
    state: u64,
}

impl Lcg {
    pub fn new(seed: u64) -> Self {
        Lcg {
            state: seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407),
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.state
    }

    /// Uniform sample in [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform sample in [lo, hi).
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }
}

/// Blueprint for a simulated venue.
struct VenueTemplate {
    name: &'static str,
    latency_us: (u32, u32),
    fill_probability: (f64, f64),
    maker_bps: f64,
    taker_bps: f64,
    tier_volume: f64,
    tier_maker_bps: f64,
    tier_taker_bps: f64,
    /// Touch spread in bps.
    spread_bps: f64,
    /// Displayed size at the touch.
    touch_depth: f64,
}

const TEMPLATES: &[VenueTemplate] = &[
    VenueTemplate {
        name: "XNAS",
        latency_us: (15, 45),
        fill_probability: (0.96, 0.995),
        maker_bps: 0.0,
        taker_bps: 3.0,
        tier_volume: 2_000_000.0,
        tier_maker_bps: 0.0,
        tier_taker_bps: 2.5,
        spread_bps: 0.6,
        touch_depth: 4_000.0,
    },
    VenueTemplate {
        name: "ARCA",
        latency_us: (30, 80),
        fill_probability: (0.94, 0.99),
        maker_bps: -0.2,
        taker_bps: 2.8,
        tier_volume: 1_500_000.0,
        tier_maker_bps: -0.3,
        tier_taker_bps: 2.3,
        spread_bps: 0.8,
        touch_depth: 3_000.0,
    },
    VenueTemplate {
        name: "BATS",
        latency_us: (10, 35),
        fill_probability: (0.93, 0.985),
        maker_bps: -0.1,
        taker_bps: 2.6,
        tier_volume: 2_500_000.0,
        tier_maker_bps: -0.2,
        tier_taker_bps: 2.1,
        spread_bps: 1.0,
        touch_depth: 2_500.0,
    },
    VenueTemplate {
        name: "EDGX",
        latency_us: (20, 60),
        fill_probability: (0.9, 0.98),
        maker_bps: 0.1,
        taker_bps: 3.2,
        tier_volume: 1_000_000.0,
        tier_maker_bps: 0.0,
        tier_taker_bps: 2.7,
        spread_bps: 1.2,
        touch_depth: 2_000.0,
    },
    VenueTemplate {
        name: "IEX",
        latency_us: (40, 120),
        fill_probability: (0.85, 0.95),
        maker_bps: 0.0,
        taker_bps: 2.0,
        tier_volume: 800_000.0,
        tier_maker_bps: 0.0,
        tier_taker_bps: 1.8,
        spread_bps: 1.5,
        touch_depth: 1_500.0,
    },
    VenueTemplate {
        name: "DARK1",
        latency_us: (60, 180),
        fill_probability: (0.7, 0.9),
        maker_bps: -0.5,
        taker_bps: 1.5,
        tier_volume: 500_000.0,
        tier_maker_bps: -0.6,
        tier_taker_bps: 1.2,
        spread_bps: 2.0,
        touch_depth: 1_000.0,
    },
];

/// Build a deterministic, seeded multi-venue topology with a simulated book
/// for each symbol on every venue. Symbol names are interned into `registry`
/// so the hot path can look books up by dense [`super::symbol::SymbolId`].
pub fn simulated_topology(
    registry: &mut SymbolRegistry,
    symbols: &[&str],
    reference_price: f64,
    seed: u64,
) -> Vec<Venue> {
    simulated_topology_with_levels(registry, symbols, reference_price, seed, 5)
}

pub fn simulated_topology_with_levels(
    registry: &mut SymbolRegistry,
    symbols: &[&str],
    reference_price: f64,
    seed: u64,
    levels: usize,
) -> Vec<Venue> {
    let symbol_ids: Vec<_> = symbols
        .iter()
        .map(|symbol| registry.intern(symbol))
        .collect();

    TEMPLATES
        .iter()
        .enumerate()
        .map(|(index, template)| {
            let mut rng = Lcg::new(seed.wrapping_add(index as u64 * 97));
            let latency_us =
                rng.range(template.latency_us.0 as f64, template.latency_us.1 as f64) as u32;
            let fill_probability =
                rng.range(template.fill_probability.0, template.fill_probability.1);

            let mut venue = Venue::new(index, template.name, latency_us, fill_probability)
                .with_fees(template.maker_bps, template.taker_bps)
                .with_fee_tiers(vec![FeeTier::from_bps(
                    template.tier_volume,
                    template.tier_maker_bps,
                    template.tier_taker_bps,
                )]);

            for (symbol_index, &symbol_id) in symbol_ids.iter().enumerate() {
                let mut book_rng = Lcg::new(
                    seed.wrapping_add((index as u64 + 1) * 1_000)
                        .wrapping_add(symbol_index as u64),
                );
                let book = build_book(
                    reference_price,
                    template.spread_bps,
                    template.touch_depth,
                    levels,
                    &mut book_rng,
                );
                venue = venue.with_book(symbol_id, book);
            }

            venue
        })
        .collect()
}

/// Build one symbol's book around `reference_price`.
fn build_book(
    reference_price: f64,
    spread_bps: f64,
    touch_depth: f64,
    levels: usize,
    rng: &mut Lcg,
) -> VenueBook {
    let mut bids = Vec::with_capacity(levels);
    let mut asks = Vec::with_capacity(levels);
    let half_spread = spread_bps / 2.0 / 10_000.0 * reference_price;
    // One tick deeper per level, approximately 1 bp.
    let tick = reference_price / 10_000.0;

    for level in 0..levels {
        let offset = level as f64 * tick;
        let size = touch_depth * (1.0 + level as f64 * 0.6) * rng.range(0.7, 1.3);
        let queue_bid = size * rng.range(0.1, 0.9);
        let queue_ask = size * rng.range(0.1, 0.9);
        bids.push(PriceLevel::with_queue(
            Fixed::from_f64(reference_price - half_spread - offset),
            Fixed::from_f64(size),
            Fixed::from_f64(queue_bid),
        ));
        asks.push(PriceLevel::with_queue(
            Fixed::from_f64(reference_price + half_spread + offset),
            Fixed::from_f64(size),
            Fixed::from_f64(queue_ask),
        ));
    }

    VenueBook::from_levels(bids, asks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topology(symbols: &[&str], seed: u64) -> (Vec<Venue>, SymbolRegistry) {
        let mut registry = SymbolRegistry::new();
        let venues = simulated_topology(&mut registry, symbols, 100.0, seed);
        (venues, registry)
    }

    #[test]
    fn test_topology_is_deterministic() {
        let (a, _) = topology(&["AAPL"], 42);
        let (b, _) = topology(&["AAPL"], 42);
        assert_eq!(a.len(), b.len());
        for (venue_a, venue_b) in a.iter().zip(b.iter()) {
            assert_eq!(venue_a.latency_us, venue_b.latency_us);
            assert_eq!(venue_a.fill_probability_ppm, venue_b.fill_probability_ppm);
            assert_eq!(
                venue_a.book(0).unwrap().best_bid(),
                venue_b.book(0).unwrap().best_bid()
            );
        }
    }

    #[test]
    fn test_different_seeds_differ() {
        let (a, _) = topology(&["AAPL"], 1);
        let (b, _) = topology(&["AAPL"], 2);
        assert!(a
            .iter()
            .zip(b.iter())
            .any(|(x, y)| x.latency_us != y.latency_us));
    }

    #[test]
    fn test_every_venue_has_every_symbol() {
        let (venues, registry) = topology(&["AAPL", "MSFT"], 7);
        assert_eq!(venues.len(), 6);
        let aapl = registry.id("AAPL").unwrap();
        let msft = registry.id("MSFT").unwrap();
        for venue in &venues {
            assert!(venue.book(aapl).is_some());
            assert!(venue.book(msft).is_some());
        }
    }

    #[test]
    fn test_books_are_crossed_free() {
        let (venues, _) = topology(&["AAPL"], 99);
        for venue in &venues {
            let book = venue.book(0).unwrap();
            assert!(book.best_bid().unwrap() < book.best_ask().unwrap());
            assert!(book
                .bids
                .windows(2)
                .all(|pair| pair[0].price > pair[1].price));
            assert!(book
                .asks
                .windows(2)
                .all(|pair| pair[0].price < pair[1].price));
        }
    }

    #[test]
    fn test_lcg_bounds() {
        let mut rng = Lcg::new(123);
        for _ in 0..1_000 {
            let value = rng.next_f64();
            assert!((0.0..1.0).contains(&value));
        }
    }
}
