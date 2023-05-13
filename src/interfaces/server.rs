use super::{processor::FixMsgProcessor, receiver::FixMsgReceiver, sender::FixMsgSender};
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

        FixMsgReceiver::create_receiver(address, receiver_port, receive_processor).await;

        create_processor(processor).await;

        FixMsgSender::create_sender(address, sender_port, send_processor).await;
    }
}

pub async fn create_processor(_processor: Arc<FixMsgProcessor>) {
    tokio::spawn(async move {
        log_debug!("[SERVER] Created processor thread");
        loop {
            FixMsgProcessor::handle_process(Arc::clone(&_processor)).await;
        }
    });
}
