use std::{collections::VecDeque, sync::Arc};

use crate::{
    exchange::exchange::Exchange,
    fix::{fixmessage::FixMessage, fixtag::FixTag},
    order::Order,
};
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
        let mut exchange = Exchange::new();
        while let Some(mut message) = received_messages.pop_front() {
            log_info!("[PROCESSOR] Processing message: {:?}", message);
            let order: Order = match message.to_order() {
                Some(order) => order,
                None => {
                    log_error!("[PROCESSOR] Error converting message to order");
                    continue;
                }
            };
            exchange.execute_order(order);
            message.modify_field(FixTag::SenderCompID, "SERVER");
            message.modify_field(FixTag::TargetCompID, "CLIENT");
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
