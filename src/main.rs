#[macro_use]
mod log;
mod exchange;
mod fix;
mod interfaces;
mod order;
use std::sync::Arc;

use interfaces::client::FixMsgClient;
use interfaces::server::FixMsgServer;
use std::env;
use tokio::task;

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

async fn run_client_task(messages_file: &str, server_receiver_port: u16) {
    let mut client = FixMsgClient::new("127.0.0.1", server_receiver_port);
    client.run(messages_file).await;
}

#[tokio::main]
async fn main() {
    env::set_var("APP_LOGLEVEL", "debug");

    let server_task = task::spawn(run_server_task(11));

    let client1_task = task::spawn(run_client_task("./messages.txt", 8080));

    let client2_task = task::spawn(run_client_task("./messages2.txt", 8080));

    let client3_task = task::spawn(run_client_task("./messages2.txt", 8080));

    match tokio::try_join!(server_task, client1_task, client2_task, client3_task,) {
        Ok(_) => log_debug!("All tasks completed successfully"),
        Err(e) => log_error!("Error: {}", e),
    };
}
