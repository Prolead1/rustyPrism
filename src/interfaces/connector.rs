use super::{receiver::FixMsgReceiver, sender::FixMsgSender};
use crate::fix::fixmessage::FixMessage;
use std::{collections::VecDeque, sync::Arc};
use tokio::{net::TcpListener, sync::Mutex};

pub struct FixMsgConnector {}

impl FixMsgConnector {
    pub async fn create_connector(
        address: &str,
        receiver_port: u16,
        receiver_queue: Arc<Mutex<VecDeque<FixMessage>>>,
        sender_queue: Arc<Mutex<VecDeque<String>>>,
    ) {
        match TcpListener::bind(format!("{}:{}", address, receiver_port)).await {
            Ok(receiver) => {
                tokio::spawn(async move {
                    loop {
                        match receiver.accept().await {
                            Ok((socket, addr)) => {
                                let receiver_queue = Arc::clone(&receiver_queue);
                                let sender_queue = Arc::clone(&sender_queue);
                                let receive_socket = Arc::new(Mutex::new(socket));
                                log_debug!("[SERVER] Accepted connection from {}", addr);
                                FixMsgSender::create_sender(
                                    &addr.to_string(),
                                    receiver_port + 1,
                                    sender_queue,
                                )
                                .await;
                                FixMsgReceiver::create_receiver(receive_socket, receiver_queue)
                                    .await;
                            }
                            Err(e) => {
                                log_error!("[SERVER] Failed to accept: {}", e);
                                return;
                            }
                        }
                    }
                });
            }
            Err(e) => {
                log_error!("[SERVER] Failed to bind to port: {}", e);
                return;
            }
        };
    }
}
