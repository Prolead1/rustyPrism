//! `rusty_prism` is a market simulation and FIX connectivity toolkit.
//!
//! In addition to the exchange/interface components it ships a Smart Order
//! Router prototype under [`router`].

#[macro_use]
pub mod log;
pub mod exchange;
pub mod fix;
pub mod interfaces;
pub mod order;
pub mod router;
