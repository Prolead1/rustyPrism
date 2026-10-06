//! End-to-end demo: the Smart Order Router sends child orders to simulated
//! venues over the FIX codec, then folds venue book state back into the router.
//!
//! Run with:
//! ```bash
//! cargo run --release --example fix_integration_demo
//! ```

use rusty_prism::execution::IntegratedRouter;
use rusty_prism::order::Side;
use rusty_prism::router::fixed::Fixed;
use rusty_prism::router::slicing::SliceSchedule;
use rusty_prism::router::sor::OrderRequest;

fn main() {
    let mut integrated = IntegratedRouter::from_topology(&["AAPL"], 100.0, 42);
    let symbol = integrated.router().symbols().id("AAPL").unwrap();

    println!("Venues (gateway <-> router)");
    for gateway in integrated.gateways() {
        println!(
            "  {:<6} id={} taker={}mbp",
            gateway.name(),
            gateway.venue_id(),
            // Look up the router's fee view for display.
            integrated
                .router()
                .venues()
                .iter()
                .find(|venue| venue.id == gateway.venue_id())
                .map(|venue| venue.effective_fees().taker_millibps)
                .unwrap_or(0),
        );
    }

    let before: Fixed = integrated
        .router()
        .venues()
        .iter()
        .filter_map(|venue| venue.book(symbol))
        .fold(Fixed::ZERO, |acc, book| {
            acc + book
                .asks
                .iter()
                .fold(Fixed::ZERO, |inner, level| inner + level.size)
        });
    println!(
        "\nConsolidated displayed asks before: {:.0}",
        before.to_f64()
    );

    let request = OrderRequest::market("AAPL", Side::Buy, 5_000.0, 100.0).with_adv(5_000_000.0);
    let report = integrated.execute(&request);

    println!("\nMarket order executed over FIX");
    println!(
        "  filled {:.0}/{:.0}  avg={}  slippage={:+.3}bp  fees={}",
        report.filled_qty.to_f64(),
        report.requested_qty.to_f64(),
        report.avg_price,
        report.realized_slippage_bps(),
        report.total_fees,
    );
    println!("  child order ExecutionReports:");
    for execution in &report.executions {
        println!(
            "    venue={} order_id={} status={:?} filled={:.0} avg={}",
            execution.venue_id,
            execution.order_id,
            execution.status,
            execution.filled_qty.to_f64(),
            execution.avg_price,
        );
    }

    integrated.sync_market_data(symbol);
    let after: Fixed = integrated
        .router()
        .venues()
        .iter()
        .filter_map(|venue| venue.book(symbol))
        .fold(Fixed::ZERO, |acc, book| {
            acc + book
                .asks
                .iter()
                .fold(Fixed::ZERO, |inner, level| inner + level.size)
        });
    println!(
        "  consolidated displayed asks after sync: {:.0}",
        after.to_f64()
    );

    let schedule = SliceSchedule::twap(Fixed::from_f64(2_000.0), 50, 10);
    let children = integrated.execute_schedule(&request, &schedule);
    let total: Fixed = children
        .iter()
        .fold(Fixed::ZERO, |acc, child| acc + child.filled_qty);
    println!(
        "\nTWAP schedule: {} children, filled {:.0} total",
        children.len(),
        total.to_f64()
    );
}
