//! Criterion benchmarks for the Smart Order Router.
//!
//! These give statistically robust timings (warm-up, outlier detection and
//! confidence intervals). For exact p99/p99.9 decision latencies see the
//! `latency_percentiles` benchmark.
//!
//! Run with:
//! ```bash
//! cargo bench --bench sor_bench
//! ```

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use rusty_prism::order::Side;
use rusty_prism::router::fixed::Fixed;
use rusty_prism::router::slicing::{u_shaped_volume_curve, SliceSchedule};
use rusty_prism::router::sor::{OrderRequest, SmartOrderRouter};
use rusty_prism::router::symbol::SymbolRegistry;
use rusty_prism::router::topology::simulated_topology;
use std::hint::black_box;

const SYMBOL: &str = "AAPL";
const SEED: u64 = 42;

fn fresh_router() -> SmartOrderRouter {
    SmartOrderRouter::simulated(&[SYMBOL], 100.0, SEED)
}

fn market_request() -> OrderRequest {
    OrderRequest::market(SYMBOL, Side::Buy, 5_000.0, 100.0).with_adv(2_000_000.0)
}

fn bench_scoring(c: &mut Criterion) {
    let router = fresh_router();
    let request = market_request();
    let mut group = c.benchmark_group("routing_decision");
    group.throughput(Throughput::Elements(1));
    group.bench_function("score_destinations_6_venues", |b| {
        b.iter(|| black_box(router.score_destinations(black_box(&request))));
    });
    group.finish();
}

fn bench_route(c: &mut Criterion) {
    let mut router = fresh_router();
    let request = market_request();
    let mut group = c.benchmark_group("routing_decision");
    group.throughput(Throughput::Elements(1));
    group.bench_function("route_market", |b| {
        b.iter(|| black_box(router.route(black_box(&request))));
    });
    group.finish();
}

fn bench_execute(c: &mut Criterion) {
    let request = market_request();
    let mut group = c.benchmark_group("execution");
    group.throughput(Throughput::Elements(1));
    group.bench_function("execute_market", |b| {
        b.iter_batched(
            fresh_router,
            |mut router| {
                black_box(router.execute(black_box(&request)));
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn bench_schedule(c: &mut Criterion) {
    let request = market_request();
    let twap = SliceSchedule::twap(Fixed::from_f64(10_000.0), 100, 20);
    let vwap = SliceSchedule::vwap(Fixed::from_f64(10_000.0), 100, 20, u_shaped_volume_curve());
    let mut group = c.benchmark_group("scheduling");
    group.throughput(Throughput::Elements(1));
    group.bench_function("execute_schedule_twap_5_slices", |b| {
        b.iter_batched(
            fresh_router,
            |mut router| {
                black_box(router.execute_schedule(black_box(&request), &twap));
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function("execute_schedule_vwap_5_slices", |b| {
        b.iter_batched(
            fresh_router,
            |mut router| {
                black_box(router.execute_schedule(black_box(&request), &vwap));
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn bench_topology(c: &mut Criterion) {
    c.bench_function("build_topology_6_venues", |b| {
        b.iter(|| {
            let mut registry = SymbolRegistry::new();
            black_box(simulated_topology(&mut registry, &[SYMBOL], 100.0, SEED))
        });
    });
}

criterion_group!(
    benches,
    bench_scoring,
    bench_route,
    bench_execute,
    bench_schedule,
    bench_topology
);
criterion_main!(benches);
