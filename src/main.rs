mod exchange;
use exchange::{Order, Side, OrderBook};

fn main() {
    // Exchange::new();
    let mut orderbook = OrderBook::new();
    let order1 = Order::new("AAPL", 100, 150.0, Side::Buy);
    let order2 = Order::new("AAPL", 100, 150.0, Side::Sell);
    orderbook.add_order(order1.clone());
    orderbook.add_order(order2);
    orderbook.remove_order(order1);
}

// #[test]
// fn test_new_exchange() {
//     let exchange = Exchange::new();
//     assert!(exchange.orderbook.buyOrders.is_empty());
//     assert!(exchange.orderbook.sellOrders.is_empty());
// }

// #[test]
// fn test_add_orders() {
//     let mut exchange = Exchange::new();
//     let order = Order::new("AAPL", 100, 150.0, Side::Buy);
//     exchange.execute_order(order.clone());
//     assert_eq!(exchange.get_orders("AAPL").len(), 1);
//     assert_eq!(exchange.get_orders("AAPL")[0], &order);
// }

// #[test]
// fn test_match_orders() {
//     let mut exchange = Exchange::new();
//     let order1 = Order::new("AAPL", 10, 100.0, Side::Buy);
//     let order2 = Order::new("AAPL", 10, 100.0, Side::Sell);
//     exchange.execute_order(order1);
//     exchange.execute_order(order2);
//     assert_eq!(exchange.get_orders("AAPL").len(), 0);
// }