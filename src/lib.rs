//! `rusty_prism` is a market simulation and FIX connectivity toolkit.
//!
//! In addition to the exchange/interface components it ships a Smart Order
//! Router prototype under [`router`].

// `unsafe` is denied crate-wide but explicitly allowed inside the Disruptor
// ring, which is the only place that needs manual synchronisation.
#![deny(unsafe_code)]

#[macro_use]
pub mod log;
pub mod backtest;
pub mod disruptor;
pub mod exchange;
pub mod execution;
pub mod fix;
pub mod interfaces;
pub mod order;
pub mod router;
