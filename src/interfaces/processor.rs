use std::{collections::VecDeque, sync::Arc};

use crate::fix::fixmessage::FixMessage;
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct FixMsgProcessor {
    pub received_messages: Arc<Mutex<VecDeque<FixMessage>>>,
    pub messages_to_send: Arc<Mutex<VecDeque<String>>>,
}

impl FixMsgProcessor {
    pub fn new() -> Self {
        FixMsgProcessor {
            received_messages: Arc::new(Mutex::new(VecDeque::new())),
            messages_to_send: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub async fn handle_process(processor: Arc<FixMsgProcessor>) {
        let mut received_messages = processor.received_messages.lock().await;
        let mut messages_to_send = processor.messages_to_send.lock().await;
        while let Some(mut message) = received_messages.pop_front() {
            log_info!("[PROCESSOR] Processing message: {:?}", message);
            messages_to_send.push_back(message.encode());
        }
    }
}

#[tokio::test]
async fn test_new() {
    let processor = FixMsgProcessor::new();
    assert!(processor.received_messages.lock().await.is_empty());
    assert!(processor.messages_to_send.lock().await.is_empty());
}
