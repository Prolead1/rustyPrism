use super::processor::FixMsgProcessor;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::MutexGuard;
pub struct FixMsgSender {}

impl FixMsgSender {
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

    pub async fn handle_send_all(
        processor: Arc<FixMsgProcessor>,
        send_stream: MutexGuard<'_, TcpStream>,
    ) {
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
}
