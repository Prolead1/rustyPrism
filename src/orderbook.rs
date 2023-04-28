use super::order::{Order, Side};
use skiplist::ordered_skiplist::OrderedSkipList;
use std::collections::HashMap;

pub struct OrderBook {
    pub buy_orders: HashMap<String, OrderedSkipList<Order>>,
    pub sell_orders: HashMap<String, OrderedSkipList<Order>>,
}

impl OrderBook {
    pub fn new() -> OrderBook {
        OrderBook {
            buy_orders: HashMap::new(),
            sell_orders: HashMap::new(),
        }
    }

    pub fn add_order(&mut self, order: Order) {
        let (symbol, side) = (&order.symbol.to_owned(), &order.side);

        match side {
            Side::Buy => {
                let buy_orders = self
                    .buy_orders
                    .entry(symbol.to_string())
                    .or_insert(OrderedSkipList::new());
                buy_orders.insert(order);
            }
            Side::Sell => {
                let sell_orders = self
                    .sell_orders
                    .entry(symbol.to_string())
                    .or_insert(OrderedSkipList::new());
                sell_orders.insert(order);
            }
        }
    }

    pub fn remove_order(&mut self, order: Order) -> Option<Order> {
        let (symbol, side) = (&order.symbol.to_owned(), &order.side);

        match side {
            Side::Buy => {
                let buy_orders = self.buy_orders.get_mut(symbol)?;
                let removed = buy_orders.remove(&order);
                if buy_orders.is_empty() {
                    self.buy_orders.remove(symbol);
                }
                removed
            }
            Side::Sell => {
                let sell_orders = self.sell_orders.get_mut(symbol)?;
                let removed = sell_orders.remove(&order);
                if sell_orders.is_empty() {
                    self.sell_orders.remove(symbol);
                }
                removed
            }
        }
    }

    pub fn match_orders(&mut self, symbol: &str) -> Option<HashMap<u32, (Order, Order)>> {
        let buy_orders = self.buy_orders.get_mut(symbol)?;

        let sell_orders = self.sell_orders.get_mut(symbol)?;

        let mut executions: HashMap<u32, (Order, Order)> = HashMap::new();

        while let (Some(mut buy_order), Some(mut sell_order)) =
            (buy_orders.pop_front(), sell_orders.pop_front())
        {
            if buy_order.price >= sell_order.price {
                executions.insert(buy_order.id, (buy_order.to_owned(), sell_order.to_owned()));
                executions.insert(sell_order.id, (buy_order.to_owned(), sell_order.to_owned()));
                if buy_order.quantity > sell_order.quantity {
                    buy_order.quantity -= sell_order.quantity;
                    buy_orders.insert(buy_order);
                    continue;
                } else if buy_order.quantity < sell_order.quantity {
                    sell_order.quantity -= buy_order.quantity;
                    sell_orders.insert(sell_order);
                    continue;
                } else {
                    executions.insert(buy_order.id, (buy_order.to_owned(), sell_order.to_owned()));
                    executions.insert(sell_order.id, (buy_order.to_owned(), sell_order.to_owned()));
                }
            } else {
                if buy_order.quantity > 0 {
                    buy_orders.insert(buy_order);
                }
                if sell_order.quantity > 0 {
                    sell_orders.insert(sell_order);
                }
                break;
            }
        }
        Some(executions)
    }
}

#[test]
fn create_order_book() {
    let order_book = OrderBook::new();
    assert_eq!(order_book.buy_orders.len(), 0);
    assert_eq!(order_book.sell_orders.len(), 0);
}

#[test]
fn test_order_book_add_order() {
    let mut order_book = OrderBook::new();
    let order1 = Order::new("AAPL", 100, 150.0, Side::Buy);
    let order2 = Order::new("AAPL", 100, 150.0, Side::Sell);
    let order3 = Order::new("AAPL", 100, 200.0, Side::Sell);
    order_book.add_order(order1);
    order_book.add_order(order2);
    order_book.add_order(order3);
    assert_eq!(order_book.buy_orders.len(), 1);
    assert_eq!(order_book.sell_orders.len(), 1);
    assert_eq!(order_book.buy_orders.get("AAPL").unwrap().len(), 1);
    assert_eq!(order_book.sell_orders.get("AAPL").unwrap().len(), 2);
}

#[test]
fn test_order_book_remove_order() {
    let mut order_book = OrderBook::new();
    let order1 = Order::new("AAPL", 100, 150.0, Side::Buy);
    let order2 = Order::new("AAPL", 100, 150.0, Side::Sell);
    let order3 = Order::new("AAPL", 100, 200.0, Side::Sell);
    order_book.add_order(order1.clone());
    order_book.add_order(order2.clone());
    order_book.add_order(order3.clone());
    assert_eq!(order_book.buy_orders.len(), 1);
    assert_eq!(order_book.sell_orders.len(), 1);
    assert_eq!(order_book.buy_orders.get("AAPL").unwrap().len(), 1);
    assert_eq!(order_book.sell_orders.get("AAPL").unwrap().len(), 2);
    order_book.remove_order(order1);
    assert_eq!(order_book.buy_orders.len(), 0);
    assert_eq!(order_book.sell_orders.len(), 1);
    assert_eq!(order_book.buy_orders.get("AAPL"), None);
    assert_eq!(order_book.sell_orders.get("AAPL").unwrap().len(), 2);
    order_book.remove_order(order2);
    assert_eq!(order_book.buy_orders.len(), 0);
    assert_eq!(order_book.sell_orders.len(), 1);
    assert_eq!(order_book.buy_orders.get("AAPL"), None);
    assert_eq!(order_book.sell_orders.get("AAPL").unwrap().len(), 1);
    order_book.remove_order(order3);
    assert_eq!(order_book.buy_orders.len(), 0);
    assert_eq!(order_book.sell_orders.len(), 0);
    assert_eq!(order_book.buy_orders.get("AAPL"), None);
    assert_eq!(order_book.sell_orders.get("AAPL"), None);
}

#[test]
fn test_order_book_priority() {
    let mut order_book = OrderBook::new();
    let order1 = Order::new("AAPL", 100, 200.0, Side::Buy);
    let order2 = Order::new("AAPL", 100, 150.0, Side::Buy);
    let order3 = Order::new("AAPL", 100, 200.0, Side::Sell);
    let order4 = Order::new("AAPL", 100, 150.0, Side::Sell);
    order_book.add_order(order1.clone());
    order_book.add_order(order2.clone());
    order_book.add_order(order3.clone());
    order_book.add_order(order4.clone());
    assert_eq!(
        order_book.buy_orders.get("AAPL").unwrap().front().unwrap(),
        &order1
    );
    assert_eq!(
        order_book.buy_orders.get("AAPL").unwrap().back().unwrap(),
        &order2
    );
    assert_eq!(
        order_book.sell_orders.get("AAPL").unwrap().front().unwrap(),
        &order4
    );
    assert_eq!(
        order_book.sell_orders.get("AAPL").unwrap().back().unwrap(),
        &order3
    );
}

#[test]
fn test_match_orders() {
    let mut order_book = OrderBook::new();
    let order1 = Order::new("AAPL", 100, 150.0, Side::Buy);
    let order2 = Order::new("AAPL", 100, 150.0, Side::Buy);
    let order3 = Order::new("AAPL", 100, 150.0, Side::Sell);
    let order4 = Order::new("AAPL", 100, 200.0, Side::Sell);
    order_book.add_order(order1.clone());
    order_book.add_order(order2.clone());
    order_book.add_order(order3.clone());
    order_book.add_order(order4.clone());
    let executions = order_book.match_orders("AAPL");
    assert_eq!(executions.unwrap().get(&order1.id), Some(&(order1, order3)));
}

#[test]
fn test_multiple_match_orders() {
    let mut order_book = OrderBook::new();
    let order1 = Order::new("AAPL", 100, 200.0, Side::Buy);
    let order2 = Order::new("AAPL", 100, 150.0, Side::Buy);
    let order3 = Order::new("AAPL", 100, 200.0, Side::Buy);
    let order4 = Order::new("AAPL", 100, 150.0, Side::Sell);
    let order5 = Order::new("AAPL", 100, 200.0, Side::Sell);
    order_book.add_order(order1.clone());
    order_book.add_order(order2.clone());
    order_book.add_order(order3.clone());
    order_book.add_order(order4.clone());
    order_book.add_order(order5.clone());
    let executions = order_book.match_orders("AAPL").unwrap();
    assert_eq!(executions.get(&order1.id), Some(&(order1, order4)));
    assert_eq!(executions.get(&order3.id), Some(&(order3, order5)));
}

#[test]
fn test_partial_match_orders() {
    let mut order_book = OrderBook::new();
    let order1 = Order::new("AAPL", 100, 200.0, Side::Buy);
    let order2 = Order::new("AAPL", 200, 150.0, Side::Buy);
    let order3 = Order::new("AAPL", 150, 200.0, Side::Sell);
    let order4 = Order::new("AAPL", 300, 300.0, Side::Sell);
    order_book.add_order(order1.clone());
    order_book.add_order(order2.clone());
    order_book.add_order(order3.clone());
    order_book.add_order(order4.clone());
    let executions = order_book.match_orders("AAPL").unwrap();
    assert_eq!(executions.get(&order1.id), Some(&(order1, order3)));
}

#[test]
fn test_multiple_partial_match_orders() {
    let mut order_book = OrderBook::new();
    let order1 = Order::new("AAPL", 100, 200.0, Side::Buy);
    let order2 = Order::new("AAPL", 200, 150.0, Side::Buy);

    let order3 = Order::new("AAPL", 150, 200.0, Side::Sell);

    let order4 = Order::new("AAPL", 300, 300.0, Side::Sell);
    let order5 = Order::new("AAPL", 100, 200.0, Side::Buy);

    let order6 = Order::new("AAPL", 200, 150.0, Side::Buy);
    let order7 = Order::new("AAPL", 150, 200.0, Side::Sell);

    let order8 = Order::new("AAPL", 300, 300.0, Side::Sell);

    order_book.add_order(order1.clone());
    order_book.add_order(order2.clone());
    order_book.add_order(order3.clone());
    order_book.add_order(order4.clone());
    order_book.add_order(order5.clone());
    order_book.add_order(order6.clone());
    order_book.add_order(order7.clone());
    order_book.add_order(order8.clone());
    let executions = order_book.match_orders("AAPL").unwrap();
    assert_eq!(
        executions.get(&order1.id),
        Some(&(order1.clone(), order3.clone()))
    );
    assert_eq!(
        executions.get(&order5.id),
        Some(&(order5.clone(), order7.clone()))
    );
    assert_eq!(
        executions.get(&order5.id),
        Some(&(order5.clone(), order7.clone()))
    );
}
