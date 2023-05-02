use super::processor::FixMsgProcessor;
use super::receiver::FixMsgReceiver;
use super::sender::FixMsgSender;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct FixMsgConnector {
    socket: Arc<Mutex<TcpStream>>,
    processor: Arc<Mutex<FixMsgProcessor>>,
}

impl FixMsgConnector {
    pub fn new(socket: Arc<Mutex<TcpStream>>, processor: Arc<Mutex<FixMsgProcessor>>) -> Self {
        FixMsgConnector { socket, processor }
    }

    pub async fn run(&self) {
        let receive_processor = Arc::clone(&self.processor);
        let send_processor = Arc::clone(&self.processor);
        let receive_socket = Arc::clone(&self.socket);
        let send_socket = Arc::clone(&self.socket);

        // Spawn a task for receiving messages
        tokio::spawn(async move {
            log_debug!("[CONNECTOR] Created receiver thread");
            loop {
                let processor_mut = Arc::clone(&receive_processor);
                let receive_stream = Arc::clone(&receive_socket);
                FixMsgReceiver::handle_receive(processor_mut, receive_stream).await;
            }
        });

        // Spawn a task for sending messages
        tokio::spawn(async move {
            log_debug!("[CONNECTOR] Created sender thread");
            loop {
                let message = match send_processor.lock().await.get_message_to_send().await {
                    Some(message) => message,
                    None => continue,
                };
                let send_stream = Arc::clone(&send_socket);
                FixMsgSender::handle_send(send_stream, &message).await;
            }
        });
    }
}
