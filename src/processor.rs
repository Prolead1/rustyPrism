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

#[tokio::test]
async fn test_new() {
    let processor = FixMsgProcessor::new();
    assert!(processor.received_messages.lock().await.is_empty());
    assert!(processor.message_to_send.lock().await.is_empty());
}

#[tokio::test]
async fn test_process_message() {
    use super::fixmessage::FixMessage;
    let mut processor = FixMsgProcessor::new();
    let message = String::from(
        "8=FIX.4.2|9=70|35=A|49=SERVER|56=CLIENT|34=1|52=20210201-00:00:00.000|98=0|108=30|10=000|\x01",
    );
    let expected_message = String::from(format!(
        "8=FIX.4.2|9=70|35=A|49=SERVER|56=CLIENT|34=1|52={}|10=000|\x01",
        FixMessage::get_time()
    ));
    let expected_fix_message = FixMessage::decode(&message, "|");
    processor.process_message(message).await;
    assert_eq!(
        processor.get_message_to_send().await.unwrap(),
        expected_message
    );
    assert_eq!(
        processor.received_messages.lock().await[0],
        expected_fix_message
    );
}
