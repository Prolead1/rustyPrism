use std::error::Error;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub struct FixMsgClient {
    stream: TcpStream,
}

impl FixMsgClient {
    pub async fn new(host: &str, port: u16) -> Self {
        let client = FixMsgClient {
            stream: TcpStream::connect(format!("{}:{}", host, port))
                .await
                .expect("[CLIENT] Failed to connect to server"),
        };

        let client_socket_addr = client
            .stream
            .local_addr()
            .expect("[CLIENT] Failed to retrieve local address");

        println!(
            "[CLIENT] Connecting from {}:{} to server at {}:{}...",
            client_socket_addr.ip(),
            client_socket_addr.port(),
            host,
            port
        );

        println!(
            "[CLIENT] Successfully connected to server at {}:{}",
            host, port
        );

        client
    }

    pub async fn send_fix_messages(&mut self, file_path: &str) -> Result<(), Box<dyn Error>> {
        let absolute_path = std::fs::canonicalize(file_path)?;
        println!("[CLIENT] Sending messages from file: {:?}", absolute_path);
        let file = File::open(absolute_path).await?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        while let Some(line) = lines.next_line().await? {
            self.stream.write_all(line.as_bytes()).await?;
        }

        Ok(())
    }
}
