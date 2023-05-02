use std::collections::HashSet;

use super::orderbook::OrderBook;
use crate::order::{Order, Side};

pub struct Exchange {
    orderbook: OrderBook,
}

impl Exchange {
    pub fn new() -> Self {
        Exchange {
            orderbook: OrderBook::new(),
        }
    }

    pub fn execute_order(&mut self, order: Order) {
        let symbol = &order.symbol.to_owned();
        self.orderbook.add_order(order);
        self.orderbook.match_orders(symbol);
    }

    pub fn cancel_order(&mut self, order: Order) {
        self.orderbook.remove_order(order);
    }

    pub fn check_execution(&self, order_id: u32) -> HashSet<(Order, Order)> {
        self.orderbook.executions.get_matches_for_id(order_id)
    }

    pub fn get_executions(&self) -> Vec<(Order, Order)> {
        self.orderbook
            .executions
            .matches
            .values()
            .cloned()
            .collect()
    }

    pub fn get_open_orders(&self, symbol: &str) -> Vec<&Order> {
        let mut orders = Vec::new();

        if let Some(buy_list) = self.orderbook.buy_orders.get(symbol) {
            orders.extend(buy_list.iter().map(|order| order));
        }

        if let Some(sell_list) = self.orderbook.sell_orders.get(symbol) {
            orders.extend(sell_list.iter().map(|order| order));
        }

        orders
    }

    pub fn get_active_symbols(&self) -> HashSet<String> {
        let mut symbols = HashSet::new();
        symbols.extend(self.orderbook.buy_orders.keys().cloned());
        symbols.extend(self.orderbook.sell_orders.keys().cloned());
        symbols
    }
}

#[test]
fn test_new_exchange() {
    let exchange = Exchange::new();
    assert!(exchange.get_active_symbols().is_empty());
}

#[test]
fn test_add_orders() {
    let mut exchange = Exchange::new();
    let order = Order::new("AAPL", 100, 150.0, Side::Buy);
    exchange.execute_order(order.clone());
    assert_eq!(exchange.get_open_orders("AAPL").len(), 1);
    assert_eq!(exchange.get_open_orders("AAPL")[0], &order);
}

#[test]
fn test_match_orders() {
    let mut exchange = Exchange::new();
    let order1 = Order::new("AAPL", 10, 100.0, Side::Buy);
    let order2 = Order::new("AAPL", 10, 100.0, Side::Sell);
    exchange.execute_order(order1);
    exchange.execute_order(order2);
    assert_eq!(exchange.get_open_orders("AAPL").len(), 0);
}

#[test]
fn test_cancel_orders() {
    let mut exchange = Exchange::new();
    let order = Order::new("AAPL", 100, 150.0, Side::Buy);
    exchange.execute_order(order.clone());
    exchange.cancel_order(order);
    assert!(exchange.get_open_orders("AAPL").is_empty());
}

#[test]
fn test_get_executions() {
    let mut exchange = Exchange::new();
    let order1 = Order::new("AAPL", 10, 100.0, Side::Buy);
    let order2 = Order::new("AAPL", 10, 100.0, Side::Sell);
    exchange.execute_order(order1.clone());
    exchange.execute_order(order2.clone());
    assert_eq!(exchange.get_executions().len(), 1);
    assert_eq!(exchange.get_executions()[0], (order1, order2));
}

#[test]
fn test_get_open_orders() {
    let mut exchange = Exchange::new();
    let order1 = Order::new("AAPL", 10, 100.0, Side::Buy);
    let order2 = Order::new("AAPL", 10, 100.0, Side::Sell);
    exchange.execute_order(order1.clone());
    exchange.execute_order(order2.clone());
    assert_eq!(exchange.get_open_orders("AAPL").len(), 0);
}

#[test]
fn test_get_active_symbols() {
    let mut exchange = Exchange::new();
    let order1 = Order::new("AAPL", 10, 100.0, Side::Buy);
    let order2 = Order::new("GOOG", 10, 100.0, Side::Sell);
    exchange.execute_order(order1.clone());
    exchange.execute_order(order2.clone());
    let mut symbols = HashSet::new();
    symbols.insert("AAPL".to_string());
    symbols.insert("GOOG".to_string());
    assert_eq!(exchange.get_active_symbols(), symbols);
}
