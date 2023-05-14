#[macro_use]
mod log;
mod exchange;
mod fix;
mod interfaces;
mod order;
use std::sync::Arc;

use exchange::exchange::Exchange;
use interfaces::client::FixMsgClient;
use interfaces::server::FixMsgServer;
use order::{Order, Side};
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

async fn run_client_task(messages_file: &str, sender_port: u16) {
    let mut client = FixMsgClient::new("127.0.0.1", sender_port);
    client.run(messages_file).await;
}

#[tokio::main]
async fn main() {
    let exchange_task = task::spawn(run_exchange_tasks());

    let server_task = task::spawn(run_server_task(11));

    let client1_task = task::spawn(run_client_task("./messages.txt", 8080));

    let client2_task = task::spawn(run_client_task("./messages2.txt", 8080));

    let client3_task = task::spawn(run_client_task("./messages2.txt", 8080));

    match tokio::try_join!(
        exchange_task,
        server_task,
        client1_task,
        client2_task,
        client3_task
    ) {
        Ok(_) => log_debug!("All tasks completed successfully"),
        Err(e) => log_error!("Error: {}", e),
    };
}
