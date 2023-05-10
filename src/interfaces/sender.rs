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
}
