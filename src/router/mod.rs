//! Smart Order Router (SOR) prototype.
//!
//! The router consumes an [`sor::OrderRequest`] and produces a cost-minimising
//! allocation across a simulated multi-venue topology. Venues are scored using
//! implied liquidity, volume-based fee tiers, fill probability and a
//! microstructure-aware market impact / queue model. TWAP and VWAP schedules
//! in [`slicing`] break parent orders into configurable child slices.
//!
//! ## Hot-path design
//!
//! - **No floating point on the hot path.** Prices and quantities use the
//!   integer [`fixed::Fixed`] type; costs are milli-basis-points.
//! - **No string keys.** Symbol names are interned once per order via
//!   [`symbol::SymbolRegistry`]; venue books and stats are indexed by dense ids.
//! - **No steady-state allocation.** `route_into` / `execute_into` reuse
//!   caller-owned [`sor::RoutePlan`] and [`sor::ExecutionReport`] buffers.
//!
//! Decision latency is measured with [`std::time::Instant`] at nanosecond
//! resolution so the hot path can be validated against microsecond budgets.

pub mod fixed;
pub mod impact;
pub mod scoring;
pub mod slicing;
pub mod sor;
pub mod symbol;
pub mod topology;
pub mod venue;
