mod exchange;
mod order;
mod orderbook;
use exchange::Exchange;
use order::{Order, Side};

fn main() {
    let mut exchange = Exchange::new();
    let order1 = Order::new("AAPL", 100, 150.0, Side::Buy);
    let order2 = Order::new("AAPL", 100, 150.0, Side::Sell);
    exchange.execute_order(order1.clone());
    exchange.execute_order(order2.clone());
    exchange.get_open_orders("AAPL");
    exchange.get_active_symbols();
}
