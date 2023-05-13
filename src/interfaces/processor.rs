use std::{collections::VecDeque, sync::Arc};

use crate::fix::fixmessage::FixMessage;
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct FixMsgProcessor {}

impl FixMsgProcessor {
    pub async fn handle_process(
        receiver_queue: Arc<Mutex<VecDeque<FixMessage>>>,
        sender_queue: Arc<Mutex<VecDeque<String>>>,
    ) {
        let mut received_messages = receiver_queue.lock().await;
        let mut messages_to_send = sender_queue.lock().await;
        while let Some(mut message) = received_messages.pop_front() {
            log_info!("[PROCESSOR] Processing message: {:?}", message);
            messages_to_send.push_back(message.encode());
        }
    }

    pub async fn create_processor(
        receiver_queue: Arc<Mutex<VecDeque<FixMessage>>>,
        sender_queue: Arc<Mutex<VecDeque<String>>>,
    ) {
        tokio::spawn(async move {
            log_debug!("[SERVER] Created processor thread");
            loop {
                let receiver_queue = Arc::clone(&receiver_queue);
                let sender_queue = Arc::clone(&sender_queue);
                FixMsgProcessor::handle_process(receiver_queue, sender_queue).await;
            }
        });
    }
}
