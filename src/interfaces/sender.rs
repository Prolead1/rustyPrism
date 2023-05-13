use std::collections::VecDeque;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::sync::MutexGuard;
pub struct FixMsgSender {}

impl FixMsgSender {
    pub async fn create_sender(
        address: &str,
        sender_port: u16,
        sender_queue: Arc<Mutex<VecDeque<String>>>,
    ) {
        match TcpStream::connect(format!("{}:{}", address, sender_port)).await {
            Ok(socket) => {
                let send_socket = Arc::new(Mutex::new(socket));
                tokio::spawn(async move {
                    log_debug!("[SERVER] Created sender thread");
                    loop {
                        let send_stream = send_socket.lock().await;
                        log_debug!(
                            "[SENDER] Remaining messages to send: {}",
                            sender_queue.lock().await.len()
                        );
                        match sender_queue.lock().await.pop_front() {
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

    pub async fn handle_send(mut stream: MutexGuard<'_, TcpStream>, message: &str) {
        log_info!(
            "[SENDER] Sending message: {} to client: {}",
            message,
            match stream.peer_addr() {
                Ok(addr) => addr,
                Err(_) => {
                    return;
                }
            },
        );
        match stream.write_all(message.as_bytes()).await {
            Ok(_) => log_debug!("[SENDER] Message sent successfully"),
            Err(err) => log_error!("[SENDER] Error sending message: {}", err),
        }
    }
}
