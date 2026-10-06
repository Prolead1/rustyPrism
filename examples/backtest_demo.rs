//! Backtest demo: compares market, TWAP and VWAP execution of the same parent
//! order through the Smart Order Router on a seeded market.
//!
//! Run with:
//! ```bash
//! cargo run --release --example backtest_demo
//! ```

use rusty_prism::backtest::{run_backtest, BacktestConfig, BacktestResult, Strategy};
use rusty_prism::order::Side;
use rusty_prism::router::fixed::Fixed;
use rusty_prism::router::slicing::{u_shaped_volume_curve, SliceSchedule};
use rusty_prism::router::sor::{OrderRequest, SmartOrderRouter};

const QUANTITY: f64 = 10_000.0;

fn run(label: &str, strategy: Strategy, config: &BacktestConfig) -> BacktestResult {
    let mut router = SmartOrderRouter::simulated(&["AAPL"], 100.0, 42);
    let symbol = router.symbols().id("AAPL").unwrap();
    let mut simulator =
        rusty_prism::backtest::MarketSimulator::from_router(&router, symbol, config);
    let request = OrderRequest::market("AAPL", Side::Buy, QUANTITY, 100.0).with_adv(5_000_000.0);
    let result = run_backtest(
        &mut router,
        &mut simulator,
        symbol,
        &request,
        &strategy,
        config,
    );
    println!("{label:<20} {}", result.summary());
    result
}

fn main() {
    let config = BacktestConfig::default();
    println!(
        "Backtest: buy {:.0} AAPL, step={}s vol={:.1}bp/step seed={}\n",
        QUANTITY, config.step_seconds, config.vol_bps_per_step, config.seed
    );

    let market = run("Market (immediate)", Strategy::Market, &config);
    let twap = run(
        "TWAP (5 x 10s)",
        Strategy::Schedule(SliceSchedule::twap(Fixed::from_f64(QUANTITY), 50, 10)),
        &config,
    );
    let vwap = run(
        "VWAP (5 x 10s)",
        Strategy::Schedule(SliceSchedule::vwap(
            Fixed::from_f64(QUANTITY),
            50,
            10,
            u_shaped_volume_curve(),
        )),
        &config,
    );

    println!("\nComparison (bps vs arrival)");
    println!(
        "  {:<20} {:>10} {:>12} {:>9}",
        "strategy", "IS(bp)", "vsVWAP(bp)", "fill %"
    );
    for (label, result) in [("Market", &market), ("TWAP", &twap), ("VWAP", &vwap)] {
        println!(
            "  {:<20} {:>10.3} {:>12.3} {:>8.1}%",
            label,
            result.implementation_shortfall_bps(),
            result.vs_vwap_bps(),
            result.fill_rate() * 100.0,
        );
    }
}
