use std::collections::VecDeque;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::sync::MutexGuard;
pub struct FixMsgSender {}

impl FixMsgSender {
    pub async fn create_sender(
        send_socket: Arc<Mutex<TcpStream>>,
        sender_queue: Arc<Mutex<VecDeque<String>>>,
    ) {
        tokio::spawn(async move {
            log_debug!("Created sender thread");
            loop {
                let send_stream = send_socket.lock().await;
                // log_debug!(
                //     "Remaining messages to send: {}",
                //     sender_queue.lock().await.len()
                // );
                match sender_queue.lock().await.pop_front() {
                    Some(message) => {
                        // log_debug!("Message to send: {}", message);
                        FixMsgSender::handle_send(send_stream, &message).await;
                    }
                    None => return,
                };
            }
        });
    }

    pub async fn handle_send(mut stream: MutexGuard<'_, TcpStream>, message: &str) {
        // log_debug!(
        //     "Sending message: {} to client: {}",
        //     message,
        //     match stream.peer_addr() {
        //         Ok(addr) => addr,
        //         Err(_) => {
        //             return;
        //         }
        //     },
        // );
        match stream.write_all(message.as_bytes()).await {
            Ok(_) => {
                // log_debug!("Message sent successfully")
            }
            Err(err) => {
                log_error!("Error sending message: {}", err)
            }
        }
    }
}
