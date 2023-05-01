use super::processor::FixMsgProcessor;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
            println!("[CONNECTOR] Created receiver thread");
            loop {
                let processor_mut = Arc::clone(&receive_processor);
                let receive_stream = Arc::clone(&receive_socket);
                FixMsgConnector::handle_receive(processor_mut, receive_stream).await;
            }
        });

        // Spawn a task for sending messages
        tokio::spawn(async move {
            println!("[CONNECTOR] Created sender thread");
            loop {
                let message = match send_processor.lock().await.get_message_to_send().await {
                    Some(message) => message,
                    None => continue,
                };
                let send_stream = Arc::clone(&send_socket);
                FixMsgConnector::handle_send(send_stream, &message).await;
            }
        });
    }

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

                        println!(
                            "[CONNECTOR] Processing message: {} from client: {}",
                            message_str,
                            match stream.peer_addr() {
                                Ok(addr) => addr,
                                Err(err) => {
                                    eprintln!("[CONNECTOR] Error getting peer address: {}", err);
                                    std::net::SocketAddr::from(([0, 0, 0, 0], 0))
                                }
                            }
                        );

                        let processor_clone = Arc::clone(&processor);
                        let message_clone = message_str.clone();
                        tokio::spawn(async move {
                            println!(
                                "[CONNECTOR] Creating processor thread for message: {}",
                                message_clone
                            );
                            let mut processor = processor_clone.lock().await;
                            processor.process_message(message_clone).await;
                        });
                    }
                }
                Err(err) => {
                    eprintln!("[CONNECTOR] Error reading from stream: {}", err);
                    break;
                }
            }
        }
    }

    pub async fn handle_send(stream: Arc<Mutex<TcpStream>>, message: &str) {
        let mut stream = stream.lock().await;
        println!(
            "[CONNECTOR] Sending message: {} to client: {}",
            message,
            match stream.peer_addr() {
                Ok(addr) => addr,
                Err(_) => {
                    return;
                }
            }
        );
        match stream.write_all(message.as_bytes()).await {
            Ok(_) => println!("[CONNECTOR] Message sent successfully"),
            Err(err) => eprintln!("[CONNECTOR] Error sending message: {}", err),
        }
    }
}
