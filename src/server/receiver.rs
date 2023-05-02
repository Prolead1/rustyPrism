use super::processor::FixMsgProcessor;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct FixMsgReceiver {}

impl FixMsgReceiver {
    pub async fn handle_receive(
        processor: Arc<Mutex<FixMsgProcessor>>,
        stream: Arc<Mutex<TcpStream>>,
    ) {
        let mut buffer = Vec::new();

        let mut stream = stream.lock().await;

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
                            "[RECEIVER] Processing message: {} from client: {}",
                            message_str,
                            match stream.peer_addr() {
                                Ok(addr) => addr,
                                Err(err) => {
                                    log_error!("[RECEIVER] Error getting peer address: {}", err,);
                                    std::net::SocketAddr::from(([0, 0, 0, 0], 0))
                                }
                            },
                        );

                        let processor_clone = Arc::clone(&processor);
                        let message_clone = message_str.clone();
                        tokio::spawn(async move {
                            log_debug!(
                                "[RECEIVER] Creating processor thread for message: {}",
                                message_clone,
                            );
                            let mut processor = processor_clone.lock().await;
                            processor.process_message(message_clone).await;
                        });
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
