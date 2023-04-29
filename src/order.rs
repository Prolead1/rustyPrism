use std::hash::Hash;
use std::sync::atomic::AtomicU32;

#[derive(Debug, PartialEq, Clone)]
pub enum Side {
    Buy,
    Sell,
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
    pub id: u32,
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
            other
                .price
                .partial_cmp(&self.price)
                .unwrap()
                .then_with(|| self.id.cmp(&other.id))
        } else {
            self.price
                .partial_cmp(&other.price)
                .unwrap()
                .then_with(|| self.id.cmp(&other.id))
        }
    }
}

impl Hash for Order {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[test]
fn test_order_cmp() {
    let order1 = Order::new("AAPL", 100, 100.0, Side::Buy);
    let order2 = Order::new("AAPL", 100, 150.0, Side::Sell);
    let order3 = Order::new("AAPL", 100, 200.0, Side::Sell);
    let order4 = Order::new("AAPL", 100, 200.0, Side::Buy);
    assert_eq!(order1.cmp(&order4), std::cmp::Ordering::Greater);
    assert_eq!(order3.cmp(&order2), std::cmp::Ordering::Greater);
}

#[test]
fn test_order_clone() {
    let order1 = Order::new("AAPL", 100, 150.0, Side::Buy);
    assert_eq!(order1.clone(), order1);
}
