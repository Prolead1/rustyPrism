use super::simulator::{consolidated_mid, MarketSimulator};
use super::{BacktestConfig, Strategy};
use crate::order::Side;
use crate::router::fixed::{diff_millibps, Fixed, MILLI_BPS, PPM, SCALE_SQUARED_I128};
use crate::router::sor::{ExecutionReport, OrderRequest, SmartOrderRouter};
use crate::router::symbol::SymbolId;

/// Execution-quality metrics for a completed backtest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BacktestResult {
    pub side: Side,
    pub requested_qty: Fixed,
    pub executed_qty: Fixed,
    /// Executed fraction of the parent quantity, in ppm.
    pub fill_rate_ppm: i64,
    pub avg_price: Fixed,
    pub arrival_price: Fixed,
    /// Time-weighted mid over the execution horizon (interval VWAP proxy).
    pub interval_vwap: Fixed,
    pub final_mid: Fixed,
    /// Implementation shortfall versus arrival, milli-bps (+ = worse).
    pub implementation_shortfall_millibps: i64,
    /// Performance versus the interval VWAP, milli-bps (+ = worse).
    pub vs_vwap_millibps: i64,
    pub total_fees: i128,
    pub num_child_orders: usize,
    /// Consolidated mid at each simulator step, including the arrival.
    pub mid_path: Vec<Fixed>,
}

impl BacktestResult {
    pub fn implementation_shortfall_bps(&self) -> f64 {
        self.implementation_shortfall_millibps as f64 / MILLI_BPS as f64
    }

    pub fn vs_vwap_bps(&self) -> f64 {
        self.vs_vwap_millibps as f64 / MILLI_BPS as f64
    }

    pub fn fill_rate(&self) -> f64 {
        self.fill_rate_ppm as f64 / PPM as f64
    }

    /// One-line, human-readable summary.
    pub fn summary(&self) -> String {
        format!(
            "filled {:.0}/{:.0} ({:.0}%)  avg={}  arrival={}  vwap={}  IS={:+.3}bp  vsVWAP={:+.3}bp  fees={}  children={}",
            self.executed_qty.to_f64(),
            self.requested_qty.to_f64(),
            self.fill_rate() * 100.0,
            self.avg_price,
            self.arrival_price,
            self.interval_vwap,
            self.implementation_shortfall_bps(),
            self.vs_vwap_bps(),
            self.total_fees,
            self.num_child_orders,
        )
    }
}

/// Run a strategy through the simulator and router.
///
/// The arrival price is captured before the first market step. At each step the
/// simulator advances the venue books and any child orders scheduled for that
/// instant are executed.
pub fn run_backtest(
    router: &mut SmartOrderRouter,
    simulator: &mut MarketSimulator,
    symbol: SymbolId,
    request: &OrderRequest,
    strategy: &Strategy,
    config: &BacktestConfig,
) -> BacktestResult {
    let arrival_price = consolidated_mid(router, symbol);
    let mut mid_path = vec![arrival_price];
    let mut reports: Vec<ExecutionReport> = Vec::new();

    match strategy {
        Strategy::Market => {
            simulator.step(router);
            mid_path.push(consolidated_mid(router, symbol));
            reports.push(router.execute(request));
        }
        Strategy::Schedule(schedule) => {
            let step_seconds = config.step_seconds.max(1);
            let steps = (schedule.duration_s / step_seconds).max(1);
            let mut slice_index = 0usize;
            for step_index in 0..=steps {
                simulator.step(router);
                mid_path.push(consolidated_mid(router, symbol));
                let now = step_index * step_seconds;
                while slice_index < schedule.slices.len()
                    && schedule.slices[slice_index].scheduled_at_s <= now
                {
                    let mut child = request.clone();
                    child.quantity = schedule.slices[slice_index].quantity;
                    child.horizon_s = step_seconds;
                    reports.push(router.execute(&child));
                    slice_index += 1;
                }
            }
        }
    }

    let requested_qty = strategy.total_quantity(request);
    let mut executed_qty = Fixed::ZERO;
    let mut notional: i128 = 0;
    let mut total_fees: i128 = 0;
    for report in &reports {
        executed_qty += report.filled_qty;
        notional += report.notional_value;
        total_fees += report.total_fees;
    }
    let avg_price = if executed_qty.raw() > 0 {
        Fixed::from_raw((notional * SCALE_SQUARED_I128 / executed_qty.raw() as i128) as i64)
    } else {
        Fixed::ZERO
    };
    let interval_vwap = if mid_path.is_empty() {
        arrival_price
    } else {
        let sum: i128 = mid_path.iter().map(|mid| mid.raw() as i128).sum();
        Fixed::from_raw((sum / mid_path.len() as i128) as i64)
    };
    let implementation_shortfall_millibps =
        signed_diff_millibps(request.side, avg_price, arrival_price, executed_qty);
    let vs_vwap_millibps =
        signed_diff_millibps(request.side, avg_price, interval_vwap, executed_qty);
    let fill_rate_ppm = executed_qty.ratio_ppm(requested_qty).clamp(0, PPM);

    BacktestResult {
        side: request.side,
        requested_qty,
        executed_qty,
        fill_rate_ppm,
        avg_price,
        arrival_price,
        interval_vwap,
        final_mid: mid_path.last().copied().unwrap_or(arrival_price),
        implementation_shortfall_millibps,
        vs_vwap_millibps,
        total_fees,
        num_child_orders: reports.len(),
        mid_path,
    }
}

/// Signed difference versus a benchmark, milli-bps, positive = worse for `side`.
fn signed_diff_millibps(side: Side, price: Fixed, benchmark: Fixed, executed: Fixed) -> i64 {
    if executed.raw() <= 0 || benchmark.raw() <= 0 || price.raw() <= 0 {
        return 0;
    }
    let raw = diff_millibps(price, benchmark);
    match side {
        Side::Buy => raw,
        Side::Sell => -raw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::fixed::Fixed;
    use crate::router::slicing::SliceSchedule;

    fn setup_with(config: BacktestConfig) -> (SmartOrderRouter, MarketSimulator, SymbolId) {
        let router = SmartOrderRouter::simulated(&["AAPL"], 100.0, 42);
        let symbol = router.symbols().id("AAPL").unwrap();
        let simulator = MarketSimulator::from_router(&router, symbol, &config);
        (router, simulator, symbol)
    }

    fn setup(seed: u64) -> (SmartOrderRouter, MarketSimulator, SymbolId) {
        setup_with(BacktestConfig {
            seed,
            ..BacktestConfig::default()
        })
    }

    #[test]
    fn test_market_backtest_fills_small_order() {
        let (mut router, mut sim, symbol) = setup(7);
        let request = OrderRequest::market("AAPL", Side::Buy, 100.0, 100.0).with_adv(5_000_000.0);
        let config = BacktestConfig::default();
        let result = run_backtest(
            &mut router,
            &mut sim,
            symbol,
            &request,
            &Strategy::Market,
            &config,
        );
        assert_eq!(result.executed_qty, Fixed::from_f64(100.0));
        assert_eq!(result.fill_rate_ppm, PPM);
        assert_eq!(result.num_child_orders, 1);
        assert!(result.implementation_shortfall_millibps >= 0);
    }

    #[test]
    fn test_backtest_is_deterministic() {
        let config = BacktestConfig::default();
        let run = || {
            let (mut router, mut sim, symbol) = setup(11);
            let request =
                OrderRequest::market("AAPL", Side::Buy, 1_000.0, 100.0).with_adv(5_000_000.0);
            let schedule = SliceSchedule::twap(Fixed::from_f64(1_000.0), 40, 10);
            run_backtest(
                &mut router,
                &mut sim,
                symbol,
                &request,
                &Strategy::Schedule(schedule),
                &config,
            )
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn test_schedule_backtest_runs_slices() {
        let (mut router, mut sim, symbol) = setup(3);
        let request = OrderRequest::market("AAPL", Side::Buy, 1_000.0, 100.0).with_adv(5_000_000.0);
        // 50 seconds at 10-second intervals -> 5 slices.
        let schedule = SliceSchedule::twap(Fixed::from_f64(1_000.0), 50, 10);
        let config = BacktestConfig::default();
        let result = run_backtest(
            &mut router,
            &mut sim,
            symbol,
            &request,
            &Strategy::Schedule(schedule),
            &config,
        );
        assert_eq!(result.num_child_orders, 5);
        assert!(result.mid_path.len() > 1);
        assert!(result.interval_vwap.is_positive());
        assert!(result.executed_qty.is_positive());
    }

    #[test]
    fn test_sell_implementation_shortfall_sign() {
        // Zero volatility so the mid is unchanged and the sell must cross the
        // bid, guaranteeing a positive implementation shortfall.
        let config = BacktestConfig {
            vol_bps_per_step: 0.0,
            seed: 5,
            ..BacktestConfig::default()
        };
        let (mut router, mut sim, symbol) = setup_with(config);
        let request = OrderRequest::market("AAPL", Side::Sell, 100.0, 100.0).with_adv(5_000_000.0);
        let result = run_backtest(
            &mut router,
            &mut sim,
            symbol,
            &request,
            &Strategy::Market,
            &config,
        );
        // Selling crosses the bid, so the average price is below mid and the
        // shortfall is positive.
        assert!(result.avg_price < result.arrival_price);
        assert!(result.implementation_shortfall_millibps > 0);
    }
}
