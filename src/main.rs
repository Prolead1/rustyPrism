mod client;
mod connector;
mod exchange;
mod executions;
mod fixmessage;
mod fixtag;
mod order;
mod orderbook;
mod processor;
use std::sync::Arc;

use client::FixMsgClient;
use connector::FixMsgServer;
use exchange::Exchange;
use order::{Order, Side};
use tokio::runtime::Runtime;

fn main() {
    let mut exchange = Exchange::new();
    let order1 = Order::new("AAPL", 100, 150.0, Side::Buy);
    let order2 = Order::new("AAPL", 100, 150.0, Side::Sell);
    exchange.execute_order(order1.clone());
    exchange.execute_order(order2.clone());
    exchange.cancel_order(order1.clone());
    exchange.get_open_orders("AAPL");
    exchange.get_active_symbols();
    exchange.get_executions();
    exchange.check_execution(order1.id);

    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        let server = Arc::new(FixMsgServer::new());
        let server_task = tokio::spawn({
            let server = Arc::clone(&server);
            async move {
                server.start("127.0.0.1", 8080).await;
            }
        });
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        let mut client = FixMsgClient::new("127.0.0.1", 8080).await;
        let client_task = client.send_fix_messages("./messages.txt").await;

        if let Err(e) = client_task {
            eprintln!("Error: {}", e);
        }

        // Gracefully stop the server
        drop(server_task);
    });
}
