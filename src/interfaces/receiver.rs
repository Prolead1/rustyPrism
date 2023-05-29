use crate::fix::fixmessage::FixMessage;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct FixMsgReceiver {}

impl FixMsgReceiver {
    pub async fn create_receiver(
        receive_socket: Arc<Mutex<TcpStream>>,
        receiver_queue: Arc<Mutex<VecDeque<FixMessage>>>,
    ) {
        tokio::spawn(async move {
            log_debug!("Created receiver thread");
            let receiver_queue = Arc::clone(&receiver_queue);
            FixMsgReceiver::handle_receive(receiver_queue, receive_socket).await;
        });
    }

    pub async fn handle_receive(
        receiver_queue: Arc<Mutex<VecDeque<FixMessage>>>,
        receive_socket: Arc<Mutex<TcpStream>>,
    ) {
        let mut buffer = Vec::new();
        let mut stream = receive_socket.lock().await;

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

                        log_debug!(
                            "Received message: {} from: {}",
                            message_str,
                            match stream.peer_addr() {
                                Ok(addr) => addr,
                                Err(err) => {
                                    log_error!("Error getting peer address: {}", err,);
                                    std::net::SocketAddr::from(([0, 0, 0, 0], 0))
                                }
                            },
                        );

                        let decoded_message = FixMessage::decode(&message_str, "|");

                        receiver_queue.lock().await.push_back(decoded_message);
                    }
                }
                Err(err) => {
                    log_error!("Error reading from stream: {}", err);
                    break;
                }
            }
        }
    }
}
