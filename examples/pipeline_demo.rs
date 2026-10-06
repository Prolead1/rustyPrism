//! Disruptor pipeline demo: several producer threads fan orders into an ingress
//! queue while a single core thread owns the Smart Order Router and executes
//! them in order.
//!
//! Run with:
//! ```bash
//! cargo run --release --example pipeline_demo
//! ```

use rusty_prism::disruptor::PipelineConfig;
use rusty_prism::execution::SorPipeline;
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
    println!("pipeline shut down cleanly");
}
