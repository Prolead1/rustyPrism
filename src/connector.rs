use super::processor::FixMsgProcessor;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::select;
use tokio::sync::Mutex;
use tokio::task::spawn;

pub struct FixMsgServer {
    processor: Arc<FixMsgProcessor>,
    connectors: Arc<Mutex<Vec<FixMsgConnector>>>,
}

impl FixMsgServer {
    pub fn new() -> Self {
        FixMsgServer {
            processor: Arc::new(FixMsgProcessor::new()),
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

            let task = async move {
                connector_arc.lock().await.run().await;
            };

            spawn(task);

            select! {
                _ = tokio::signal::ctrl_c() => {
                    break;
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct FixMsgConnector {
    socket: Arc<Mutex<TcpStream>>,
    processor: Arc<FixMsgProcessor>,
}

impl FixMsgConnector {
    pub fn new(socket: Arc<Mutex<TcpStream>>, processor: Arc<FixMsgProcessor>) -> Self {
        FixMsgConnector { socket, processor }
    }

    pub async fn run(&self) {
        let receive_stream = self.socket.clone();
        let connector = self.clone();

        // Spawn a task for receiving messages
        spawn(async move {
            loop {
                FixMsgConnector::handle_receive(&connector, receive_stream.clone()).await;
            }
        });
    }

    pub async fn handle_receive(connector: &FixMsgConnector, stream: Arc<Mutex<TcpStream>>) {
        let mut stream = stream.lock().await;
        let mut buffer = [0u8; 1024];
        if let Ok(bytes_read) = stream.read(&mut buffer).await {
            let received_message = String::from_utf8_lossy(&buffer[..bytes_read]);
            println!("Received message: {}", received_message);
            connector
                .processor
                .process_message(Arc::new(connector.to_owned()), received_message.to_string());
        }
    }

    pub async fn handle_send(connector: &FixMsgConnector, message: &str) {
        let mut stream = connector.socket.lock().await;
        if let Err(err) = stream.write_all(message.as_bytes()).await {
            eprintln!("Error sending message: {}", err);
        }
    }
}
