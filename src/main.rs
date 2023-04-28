mod orderbook;
mod order;
mod exchange;
use exchange::Exchange;
use orderbook::{OrderBook};
use order::{Order, Side};

fn main() {
    let mut exchange = Exchange::new();
    let order1 = Order::new("AAPL", 100, 150.0, Side::Buy);
    exchange.execute_order(order1.clone());
    exchange.get_open_orders("AAPL");
    exchange.get_symbols();
}
