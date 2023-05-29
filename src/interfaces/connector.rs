use super::{receiver::FixMsgReceiver, sender::FixMsgSender};
use crate::fix::fixmessage::FixMessage;
use std::{collections::VecDeque, sync::Arc};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Mutex,
};

pub struct FixMsgConnector {}

impl FixMsgConnector {
    pub async fn receiver_thread(
        address: &str,
        receiver_port: u16,
        receiver_queue: Arc<Mutex<VecDeque<FixMessage>>>,
    ) {
        let address = address.to_owned();
        match TcpListener::bind(format!("{}:{}", address, receiver_port)).await {
            Ok(receiver) => {
                tokio::spawn(async move {
                    loop {
                        let receiver_queue = Arc::clone(&receiver_queue);
                        match receiver.accept().await {
                            Ok((socket, addr)) => {
                                let receive_socket = Arc::new(Mutex::new(socket));
                                log_debug!(
                                    "Accepted connection from {}:{}",
                                    addr.ip(),
                                    addr.port()
                                );
                                FixMsgReceiver::create_receiver(receive_socket, receiver_queue)
                                    .await;
                            }
                            Err(e) => {
                                log_error!("Failed to accept: {}", e);
                                continue;
                            }
                        }
                    }
                });
            }
            Err(e) => {
                log_error!("Failed to bind to port: {}", e);
                return;
            }
        };
    }

    pub async fn sender_thread(
        address: &str,
        sender_port: u16,
        sender_queue: Arc<Mutex<VecDeque<String>>>,
    ) {
        let sender_queue = Arc::clone(&sender_queue);
        match TcpStream::connect(format!("{}:{}", address, sender_port)).await {
            Ok(socket) => {
                let send_socket = Arc::new(Mutex::new(socket));
                log_debug!("Connected to sender at {}:{}", address, sender_port);
                FixMsgSender::create_sender(send_socket, sender_queue).await;
            }
            Err(e) => {
                log_warn!("Failed to create sender: {}", e);
                return;
            }
        }
    }

    pub async fn create_connector(
        address: &str,
        receiver_port: u16,
        receiver_queue: Arc<Mutex<VecDeque<FixMessage>>>,
        sender_queue: Arc<Mutex<VecDeque<String>>>,
        sender_port: u16,
    ) {
        let receiver = FixMsgConnector::receiver_thread(address, receiver_port, receiver_queue);
        let sender = FixMsgConnector::sender_thread(address, sender_port, sender_queue);
        tokio::join!(receiver, sender);
    }
}
