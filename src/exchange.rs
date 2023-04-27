use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::atomic::AtomicU32;
use skiplist::ordered_skiplist::OrderedSkipList;

#[derive(Debug,PartialEq,Clone)]
pub enum Side {
    Buy,
    Sell
}

#[test]
fn test_side_eq() {
    let buy_side = Side::Buy;
    let sell_side = Side::Sell;
    assert_eq!(buy_side, Side::Buy);
    assert_eq!(sell_side, Side::Sell);
}

#[test]
fn test_side_clone() {
    let buy_side = Side::Buy;
    let sell_side = Side::Sell;
    assert_eq!(buy_side.clone(), Side::Buy);
    assert_eq!(sell_side.clone(), Side::Sell);
}

#[derive(Debug, Clone)]
pub struct Order {
    id: u32,
    pub symbol: String,
    pub quantity: u32,
    pub price: f64,
    pub side: Side,
}

impl Order {
    pub fn new(symbol: &str, quantity: u32, price: f64, side: Side) -> Order {
        static NEXT_ID: AtomicU32 = AtomicU32::new(0);
        let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Order {
            id,
            symbol: symbol.to_string(),
            quantity,
            price,
            side,
        }
    }
}

impl Eq for Order {}

impl PartialEq for Order {
    fn eq(&self, other: &Order) -> bool {
        self.id == other.id
    }
}

impl PartialOrd for Order {
    fn partial_cmp(&self, other: &Order) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Order {
    fn cmp(&self, other: &Order) -> std::cmp::Ordering {
        if self.side == Side::Buy {
            other.price
                .partial_cmp(&self.price)
                .unwrap().then_with(|| other.id.cmp(&self.id))
        } else {
            self.price
                .partial_cmp(&other.price)
                .unwrap().then_with(|| other.id.cmp(&self.id))
        }
    }
}

#[test]
fn test_order_cmp() {
    let order1 = Order::new("AAPL", 100, 150.0, Side::Buy);
    let order2 = Order::new("AAPL", 100, 150.0, Side::Sell);
    assert_eq!(order1.cmp(&order2), std::cmp::Ordering::Less);
    assert_eq!(order2.cmp(&order1), std::cmp::Ordering::Greater);
    let order3 = Order::new("AAPL", 100, 200.0, Side::Sell);
    assert_eq!(order3.cmp(&order2), std::cmp::Ordering::Less);
    assert_eq!(order3.cmp(&order1), std::cmp::Ordering::Less);
}

#[test]
fn test_order_clone() {
    let order1 = Order::new("AAPL", 100, 150.0, Side::Buy);
    assert_eq!(order1.clone(), order1);
}

pub struct OrderBook {
    buy_orders: HashMap<String, OrderedSkipList<Reverse<Order>>>,
    sell_orders: HashMap<String, OrderedSkipList<Order>>,
}

impl OrderBook
{
    pub fn new() -> OrderBook {
        OrderBook {
            buy_orders: HashMap::new(),
            sell_orders: HashMap::new(),
        }
    }

    pub fn add_order(&mut self, order: Order) {
        let symbol = order.symbol.clone();
        let side = &order.side;

        match side {
            Side::Buy => {
                let buy_list = self.buy_orders.entry(symbol.clone()).or_insert(OrderedSkipList::new());
                buy_list.insert(Reverse(order));
            }
            Side::Sell => {
                let sell_list = self.sell_orders.entry(symbol.clone()).or_insert(OrderedSkipList::new());
                sell_list.insert(order);
            }
        }
    }

    pub fn remove_order(&mut self, order: Order) -> Option<Order> {
        let (symbol, side) = (order.symbol.clone(), &order.side);
        match side {
            Side::Buy => {
                let buy_orders = self.buy_orders.get_mut(&symbol)?;
                let removed = buy_orders.remove(&Reverse(order)).map(|Reverse(order)| order);
                if buy_orders.is_empty() {
                    self.buy_orders.remove(&symbol);
                }
                removed
            }
            Side::Sell => {
                let sell_orders = self.sell_orders.get_mut(&symbol)?;
                let removed = sell_orders.remove(&order);
                if sell_orders.is_empty() {
                    self.sell_orders.remove(&symbol);
                }
                removed
            }
        }
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
    let order1 = Order::new("AAPL", 100, 150.0, Side::Buy);
    let order2 = Order::new("AAPL", 100, 150.0, Side::Buy);
    let order3 = Order::new("AAPL", 100, 150.0, Side::Sell);
    let order4 = Order::new("AAPL", 100, 200.0, Side::Sell);
    order_book.add_order(order1.clone());
    order_book.add_order(order2.clone());
    order_book.add_order(order3.clone());
    order_book.add_order(order4.clone());
    println!("{:?}", order_book.buy_orders.get("AAPL").unwrap());
    println!("{:?}", order_book.sell_orders.get("AAPL").unwrap());
    assert_eq!(order_book.buy_orders.get("AAPL").unwrap().front().unwrap(), &Reverse(order1));
    assert_eq!(order_book.buy_orders.get("AAPL").unwrap().back().unwrap(), &Reverse(order2));
    assert_eq!(order_book.sell_orders.get("AAPL").unwrap().front().unwrap(), &order3);
    assert_eq!(order_book.sell_orders.get("AAPL").unwrap().back().unwrap(), &order4);
}