//! Smart Order Router prototype demo.
//!
//! Run with:
//! ```bash
//! cargo run --release --example sor_demo
//! ```

use rusty_prism::order::Side;
use rusty_prism::router::fixed::Fixed;
use rusty_prism::router::slicing::{u_shaped_volume_curve, SliceSchedule};
use rusty_prism::router::sor::{OrderRequest, RouterConfig, SmartOrderRouter};
use rusty_prism::router::symbol::SymbolId;

fn print_topology(router: &SmartOrderRouter, symbol: SymbolId) {
    println!("Simulated venue topology");
    println!(
        "  {:<6} {:>9} {:>9} {:>13} {:>11} {:>12}",
        "venue", "latency", "fill_p", "taker(bps)", "spread(bp)", "touch depth"
    );
    for venue in router.venues() {
        let book = venue.book(symbol).unwrap();
        println!(
            "  {:<6} {:>6}us {:>9.3} {:>13.3} {:>9.3} {:>12.0}",
            venue.name,
            venue.latency_us,
            venue.fill_probability_ppm as f64 / 1_000_000.0,
            venue.effective_fees().taker_millibps as f64 / 1_000.0,
            book.spread_millibps() as f64 / 1_000.0,
            book.asks
                .first()
                .map(|level| level.size.to_f64())
                .unwrap_or(0.0),
        );
    }
}

fn demo_scoring_and_execution() {
    let config = RouterConfig {
        max_participation_ppm: 50_000,
        ..RouterConfig::default()
    };
    let mut router = SmartOrderRouter::simulated(&["AAPL"], 100.0, 42).with_config(config);
    let aapl = router.symbols().id("AAPL").unwrap();

    print_topology(&router, aapl);

    let request = OrderRequest::market("AAPL", Side::Buy, 5_000.0, 100.0).with_adv(2_000_000.0);

    println!(
        "\nPer-venue scores for {:.0} AAPL @ 100.00",
        request.quantity.to_f64()
    );
    println!(
        "  {:<6} {:>11} {:>10} {:>10} {:>9} {:>12} {:>9}",
        "venue", "slip(mbp)", "fee(mbp)", "impact", "fill_p", "implied liq", "score"
    );
    for score in router.score_destinations(&request) {
        println!(
            "  {:<6} {:>11} {:>10} {:>10} {:>9.3} {:>12.0} {:>9}",
            router.venues()[score.venue_id].name,
            score.slippage_millibps,
            score.net_fee_millibps,
            score.impact_millibps,
            score.fill_probability_ppm as f64 / 1_000_000.0,
            score.liquidity_available.to_f64(),
            score.score,
        );
    }

    let report = router.execute(&request);
    println!("\nMarket order execution");
    println!(
        "  requested={:.0}  filled={:.0}  avg_px={}  slippage={:.3}bp  fees={}  decision={}ns",
        report.requested_qty.to_f64(),
        report.filled_qty.to_f64(),
        report.avg_price,
        report.realized_slippage_bps(),
        report.total_fees,
        report.decision_latency_ns,
    );
    print!("  child routes: ");
    for fill in &report.fills {
        print!(
            "{}x{}@{}  ",
            router.venues()[fill.venue_id].name,
            fill.quantity,
            fill.price
        );
    }
    println!();

    let stats = router.stats();
    println!(
        "\nRouter stats: orders={} avg_decision={:.0}ns max_decision={}ns",
        stats.orders,
        stats.avg_decision_ns(),
        stats.max_decision_ns,
    );
}

fn demo_schedule(label: &str, schedule: SliceSchedule, adaptive: bool) {
    let config = RouterConfig {
        max_participation_ppm: 50_000,
        adaptive_slicing: adaptive,
        ..RouterConfig::default()
    };
    let mut router = SmartOrderRouter::simulated(&["AAPL"], 100.0, 42).with_config(config);

    let request = OrderRequest::market("AAPL", Side::Buy, 10_000.0, 100.0).with_adv(5_000_000.0);
    let report = router.execute_schedule(&request, &schedule);

    println!("\n{label} execution");
    println!(
        "  slices={} filled={:.0}/{:.0} avg_px={} slippage={:.3}bp fees={}",
        report.num_slices(),
        report.total_filled.to_f64(),
        report.total_requested.to_f64(),
        report.avg_price,
        report.realized_slippage_bps(),
        report.total_fees,
    );
    println!(
        "  decision latency: avg={}ns max={}ns",
        report.avg_decision_latency_ns, report.max_decision_latency_ns,
    );
}

fn main() {
    demo_scoring_and_execution();
    demo_schedule(
        "TWAP",
        SliceSchedule::twap(Fixed::from_f64(10_000.0), 100, 20),
        false,
    );
    demo_schedule(
        "VWAP (adaptive)",
        SliceSchedule::vwap(Fixed::from_f64(10_000.0), 100, 20, u_shaped_volume_curve()),
        true,
    );
}
