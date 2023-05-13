use super::{connector::FixMsgConnector, processor::FixMsgProcessor};
use crate::fix::fixmessage::FixMessage;
use std::{collections::VecDeque, sync::Arc};
use tokio::sync::Mutex;
pub struct FixMsgServer {
    receiver_queue: Arc<Mutex<VecDeque<FixMessage>>>,
    sender_queue: Arc<Mutex<VecDeque<String>>>,
}

impl FixMsgServer {
    pub fn new() -> Self {
        FixMsgServer {
            receiver_queue: Arc::new(Mutex::new(VecDeque::new())),
            sender_queue: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub async fn start(&self, address: &str, receiver_port: u16) {
        let receiver_queue = Arc::clone(&self.receiver_queue);
        let sender_queue = Arc::clone(&self.sender_queue);

        let processor_receiver_queue = Arc::clone(&self.receiver_queue);
        let processor_sender_queue = Arc::clone(&self.sender_queue);

        FixMsgConnector::create_connector(address, receiver_port, receiver_queue, sender_queue)
            .await;

        FixMsgProcessor::create_processor(processor_receiver_queue, processor_sender_queue).await;
    }
}
