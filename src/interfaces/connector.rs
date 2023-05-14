use super::{receiver::FixMsgReceiver, sender::FixMsgSender};
use crate::fix::fixmessage::FixMessage;
use std::{collections::VecDeque, sync::Arc};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Mutex,
};

pub struct FixMsgConnector {}

impl FixMsgConnector {
    pub async fn create_connector(
        address: &str,
        receiver_port: u16,
        receiver_queue: Arc<Mutex<VecDeque<FixMessage>>>,
        sender_queue: Arc<Mutex<VecDeque<String>>>,
        sender_port: u16,
    ) {
        match TcpListener::bind(format!("{}:{}", address, receiver_port)).await {
            Ok(receiver) => {
                let address = address.to_owned();
                tokio::spawn(async move {
                    loop {
                        let receiver_queue = Arc::clone(&receiver_queue);
                        match receiver.accept().await {
                            Ok((socket, addr)) => {
                                let receive_socket = Arc::new(Mutex::new(socket));
                                log_debug!(
                                    "[CONNECTOR] Accepted connection from {}:{}",
                                    addr.ip(),
                                    addr.port()
                                );
                                FixMsgReceiver::create_receiver(receive_socket, receiver_queue)
                                    .await;
                            }
                            Err(e) => {
                                log_error!("[CONNECTOR] Failed to accept: {}", e);
                                continue;
                            }
                        }

                        let sender_queue = Arc::clone(&sender_queue);
                        match TcpStream::connect(format!("{}:{}", address, sender_port)).await {
                            Ok(socket) => {
                                let send_socket = Arc::new(Mutex::new(socket));
                                log_debug!(
                                    "[CONNECTOR] Connected to sender at {}:{}",
                                    address,
                                    sender_port
                                );
                                FixMsgSender::create_sender(send_socket, sender_queue).await;
                            }
                            Err(e) => {
                                log_warn!("[CONNECTOR] Failed to create sender: {}", e);
                                return;
                            }
                        };
                    }
                });
            }
            Err(e) => {
                log_error!("[CONNECTOR] Failed to bind to port: {}", e);
                return;
            }
        };
    }
}
