use super::processor::FixMsgProcessor;
use super::threads::{create_processor, create_receiver, create_sender};
use std::sync::Arc;
pub struct FixMsgServer {
    processor: Arc<FixMsgProcessor>,
}

impl FixMsgServer {
    pub fn new() -> Self {
        FixMsgServer {
            processor: Arc::new(FixMsgProcessor::new()),
        }
    }

    pub async fn start(&self, address: &str, receiver_port: u16, sender_port: u16) {
        let receive_processor = Arc::clone(&self.processor);
        let processor = Arc::clone(&self.processor);
        let send_processor = Arc::clone(&self.processor);

        create_receiver(address, receiver_port, receive_processor).await;

        create_processor(processor).await;

        create_sender(address, sender_port, send_processor).await;
    }
}
