use super::connector::FixMsgConnector;
use super::processor::FixMsgProcessor;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
pub struct FixMsgServer {
    processor: Arc<Mutex<FixMsgProcessor>>,
    connectors: Arc<Mutex<Vec<Arc<Mutex<FixMsgConnector>>>>>,
}

impl FixMsgServer {
    pub fn new() -> Self {
        FixMsgServer {
            processor: Arc::new(Mutex::new(FixMsgProcessor::new())),
            connectors: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn start(&self, address: &str, port: u16) {
        let listener = TcpListener::bind(format!("{}:{}", address, port))
            .await
            .expect("[SERVER] Failed to bind");

        loop {
            let (socket, addr) = listener.accept().await.expect("[SERVER] Failed to accept");
            log_debug!("[SERVER] Accepted connection from {}", addr);

            let processor = Arc::clone(&self.processor);
            let connectors = Arc::clone(&self.connectors);

            tokio::spawn(async move {
                log_debug!("[SERVER] Spawning connector");
                let connector = Arc::new(Mutex::new(FixMsgConnector::new(
                    Arc::new(Mutex::new(socket)),
                    processor,
                )));
                log_debug!("[SERVER] Connector spawned, running...");
                let _ = connector.lock().await.run().await;

                let mut connectors = connectors.lock().await;
                connectors.retain(|c| !Arc::ptr_eq(c, &connector));
                log_debug!("[SERVER] Connector removed from the list");
            });
        }
    }
}
