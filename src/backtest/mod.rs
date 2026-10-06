//! Backtesting and execution-quality evaluation for the Smart Order Router.
//!
//! The backtest drives a [`sor::SmartOrderRouter`](crate::router::sor::SmartOrderRouter)
//! through a seeded market simulator. At every step the venue books are
//! regenerated around an evolving mid, then any child orders scheduled for that
//! instant are executed through the router. The result reports standard
//! execution-quality metrics: implementation shortfall versus the arrival
//! price, performance versus the interval VWAP, fill rate and fees.
//!
//! Everything is deterministic for a given seed, so results are reproducible.

pub mod runner;
pub mod simulator;

pub use runner::{run_backtest, BacktestResult};
pub use simulator::MarketSimulator;

use crate::router::fixed::Fixed;

/// Configuration for a backtest run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BacktestConfig {
    /// Simulated seconds between market updates.
    pub step_seconds: i64,
    /// Per-step volatility in basis points (one standard deviation).
    pub vol_bps_per_step: f64,
    /// Per-step drift in basis points.
    pub drift_bps_per_step: f64,
    /// Seed for the deterministic price path.
    pub seed: u64,
}

impl Default for BacktestConfig {
    fn default() -> Self {
        BacktestConfig {
            step_seconds: 5,
            vol_bps_per_step: 4.0,
            drift_bps_per_step: 0.0,
            seed: 7,
        }
    }
}

/// A strategy to backtest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Strategy {
    /// Execute the whole parent order immediately.
    Market,
    /// Execute a pre-built slicing schedule.
    Schedule(crate::router::slicing::SliceSchedule),
}

impl Strategy {
    pub fn total_quantity(&self, request: &crate::router::sor::OrderRequest) -> Fixed {
        match self {
            Strategy::Market => request.quantity,
            Strategy::Schedule(schedule) => schedule.total_quantity,
        }
    }
}
