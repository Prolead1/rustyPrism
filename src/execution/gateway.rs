//! Venue gateways: the boundary between the Smart Order Router and real or
//! simulated trading venues.
//!
//! [`FixVenueGateway`] performs a full in-process FIX round trip: a child order
//! is encoded as a `NewOrderSingle` (MsgType `D`), decoded by the venue, matched
//! against the existing [`Exchange`], and answered with `ExecutionReport`
//! (MsgType `8`) messages that are encoded and decoded again. This exercises the
//! real FIX codec while keeping the venue deterministic and test-friendly.
//!
//! The [`VenueGateway`] trait abstracts this so a TCP/FIX implementation can be
//! dropped in later without touching the router.

use crate::exchange::exchange::Exchange;
use crate::fix::fixmessage::FixMessage;
use crate::fix::fixtag::FixTag;
use crate::order::{Order, Side};
use crate::router::fixed::Fixed;
use crate::router::venue::{fee_amount, PriceLevel, VenueBook, VenueId};
use std::collections::HashMap;

/// Price used for an aggressive market child order (sweeps every resting level).
const AGGRESSIVE_BUY_PRICE: f64 = 1.0e9;
const AGGRESSIVE_SELL_PRICE: f64 = 1.0e-3;

/// Lifecycle status of a child order at a venue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatus {
    New,
    PartiallyFilled,
    Filled,
    Rejected,
}

/// A child order sent to a venue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildOrder {
    pub venue_id: VenueId,
    pub symbol: String,
    pub side: Side,
    pub quantity: Fixed,
    pub limit_price: Option<Fixed>,
}

/// Result of submitting a child order to a venue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VenueExecution {
    pub venue_id: VenueId,
    pub order_id: u32,
    pub status: OrderStatus,
    pub filled_qty: Fixed,
    pub avg_price: Fixed,
    pub fee_millibps: i64,
    pub notional: i128,
}

impl VenueExecution {
    fn rejected(venue_id: VenueId) -> Self {
        VenueExecution {
            venue_id,
            order_id: 0,
            status: OrderStatus::Rejected,
            filled_qty: Fixed::ZERO,
            avg_price: Fixed::ZERO,
            fee_millibps: 0,
            notional: 0,
        }
    }
}

/// A trading venue the router can send child orders to.
pub trait VenueGateway {
    fn venue_id(&self) -> VenueId;
    fn name(&self) -> &str;
    fn submit(&mut self, order: &ChildOrder) -> VenueExecution;
    fn cancel(&mut self, order_id: u32) -> bool;
    /// Current resting book, used to refresh the router's market-data view.
    fn book_snapshot(&self, symbol: &str) -> VenueBook;
}

/// A simulated venue reached through the FIX codec and backed by [`Exchange`].
pub struct FixVenueGateway {
    venue_id: VenueId,
    name: String,
    fee_millibps: i64,
    exchange: Exchange,
    open_orders: HashMap<u32, Order>,
    last_exec_id: u32,
    exec_reports: u32,
}

impl FixVenueGateway {
    pub fn new(venue_id: VenueId, name: &str, fee_millibps: i64) -> Self {
        FixVenueGateway {
            venue_id,
            name: name.to_string(),
            fee_millibps,
            exchange: Exchange::new(),
            open_orders: HashMap::new(),
            last_exec_id: 0,
            exec_reports: 0,
        }
    }

    /// Number of `ExecutionReport` FIX messages encoded and decoded.
    pub fn exec_reports(&self) -> u32 {
        self.exec_reports
    }

    /// Load a venue's displayed book as resting orders so the matching engine
    /// has liquidity to trade against.
    pub fn seed_book(&mut self, symbol: &str, book: &VenueBook) {
        for level in &book.bids {
            self.rest(symbol, level, Side::Buy);
        }
        for level in &book.asks {
            self.rest(symbol, level, Side::Sell);
        }
    }

    fn rest(&mut self, symbol: &str, level: &PriceLevel, side: Side) {
        let quantity = level.size.to_f64().round().max(0.0) as u32;
        if quantity == 0 {
            return;
        }
        let order = Order::new(symbol, quantity, level.price.to_f64(), side);
        self.exchange.execute_order(order);
    }

    /// Encode a `NewOrderSingle`, decode it, and match it.
    fn round_trip_new_order(&self, child: &ChildOrder, price: f64, quantity: u32) -> Option<Order> {
        let mut message = FixMessage::new();
        message.add_field(FixTag::BeginString, "FIX.4.2");
        message.add_field(FixTag::MsgType, "D");
        message.add_field(FixTag::SenderCompID, "SOR");
        message.add_field(FixTag::TargetCompID, &self.name);
        message.add_field(FixTag::Symbol, &child.symbol);
        message.add_field(FixTag::Side, side_to_fix(child.side));
        message.add_field(FixTag::OrderQty, &quantity.to_string());
        message.add_field(FixTag::Price, &format!("{price:.4}"));
        message.add_field(
            FixTag::OrdType,
            if child.limit_price.is_some() {
                "2"
            } else {
                "1"
            },
        );

        let wire = message.encode();
        let received = FixMessage::decode(&wire, "|");
        received.to_order()
    }

    /// Encode and decode an `ExecutionReport`, returning whether all required
    /// tags survived the round trip.
    #[allow(clippy::too_many_arguments)]
    fn round_trip_exec_report(
        &mut self,
        order_id: u32,
        child: &ChildOrder,
        last_qty: u32,
        price: f64,
        cum_qty: u32,
        leaves_qty: u32,
        avg_px: f64,
    ) -> bool {
        self.last_exec_id += 1;
        let mut report = FixMessage::new();
        report.add_field(FixTag::BeginString, "FIX.4.2");
        report.add_field(FixTag::MsgType, "8");
        report.add_field(FixTag::SenderCompID, &self.name);
        report.add_field(FixTag::TargetCompID, "SOR");
        report.add_field(FixTag::OrderID, &order_id.to_string());
        report.add_field(FixTag::ExecID, &self.last_exec_id.to_string());
        report.add_field(FixTag::ExecType, if leaves_qty == 0 { "2" } else { "1" });
        report.add_field(FixTag::Symbol, &child.symbol);
        report.add_field(FixTag::Side, side_to_fix(child.side));
        report.add_field(FixTag::LastQty, &last_qty.to_string());
        report.add_field(FixTag::Price, &format!("{price:.4}"));
        report.add_field(FixTag::CumQty, &cum_qty.to_string());
        report.add_field(FixTag::LeavesQty, &leaves_qty.to_string());
        report.add_field(FixTag::AvgPx, &format!("{avg_px:.4}"));

        let wire = report.encode();
        let decoded = FixMessage::decode(&wire, "|");
        self.exec_reports += 1;

        decoded.fields.contains_key(&FixTag::OrderID)
            && decoded.fields.contains_key(&FixTag::LastQty)
            && decoded.fields.contains_key(&FixTag::CumQty)
            && decoded.fields.contains_key(&FixTag::AvgPx)
    }
}

impl VenueGateway for FixVenueGateway {
    fn venue_id(&self) -> VenueId {
        self.venue_id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn submit(&mut self, child: &ChildOrder) -> VenueExecution {
        let quantity = child.quantity.to_f64().round().max(0.0) as u32;
        if quantity == 0 {
            return VenueExecution::rejected(self.venue_id);
        }
        let price = match child.limit_price {
            Some(limit) => limit.to_f64(),
            None => match child.side {
                Side::Buy => AGGRESSIVE_BUY_PRICE,
                Side::Sell => AGGRESSIVE_SELL_PRICE,
            },
        };

        let matched_order = match self.round_trip_new_order(child, price, quantity) {
            Some(order) => order,
            None => return VenueExecution::rejected(self.venue_id),
        };
        let order_id = matched_order.id;
        self.open_orders.insert(order_id, matched_order.clone());
        self.exchange.execute_order(matched_order);

        let matches = self.exchange.check_execution(order_id);
        let mut filled_units: u32 = 0;
        let mut notional = 0.0_f64;
        for (buy, sell) in matches.iter() {
            let (ours, theirs) = if buy.id == order_id {
                (buy, sell)
            } else {
                (sell, buy)
            };
            let matched = ours.quantity.min(theirs.quantity);
            filled_units += matched;
            notional += matched as f64 * theirs.price;
            self.round_trip_exec_report(
                order_id,
                child,
                matched,
                theirs.price,
                filled_units,
                quantity.saturating_sub(filled_units),
                if filled_units > 0 {
                    notional / filled_units as f64
                } else {
                    0.0
                },
            );
        }

        let filled_qty = Fixed::from_f64(filled_units as f64);
        let avg_price = Fixed::from_f64(if filled_units > 0 {
            notional / filled_units as f64
        } else {
            0.0
        });
        let notional_value = notional.round() as i128;
        let fee_millibps = self.fee_millibps;
        let status = if filled_units == 0 {
            OrderStatus::New
        } else if filled_units < quantity {
            OrderStatus::PartiallyFilled
        } else {
            OrderStatus::Filled
        };

        VenueExecution {
            venue_id: self.venue_id,
            order_id,
            status,
            filled_qty,
            avg_price,
            fee_millibps,
            notional: notional_value,
        }
    }

    fn cancel(&mut self, order_id: u32) -> bool {
        match self.open_orders.remove(&order_id) {
            Some(order) => {
                self.exchange.cancel_order(order);
                true
            }
            None => false,
        }
    }

    fn book_snapshot(&self, symbol: &str) -> VenueBook {
        let mut bids: HashMap<i64, Fixed> = HashMap::new();
        let mut asks: HashMap<i64, Fixed> = HashMap::new();
        for order in self.exchange.get_open_orders(symbol) {
            let price = Fixed::from_f64(order.price);
            let quantity = Fixed::from_f64(order.quantity as f64);
            let side = match order.side {
                Side::Buy => &mut bids,
                Side::Sell => &mut asks,
            };
            *side.entry(price.raw()).or_insert(Fixed::ZERO) += quantity;
        }
        let bids = bids
            .into_iter()
            .map(|(price, size)| PriceLevel::new(Fixed::from_raw(price), size))
            .collect();
        let asks = asks
            .into_iter()
            .map(|(price, size)| PriceLevel::new(Fixed::from_raw(price), size))
            .collect();
        VenueBook::from_levels(bids, asks)
    }
}

/// Fee owed for an execution.
pub fn execution_fee(execution: &VenueExecution) -> i128 {
    fee_amount(execution.notional, execution.fee_millibps)
}

fn side_to_fix(side: Side) -> &'static str {
    match side {
        Side::Buy => "1",
        Side::Sell => "2",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_book() -> VenueBook {
        VenueBook::from_levels(
            vec![
                PriceLevel::from_f64(99.99, 500.0),
                PriceLevel::from_f64(99.98, 800.0),
            ],
            vec![
                PriceLevel::from_f64(100.01, 400.0),
                PriceLevel::from_f64(100.02, 700.0),
            ],
        )
    }

    fn child(side: Side, quantity: f64, limit: Option<f64>) -> ChildOrder {
        ChildOrder {
            venue_id: 0,
            symbol: "AAPL".to_string(),
            side,
            quantity: Fixed::from_f64(quantity),
            limit_price: limit.map(Fixed::from_f64),
        }
    }

    #[test]
    fn test_market_buy_fills_through_fix() {
        let mut gateway = FixVenueGateway::new(0, "XNAS", 2_500);
        gateway.seed_book("AAPL", &sample_book());
        let execution = gateway.submit(&child(Side::Buy, 1_000.0, None));
        assert_eq!(execution.status, OrderStatus::Filled);
        assert_eq!(execution.filled_qty, Fixed::from_f64(1_000.0));
        // 400 @ 100.01 + 600 @ 100.02 = 100016 -> avg 100.016
        assert_eq!(execution.avg_price, Fixed::from_f64(100.016));
        assert_eq!(execution.notional, 100_016);
        assert_eq!(execution_fee(&execution), 25); // 100016 * 2.5bps
        assert!(gateway.exec_reports() > 0);
    }

    #[test]
    fn test_book_snapshot_reflects_consumed_liquidity() {
        let mut gateway = FixVenueGateway::new(0, "XNAS", 0);
        gateway.seed_book("AAPL", &sample_book());
        let before =
            gateway
                .book_snapshot("AAPL")
                .implied_liquidity(Side::Buy, 10, Fixed::from_f64(100.0));
        gateway.submit(&child(Side::Buy, 500.0, None));
        let after =
            gateway
                .book_snapshot("AAPL")
                .implied_liquidity(Side::Buy, 10, Fixed::from_f64(100.0));
        assert!(after < before);
    }

    #[test]
    fn test_passive_limit_rests_then_cancels() {
        let mut gateway = FixVenueGateway::new(0, "XNAS", 0);
        gateway.seed_book("AAPL", &sample_book());
        // Buy below the bid cannot cross.
        let execution = gateway.submit(&child(Side::Buy, 100.0, Some(99.0)));
        assert_eq!(execution.status, OrderStatus::New);
        assert_eq!(execution.filled_qty, Fixed::ZERO);
        assert!(gateway.cancel(execution.order_id));
        assert!(!gateway.cancel(execution.order_id));
    }

    #[test]
    fn test_sell_crosses_bid() {
        let mut gateway = FixVenueGateway::new(0, "XNAS", 0);
        gateway.seed_book("AAPL", &sample_book());
        let execution = gateway.submit(&child(Side::Sell, 500.0, None));
        assert_eq!(execution.filled_qty, Fixed::from_f64(500.0));
        assert_eq!(execution.avg_price, Fixed::from_f64(99.99));
    }
}
