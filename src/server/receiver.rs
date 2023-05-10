use super::processor::FixMsgProcessor;
use crate::fix::fixmessage::FixMessage;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::sync::MutexGuard;

#[derive(Debug)]
pub struct FixMsgReceiver {}

impl FixMsgReceiver {
    pub async fn handle_receive(
        processor: Arc<FixMsgProcessor>,
        mut stream: MutexGuard<'_, TcpStream>,
    ) {
        let mut buffer = Vec::new();

        loop {
            let mut chunk = vec![0u8; 1024];

            match stream.read(&mut chunk).await {
                Ok(bytes_read) => {
                    if bytes_read == 0 {
                        break;
                    }

                    buffer.extend_from_slice(&chunk[..bytes_read]);

                    while let Some(index) = buffer.iter().position(|&byte| byte == b'\x01') {
                        let current_message = buffer[..index].to_owned();
                        buffer = buffer[index + 1..].to_owned();

                        let message_str = String::from_utf8_lossy(&current_message).to_string();

                        log_info!(
                            "[RECEIVER] Received message: {} from client: {}",
                            message_str,
                            match stream.peer_addr() {
                                Ok(addr) => addr,
                                Err(err) => {
                                    log_error!("[RECEIVER] Error getting peer address: {}", err,);
                                    std::net::SocketAddr::from(([0, 0, 0, 0], 0))
                                }
                            },
                        );

                        let decoded_message = FixMessage::decode(&message_str, "|");

                        processor
                            .received_messages
                            .lock()
                            .await
                            .push_back(decoded_message);
                    }
                }
                Err(err) => {
                    log_error!("[RECEIVER] Error reading from stream: {}", err);
                    break;
                }
            }
        }
    }
}
