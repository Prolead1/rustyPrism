use super::connector::FixMsgConnector;
use super::processor::FixMsgProcessor;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
pub struct FixMsgServer {
    processor: Arc<Mutex<FixMsgProcessor>>,
    connectors: Arc<Mutex<Vec<FixMsgConnector>>>,
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
            .expect("Failed to bind");

        loop {
            let (socket, _) = listener.accept().await.expect("Failed to accept");

            let connector =
                FixMsgConnector::new(Arc::new(Mutex::new(socket)), Arc::clone(&self.processor));
            let connector_arc = Arc::new(Mutex::new(connector.clone()));
            self.connectors.lock().await.push(connector.clone());

            tokio::spawn(async move {
                connector_arc.lock().await.run().await;
            });

            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    break;
                }
            }
        }
    }
}
