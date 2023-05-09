use super::processor::FixMsgProcessor;
use super::receiver::FixMsgReceiver;
use super::sender::FixMsgSender;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct FixMsgConnector {
    socket: Arc<Mutex<TcpStream>>,
    processor: Arc<FixMsgProcessor>,
}

impl FixMsgConnector {
    pub fn new(socket: Arc<Mutex<TcpStream>>, processor: Arc<FixMsgProcessor>) -> Self {
        FixMsgConnector { socket, processor }
    }

    pub async fn run(&self) {
        let send_processor_ref = Arc::clone(&self.processor);
        let receive_processor_ref = Arc::clone(&self.processor);
        let receive_socket = Arc::clone(&self.socket);
        let send_socket = Arc::clone(&self.socket);

        // Spawn a task for receiving messages
        tokio::spawn(async move {
            log_debug!("[CONNECTOR] Created receiver thread");
            loop {
                let processor = Arc::clone(&send_processor_ref);
                let receive_stream = Arc::clone(&receive_socket);
                FixMsgReceiver::handle_receive(processor, receive_stream).await;
            }
        });

        let processor = Arc::clone(&self.processor);
        // Spawn a task for processing messages
        tokio::spawn(async move {
            log_debug!("[CONNECTOR] Created processor thread");
            loop {
                FixMsgProcessor::handle_process(Arc::clone(&processor)).await;
            }
        });

        // Spawn a task for sending messages
        tokio::spawn(async move {
            log_debug!("[CONNECTOR] Created sender thread");
            loop {
                let processor = Arc::clone(&receive_processor_ref);
                match processor.messages_to_send.lock().await.pop_front() {
                    Some(message) => {
                        log_debug!("[CONNECTOR] Message to send: {}", message);
                        let send_stream = Arc::clone(&send_socket);
                        FixMsgSender::handle_send(send_stream, &message).await;
                    }
                    None => break,
                };
            }
        });
    }
}
