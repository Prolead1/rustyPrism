use super::processor::FixMsgProcessor;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, MutexGuard};

#[derive(Clone)]
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
            loop {
                let processor_mut = receive_processor.lock().await;
                let receive_stream = receive_socket.lock().await;
                FixMsgConnector::handle_receive(processor_mut, receive_stream).await;
                println!("Message processed successfully");
            }
        });

        // Spawn a task for sending messages
        tokio::spawn(async move {
            loop {
                let message = match send_processor.lock().await.get_message_to_send().await {
                    Some(message) => message,
                    None => continue,
                };
                println!("Sending message: {}", message);
                let send_stream = send_socket.lock().await;
                FixMsgConnector::handle_send(send_stream, &message).await;
                println!("Message sent successfully");
            }
        });
    }

    pub async fn handle_receive(
        mut processor: MutexGuard<'_, FixMsgProcessor>,
        mut stream: MutexGuard<'_, TcpStream>,
    ) {
        let mut buffer = [0u8; 1024];
        if let Ok(bytes_read) = stream.read(&mut buffer).await {
            let received_message = String::from_utf8_lossy(&buffer[..bytes_read]);
            println!("Received message: {}", received_message);
            processor
                .process_message(received_message.to_string())
                .await;
        }
    }

    pub async fn handle_send(mut stream: MutexGuard<'_, TcpStream>, message: &str) {
        if let Err(err) = stream.write_all(message.as_bytes()).await {
            eprintln!("Error sending message: {}", err);
        }
    }
}
