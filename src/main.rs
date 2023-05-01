mod client;
mod connector;
mod exchange;
mod executions;
mod fixmessage;
mod fixtag;
mod order;
mod orderbook;
mod processor;
mod server;
use std::sync::Arc;

use client::FixMsgClient;
use exchange::Exchange;
use order::{Order, Side};
use server::FixMsgServer;
use tokio::task;

async fn run_exchange_tasks() {
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
}

async fn run_server_task(seconds: u64) {
    let server = Arc::new(FixMsgServer::new());
    let server_task = tokio::spawn({
        let server = Arc::clone(&server);
        async move {
            server.start("127.0.0.1", 8080).await;
        }
    });
    tokio::time::sleep(tokio::time::Duration::from_secs(seconds)).await;
    drop(server_task);
}

async fn run_client_task(messages_file: &str) {
    let mut client = FixMsgClient::new("127.0.0.1", 8080).await;
    let client_send = client.send_fix_messages(messages_file).await;
    if let Err(e) = client_send {
        eprintln!("Error: {}", e);
    }
}

#[tokio::main]
async fn main() {
    let exchange_task = task::spawn(run_exchange_tasks());

    let server_task = task::spawn(run_server_task(30));

    let client1_task = task::spawn(run_client_task("./messages.txt"));

    let client2_task = task::spawn(run_client_task("./messages2.txt"));

    tokio::try_join!(exchange_task, server_task, client1_task, client2_task).unwrap();
}
