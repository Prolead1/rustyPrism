//! Dependency-free latency percentile harness for the Smart Order Router.
//!
//! Unlike `cargo bench` (Criterion), which estimates a mean with confidence
//! intervals, this harness records the *distribution* of decision latencies and
//! prints exact percentiles. It also contrasts the allocating convenience APIs
//! with the buffer-reusing `_into` APIs.
//!
//! Run with:
//! ```bash
//! cargo bench --bench latency_percentiles
//! ```

use rusty_prism::order::Side;
use rusty_prism::router::slicing::SliceSchedule;
use rusty_prism::router::sor::{ExecutionReport, OrderRequest, RoutePlan, SmartOrderRouter};
use rusty_prism::router::symbol::SymbolRegistry;
use rusty_prism::router::topology::simulated_topology;
use std::hint::black_box;
use std::time::Instant;

const SYMBOL: &str = "AAPL";
const SEED: u64 = 42;

fn fresh_router() -> SmartOrderRouter {
    SmartOrderRouter::simulated(&[SYMBOL], 100.0, SEED)
}

fn market_request() -> OrderRequest {
    OrderRequest::market(SYMBOL, Side::Buy, 5_000.0, 100.0).with_adv(2_000_000.0)
}

fn passive_request() -> OrderRequest {
    OrderRequest::market(SYMBOL, Side::Buy, 5_000.0, 100.0)
        .with_adv(2_000_000.0)
        .with_limit(99.0)
}

struct Stats {
    name: String,
    n: usize,
    min: u64,
    p50: u64,
    p90: u64,
    p99: u64,
    p999: u64,
    max: u64,
    mean: f64,
    stddev: f64,
}

impl Stats {
    fn from_samples(name: impl Into<String>, mut samples: Vec<u64>) -> Self {
        let name = name.into();
        samples.sort_unstable();
        let n = samples.len();
        let mean = if n > 0 {
            samples.iter().map(|&value| value as f64).sum::<f64>() / n as f64
        } else {
            0.0
        };
        let variance = if n > 0 {
            samples
                .iter()
                .map(|&value| {
                    let delta = value as f64 - mean;
                    delta * delta
                })
                .sum::<f64>()
                / n as f64
        } else {
            0.0
        };
        Stats {
            name,
            n,
            min: percentile(&samples, 0.0),
            p50: percentile(&samples, 50.0),
            p90: percentile(&samples, 90.0),
            p99: percentile(&samples, 99.0),
            p999: percentile(&samples, 99.9),
            max: percentile(&samples, 100.0),
            mean,
            stddev: variance.sqrt(),
        }
    }
}

/// Nearest-rank percentile. `p` is in [0, 100].
fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    if p <= 0.0 {
        return sorted[0];
    }
    if p >= 100.0 {
        return sorted[sorted.len() - 1];
    }
    let rank = (p / 100.0 * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

/// Warm up, then time `iterations` calls of `f`, returning per-call nanoseconds.
fn measure<F: FnMut()>(iterations: usize, warmup: usize, mut f: F) -> Vec<u64> {
    for _ in 0..warmup {
        f();
    }
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        f();
        samples.push(start.elapsed().as_nanos().min(u64::MAX as u128) as u64);
    }
    samples
}

/// Time `f` once per freshly built router. Topology construction happens
/// outside the timed region so only the router work is measured.
fn measure_fresh<F: Fn(&mut SmartOrderRouter)>(iterations: usize, warmup: usize, f: F) -> Vec<u64> {
    for _ in 0..warmup {
        let mut router = fresh_router();
        f(&mut router);
    }
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let mut router = fresh_router();
        let start = Instant::now();
        f(&mut router);
        samples.push(start.elapsed().as_nanos().min(u64::MAX as u128) as u64);
    }
    samples
}

/// Time `route_into` with a single reused plan across every iteration.
fn measure_route_reused(iterations: usize, warmup: usize) -> Vec<u64> {
    let mut router = fresh_router();
    let request = market_request();
    let mut plan = RoutePlan::default();
    for _ in 0..warmup {
        router.route_into(&request, 0, &mut plan);
    }
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        router.route_into(&request, 0, &mut plan);
        samples.push(start.elapsed().as_nanos().min(u64::MAX as u128) as u64);
    }
    samples
}

/// Time `execute_into` with a single reused report across every iteration,
/// rebuilding the books outside the timed region.
fn measure_execute_reused(iterations: usize, warmup: usize) -> Vec<u64> {
    let request = market_request();
    let mut report = ExecutionReport::default();
    for _ in 0..warmup {
        let mut router = fresh_router();
        router.execute_into(&request, 0, &mut report);
    }
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let mut router = fresh_router();
        let start = Instant::now();
        router.execute_into(&request, 0, &mut report);
        samples.push(start.elapsed().as_nanos().min(u64::MAX as u128) as u64);
    }
    samples
}

/// Build a router with `venue_count` venues by cycling the simulated templates.
fn scale_router(venue_count: usize) -> SmartOrderRouter {
    let mut registry = SymbolRegistry::new();
    let base = simulated_topology(&mut registry, &[SYMBOL], 100.0, SEED);
    let mut venues = Vec::with_capacity(venue_count);
    for index in 0..venue_count {
        let mut venue = base[index % base.len()].clone();
        venue.id = index;
        venue.name = format!("{}-{}", base[index % base.len()].name, index / base.len());
        venues.push(venue);
    }
    SmartOrderRouter::new(venues, registry)
}

fn print_header() {
    println!(
        "\n{:<34} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "operation", "samples", "min", "p50", "p90", "p99", "p99.9", "max", "mean", "stddev"
    );
    println!("{}", "-".repeat(128));
}

fn print_row(stats: &Stats) {
    println!(
        "{:<34} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9.0} {:>9.1}",
        stats.name,
        stats.n,
        stats.min,
        stats.p50,
        stats.p90,
        stats.p99,
        stats.p999,
        stats.max,
        stats.mean,
        stats.stddev,
    );
}

fn main() {
    println!("Smart Order Router latency percentiles (nanoseconds)");
    println!("release build recommended: cargo bench --bench latency_percentiles");
    print_header();

    let decision_iterations = 500_000;
    let decision_warmup = 50_000;

    let mut router = fresh_router();
    let market = market_request();
    let passive = passive_request();

    let stats = Stats::from_samples(
        "score_destinations (6 venues)",
        measure(decision_iterations, decision_warmup, || {
            black_box(router.score_destinations(black_box(&market)));
        }),
    );
    print_row(&stats);

    let stats = Stats::from_samples(
        "route market",
        measure(decision_iterations, decision_warmup, || {
            black_box(router.route(black_box(&market)));
        }),
    );
    print_row(&stats);

    let stats = Stats::from_samples(
        "route market (reused plan)",
        measure_route_reused(decision_iterations, decision_warmup),
    );
    print_row(&stats);

    let stats = Stats::from_samples(
        "route passive limit",
        measure(decision_iterations, decision_warmup, || {
            black_box(router.route(black_box(&passive)));
        }),
    );
    print_row(&stats);

    let execution_iterations = 20_000;
    let execution_warmup = 2_000;

    let stats = Stats::from_samples(
        "execute market",
        measure_fresh(execution_iterations, execution_warmup, |router| {
            black_box(router.execute(black_box(&market)));
        }),
    );
    print_row(&stats);

    let stats = Stats::from_samples(
        "execute market (reused report)",
        measure_execute_reused(execution_iterations, execution_warmup),
    );
    print_row(&stats);

    let schedule = SliceSchedule::twap(
        rusty_prism::router::fixed::Fixed::from_f64(10_000.0),
        100,
        20,
    );
    let stats = Stats::from_samples(
        "execute_schedule TWAP (5 slices)",
        measure_fresh(execution_iterations, execution_warmup, |router| {
            black_box(router.execute_schedule(black_box(&market), &schedule));
        }),
    );
    print_row(&stats);

    println!("\nAll timings are per-call wall-clock nanoseconds.");

    println!("\nRouting-decision scaling with venue count (route market)");
    print_header();
    for venue_count in [6usize, 12, 24, 48, 96] {
        let mut router = scale_router(venue_count);
        let request = market_request();
        let stats = Stats::from_samples(
            format!("route market ({} venues)", venue_count),
            measure(100_000, 10_000, || {
                black_box(router.route(black_box(&request)));
            }),
        );
        print_row(&stats);
    }
}
