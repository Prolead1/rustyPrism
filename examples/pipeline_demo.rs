//! Disruptor pipeline demo: several producer threads fan orders into an ingress
//! queue while a single core thread owns the Smart Order Router and executes
//! them in order.
//!
//! Run with:
//! ```bash
//! cargo run --release --example pipeline_demo
//! ```

use rusty_prism::disruptor::{DirectConfig, PipelineConfig, ShardConfig, WaitStrategy};
use rusty_prism::execution::{DirectSorPipeline, ShardedSorPipeline, SorPipeline};
use rusty_prism::order::Side;
use rusty_prism::router::fixed::Fixed;
use rusty_prism::router::sor::{OrderRequest, SmartOrderRouter};
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let producers = 4;
    let orders_per_producer = 5_000;

    let router = SmartOrderRouter::simulated(&["AAPL"], 100.0, 42);
    let config = PipelineConfig {
        ring_capacity: 8192,
        input_capacity: 8192,
        batch_size: 64,
        pin_threads: true,
        ..PipelineConfig::default()
    };
    let pipeline = Arc::new(SorPipeline::spawn(router, config));

    println!(
        "Disruptor pipeline: {} producers x {} orders (ingress {} / ring {} / batch {})",
        producers,
        orders_per_producer,
        config.input_capacity,
        config.ring_capacity,
        config.batch_size
    );
    println!(
        "pinning requested: {} (cores visible to affinity layer: {})",
        config.pin_threads,
        rusty_prism::disruptor::available_core_ids()
    );

    let start = Instant::now();
    let mut threads = Vec::with_capacity(producers);
    // (fan-in timing below includes the synchronous submit round trip)
    for producer in 0..producers {
        let pipeline = Arc::clone(&pipeline);
        threads.push(std::thread::spawn(move || {
            let mut filled = Fixed::ZERO;
            for i in 0..orders_per_producer {
                let quantity = 10.0 + (i % 5) as f64;
                let request =
                    OrderRequest::market("AAPL", Side::Buy, quantity, 100.0).with_adv(5_000_000.0);
                let report = pipeline.submit(request).unwrap();
                filled += report.filled_qty;
            }
            println!("  producer {producer} done");
            filled
        }));
    }

    let total_filled: Fixed = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .fold(Fixed::ZERO, |acc, filled| acc + filled);
    let elapsed = start.elapsed();

    let processed = pipeline.processed();
    let throughput = processed as f64 / elapsed.as_secs_f64();
    println!(
        "\ncore processed {} orders, filled {:.0}, in {:.2?}",
        processed,
        total_filled.to_f64(),
        elapsed
    );
    println!("end-to-end throughput: {:.0} orders/sec", throughput);

    Arc::try_unwrap(pipeline)
        .ok()
        .expect("no outstanding references")
        .shutdown();
    println!("fan-in pipeline shut down cleanly");

    // Direct, single-producer pipeline: no ingester hop, allocation-free.
    let router = SmartOrderRouter::simulated(&["AAPL"], 100.0, 42);
    let symbol = router.symbols().id("AAPL").unwrap();
    let direct = DirectSorPipeline::spawn_for_symbol(
        router,
        symbol,
        DirectConfig {
            wait_strategy: WaitStrategy::BusySpin,
            ..DirectConfig::default()
        },
    );
    let request = OrderRequest::market("AAPL", Side::Buy, 10.0, 100.0).with_adv(5_000_000.0);
    let iterations = 200_000;
    let mut total = std::time::Duration::ZERO;
    for _ in 0..iterations {
        // Clone (and its String allocation) happens outside the timed region.
        let input = request.clone();
        let op = Instant::now();
        let _ = std::hint::black_box(direct.submit(input));
        total += op.elapsed();
    }
    println!(
        "\ndirect pipeline: {} submits, {:.0} ns/op mean (busy-spin, clone excluded)",
        iterations,
        total.as_nanos() as f64 / iterations as f64
    );
    direct.shutdown();
    println!("direct pipeline shut down cleanly");

    // Sharded, multi-producer pipeline: one ring pair per producer.
    let router = SmartOrderRouter::simulated(&["AAPL"], 100.0, 42);
    let symbol = router.symbols().id("AAPL").unwrap();
    let producer_count = 4;
    let per_producer = 5_000;
    let mut sharded = ShardedSorPipeline::spawn_for_symbol(
        router,
        symbol,
        ShardConfig {
            pin_core: true,
            ..ShardConfig::default()
        },
        producer_count,
    );
    let start = Instant::now();
    let handles: Vec<_> = sharded
        .take_producers()
        .into_iter()
        .map(|producer| {
            std::thread::spawn(move || {
                let request =
                    OrderRequest::market("AAPL", Side::Buy, 10.0, 100.0).with_adv(5_000_000.0);
                for _ in 0..per_producer {
                    let _ = std::hint::black_box(producer.submit(request.clone()));
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    let elapsed = start.elapsed();
    let total = (producer_count * per_producer) as f64;
    println!(
        "\nsharded pipeline: {} producers x {} orders in {:.2?} ({:.0} orders/sec)",
        producer_count,
        per_producer,
        elapsed,
        total / elapsed.as_secs_f64()
    );
    sharded.shutdown();
    println!("sharded pipeline shut down cleanly");
}
