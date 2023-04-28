use std::cmp::Reverse;
use std::collections::HashSet;

use super::order::{Order, Side};
use super::orderbook::OrderBook;

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

    pub fn get_symbols(&self) -> HashSet<String> {
        let mut symbols = HashSet::new();
        symbols.extend(self.orderbook.buy_orders.keys().cloned());
        symbols.extend(self.orderbook.sell_orders.keys().cloned());
        symbols
    }
}

#[test]
fn test_new_exchange() {
    let exchange = Exchange::new();
    assert!(exchange.get_open_orders("AAPL").is_empty());
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
