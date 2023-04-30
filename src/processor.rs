use std::sync::Arc;

use super::connector::FixMsgConnector;
use super::fixmessage::FixMessage;

pub struct FixMsgProcessor {}

impl FixMsgProcessor {
    pub fn new() -> Self {
        FixMsgProcessor {}
    }

    pub fn process_message(&self, connector: Arc<FixMsgConnector>, message: String) {
        println!("Processing message: {}", message);
        let fix_message = FixMessage::decode(&message, "|");
        println!("Decoded message: {:?}", fix_message);
        tokio::spawn(async move { FixMsgConnector::handle_send(&connector, &message).await });
    }
}
