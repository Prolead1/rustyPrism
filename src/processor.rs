use std::sync::Arc;

use super::fixmessage::FixMessage;
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct FixMsgProcessor {
    received_messages: Arc<Mutex<Vec<FixMessage>>>,
    pub message_to_send: Arc<Mutex<String>>,
}

impl FixMsgProcessor {
    pub fn new() -> Self {
        FixMsgProcessor {
            received_messages: Arc::new(Mutex::new(Vec::new())),
            message_to_send: Arc::new(Mutex::new(String::new())),
        }
    }

    pub async fn process_message(&mut self, message: String) {
        println!("[PROCESSOR] Processing message: {}", message);
        let mut fix_message = FixMessage::decode(&message, "|");
        println!("[PROCESSOR] Decoded message: {:?}", fix_message);
        self.received_messages
            .lock()
            .await
            .push(fix_message.clone());
        self.message_to_send = Arc::new(Mutex::new(fix_message.encode()));
        println!(
            "[PROCESSOR] Updated message: {}",
            self.message_to_send.lock().await
        );
        println!("[PROCESSOR] Message processing finished");
    }

    pub async fn get_message_to_send(&self) -> Option<String> {
        if self.message_to_send.lock().await.is_empty() {
            return None;
        }
        Some(self.message_to_send.lock().await.clone())
    }
}
