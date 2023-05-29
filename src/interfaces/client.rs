use super::connector::FixMsgConnector;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Mutex;

pub struct FixMsgClient {
    sender_queue: Arc<Mutex<VecDeque<String>>>,
    host: String,
    sender_port: u16,
}

impl FixMsgClient {
    pub fn new(host: &str, sender_port: u16) -> Self {
        FixMsgClient {
            sender_queue: Arc::new(Mutex::new(VecDeque::new())),
            host: host.to_owned(),
            sender_port,
        }
    }

    pub async fn run(&mut self, file_path: &str) {
        let host = self.host.clone();
        let sender_port = self.sender_port;
        let sender_queue = Arc::clone(&self.sender_queue);
        self.send_fix_messages(file_path).await;
        FixMsgConnector::sender_thread(&host, sender_port, sender_queue).await;
    }

    pub async fn send_fix_messages(&mut self, file_path: &str) {
        let absolute_path = match std::fs::canonicalize(file_path) {
            Ok(path) => path,
            Err(e) => {
                log_error!("Failed to get absolute path: {}", e);
                return;
            }
        };

        log_debug!("Sending messages from file: {:?}", absolute_path);

        let file = match File::open(absolute_path).await {
            Ok(file) => file,
            Err(e) => {
                log_error!("Failed to open file: {}", e);
                return;
            }
        };

        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        while let Some(line) = match lines.next_line().await {
            Ok(line) => line,
            Err(e) => {
                log_error!("Failed to read line: {}", e);
                return;
            }
        } {
            let mut sender_queue = self.sender_queue.lock().await;
            sender_queue.push_back(line + "\x01");
        }
    }
}
