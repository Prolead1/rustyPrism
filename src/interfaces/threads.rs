use super::processor::FixMsgProcessor;
use super::receiver::FixMsgReceiver;
use super::sender::FixMsgSender;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::Mutex;

pub async fn create_receiver(
    _address: &str,
    _receiver_port: u16,
    _processor: Arc<FixMsgProcessor>,
) {
    match TcpListener::bind(format!("{}:{}", _address, _receiver_port)).await {
        Ok(receiver) => {
            tokio::spawn(async move {
                loop {
                    match receiver.accept().await {
                        Ok((socket, addr)) => {
                            let receive_socket = Arc::new(Mutex::new(socket));
                            let receive_stream = receive_socket.lock().await;

                            log_debug!("[SERVER] Accepted connection from {}", addr);
                            log_debug!("[SERVER] Created receiver thread");

                            let processor = Arc::clone(&_processor);
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
}

pub async fn create_processor(_processor: Arc<FixMsgProcessor>) {
    tokio::spawn(async move {
        log_debug!("[SERVER] Created processor thread");
        loop {
            FixMsgProcessor::handle_process(Arc::clone(&_processor)).await;
        }
    });
}

pub async fn create_sender(_address: &str, _sender_port: u16, _processor: Arc<FixMsgProcessor>) {
    match TcpStream::connect(format!("{}:{}", _address, _sender_port)).await {
        Ok(socket) => {
            let send_socket = Arc::new(Mutex::new(socket));
            tokio::spawn(async move {
                log_debug!("[SERVER] Created sender thread");
                let processor = Arc::clone(&_processor);
                loop {
                    let send_stream = send_socket.lock().await;
                    log_debug!(
                        "[SENDER] Remaining messages to send: {}",
                        processor.messages_to_send.lock().await.len()
                    );
                    match processor.messages_to_send.lock().await.pop_front() {
                        Some(message) => {
                            log_debug!("[SENDER] Message to send: {}", message);
                            FixMsgSender::handle_send(send_stream, &message).await;
                        }
                        None => return,
                    };
                }
            });
        }
        Err(e) => {
            log_warn!("[SERVER] Failed to connect to sender: {}", e);
            return;
        }
    };
}
