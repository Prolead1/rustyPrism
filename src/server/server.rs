use super::processor::FixMsgProcessor;
use super::receiver::FixMsgReceiver;
use super::sender::FixMsgSender;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
pub struct FixMsgServer {
    processor: Arc<FixMsgProcessor>,
}

impl FixMsgServer {
    pub fn new() -> Self {
        FixMsgServer {
            processor: Arc::new(FixMsgProcessor::new()),
        }
    }

    pub async fn start(&self, address: &str, receiver_port: u16, sender_port: u16) {
        let receive_processor = Arc::clone(&self.processor);
        let processor = Arc::clone(&self.processor);
        let send_processor = Arc::clone(&self.processor);

        match TcpListener::bind(format!("{}:{}", address, receiver_port)).await {
            Ok(receiver) => {
                tokio::spawn(async move {
                    loop {
                        match receiver.accept().await {
                            Ok((socket, addr)) => {
                                let receive_socket = Arc::new(Mutex::new(socket));
                                let receive_stream = receive_socket.lock().await;

                                log_debug!("[SERVER] Accepted connection from {}", addr);
                                log_debug!("[SERVER] Created receiver thread");

                                let processor = Arc::clone(&receive_processor);
                                FixMsgReceiver::handle_receive(processor, receive_stream).await;
                            }
                            Err(e) => {
                                log_error!("[SERVER] Failed to accept: {}", e);
                                continue;
                            }
                        };
                    }
                });
            }
            Err(e) => {
                log_error!("[SERVER] Failed to bind to port: {}", e);
                return;
            }
        };

        tokio::spawn(async move {
            log_debug!("[SERVER] Created processor thread");
            loop {
                FixMsgProcessor::handle_process(Arc::clone(&processor)).await;
            }
        });

        match TcpStream::connect(format!("{}:{}", address, sender_port)).await {
            Ok(socket) => {
                let send_socket = Arc::new(Mutex::new(socket));
                tokio::spawn(async move {
                    log_debug!("[SERVER] Created sender thread");
                    loop {
                        let processor = Arc::clone(&send_processor);
                        let send_stream = send_socket.lock().await;
                        FixMsgSender::handle_send_all(processor, send_stream).await;
                    }
                });
            }
            Err(e) => {
                log_warn!("[SERVER] Failed to connect to sender: {}", e);
                return;
            }
        };
    }
}
