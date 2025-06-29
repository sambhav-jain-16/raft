use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use crate::types::NodeId;
use crate::messages::RaftMessage;
use std::time::Instant;
use std::time::Duration;
use serde_json;
use tokio::io::AsyncBufReadExt;

pub struct Connection {
    stream: TcpStream,
    last_heartbeat: Instant,
    is_active: bool,
    retry_count: u32,
    last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum RetryStrategy {
    Static{delay :Duration},
    Exponential{base :Duration, max :Duration},
}

impl RetryStrategy {
    pub fn calculate_retry_delay(&self, attempts: u32) -> Duration {
        match self {
            RetryStrategy::Static{delay} => *delay,
            RetryStrategy::Exponential{base, max} => {
                let delay = *base * (2_u32.pow(attempts));
                std::cmp::min(delay, *max)
            }
        }
    }
}

pub struct Network {
    node_id: NodeId,
    listener: TcpListener,
    connections: HashMap<NodeId, Connection>,
    node_addresses: HashMap<NodeId, SocketAddr>,
    message_sender: mpsc::Sender<RaftMessage>,
    message_receiver: mpsc::Receiver<RaftMessage>,
    connection_timeout: Duration,
    retry_count: u32,
    retry_strategy: RetryStrategy,
}

impl Network {
    pub async fn new(node_id: NodeId, listen_addr: SocketAddr, retry_strategy: RetryStrategy) -> Result<Self, String> {

        let listener = TcpListener::bind(listen_addr)
            .await
            .map_err(|e| format!("Failed to bind to {}: {}", listen_addr, e))?;
        
            let (message_sender, message_receiver) = mpsc::channel(100);

            Ok (Self{
                node_id,
                listener,
                connections: HashMap::new(),
                node_addresses: HashMap::new(),
                message_sender,
                message_receiver: message_receiver,
                connection_timeout: Duration::from_secs(10),
                retry_count: 3,
                retry_strategy,
            })
    }
    
    pub fn add_node(&mut self, node_id: NodeId, address: SocketAddr) {
        self.node_addresses.insert(node_id, address);
    }
    
    pub fn remove_node(&mut self, node_id: NodeId) {
        self.connections.remove(&node_id);
    }
    
    pub async fn connect_to_node(&mut self, node_id: NodeId) -> Result<(), String> {
        let address = self.node_addresses.get(&node_id).ok_or_else(|| format!("Node {} not found", node_id))?;

        let stream = TcpStream::connect(address).await.map_err(|e| format!("Failed to connect to {}: {}", address, e))?;

        let connection = Connection {
            stream,
            last_heartbeat: Instant::now(),
            is_active: true,
            retry_count: 0,
            last_error: None,
        };

        self.connections.insert(node_id, connection);
        Ok(())
    }

    pub async fn send_message(&mut self, node_id: NodeId, message: RaftMessage) -> Result<(), String> {
        self.send_with_retry(node_id, message).await
    }

    pub async fn send_with_retry(&mut self, node_id: NodeId, message: RaftMessage) -> Result<(), String> {
        let mut attempts = 0;

        while attempts < self.retry_count {
            match self.send_to_node(node_id, &message).await {
                Ok(_) => {
                    return Ok(());
                }
                Err(_e) => {
                    attempts += 1;
                    if attempts < self.retry_count {
                        let delay = self.retry_strategy.calculate_retry_delay(attempts);
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }
        Err(format!("Failed to send message to {}: {}", node_id, "Max retry attempts reached"))
    }

    pub async fn send_to_node(&mut self, node_id: NodeId, message: &RaftMessage) -> Result<(), String> {
        let connection = self.connections.get_mut(&node_id).ok_or_else(|| format!("Node {} not found", node_id))?;

        let message_json = serde_json::to_string(message).map_err(|e| format!("Failed to serialize message: {}", e))?;

        let message_with_new_line = format!("{}\n", message_json);

        use tokio::io::AsyncWriteExt;
        connection.stream.write_all(message_with_new_line.as_bytes()).await.map_err(|e| format!("Failed to send message to {}: {}", node_id, e))?;

        Ok(())
    }

    

    pub async fn start_listening(&mut self) -> Result<(), String> {
        loop {
            match self.listener.accept().await {
                Ok((socket, addr)) => {
                    let _ = self.handle_new_connection(socket, addr).await;
                }
                Err(e) => {
                    return Err(format!("Failed to accept connection: {}", e));
                }
            }
        }
    }

    pub async fn handle_new_connection(&mut self, socket: TcpStream, addr: SocketAddr) -> Result<(), String> {
        let message_sender = self.message_sender.clone();

        tokio::spawn(async move {
            if let Err(e) = Self::handle_connection_loop(socket, message_sender).await {
                eprintln!("Connection error: {}", e);
            }
        });
        Ok(())
    }

    async fn handle_connection_loop(socket: TcpStream, message_sender: mpsc::Sender<RaftMessage>) -> Result<(), String> {
        use tokio::io::BufReader;

        let reader = BufReader::new(socket);
        let mut lines = reader.lines();

        while let Some(line) = lines.next_line().await.map_err(|e| format!("Failed to read line: {}", e))? {
            let message: RaftMessage = serde_json::from_str(&line).map_err(|e| format!("Failed to parse message: {}", e))?;

            message_sender.send(message).await.map_err(|e| format!("Failed to send message to receiver: {}", e))?;
        }
        Ok(())
    }

    pub async fn broadcast(&mut self, message: RaftMessage) -> Vec<Result<(), String>> {
        let mut results = Vec::new();

        for node in self.connections.keys().cloned().collect::<Vec<_>>() {
            let result = self.send_message(node, message.clone()).await;
            results.push(result);
        }
        results
    }

    pub fn is_connected(&self, node_id: NodeId) -> bool {
        self.connections.contains_key(&node_id)
    }

    pub fn get_connected_nodes(&self) -> Vec<NodeId> {
        self.connections.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpStream;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use std::net::SocketAddr;
    use std::str::FromStr;

    #[tokio::test]
    async fn test_network_creation() {
        let addr = SocketAddr::from_str("127.0.0.1:0").unwrap();
        let retry_strategy = RetryStrategy::Static { delay: Duration::from_millis(100) };
        
        let network = Network::new(1, addr, retry_strategy).await;
        assert!(network.is_ok());
    }

    #[tokio::test]
    async fn test_add_and_remove_nodes() {
        let addr = SocketAddr::from_str("127.0.0.1:0").unwrap();
        let retry_strategy = RetryStrategy::Static { delay: Duration::from_millis(100) };
        let mut network = Network::new(1, addr, retry_strategy).await.unwrap();
        
        let node_addr = SocketAddr::from_str("127.0.0.1:8080").unwrap();
        network.add_node(2, node_addr);
        
        // Should be able to connect to added node
        assert!(network.node_addresses.contains_key(&2));
        
        network.remove_node(2);
        assert!(!network.connections.contains_key(&2));
    }

    #[tokio::test]
    async fn test_connect_to_unknown_node() {
        let addr = SocketAddr::from_str("127.0.0.1:0").unwrap();
        let retry_strategy = RetryStrategy::Static { delay: Duration::from_millis(100) };
        let mut network = Network::new(1, addr, retry_strategy).await.unwrap();
        
        let result = network.connect_to_node(999).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[tokio::test]
    async fn test_retry_strategies() {
        let static_delay = Duration::from_millis(50);
        let static_strategy = RetryStrategy::Static { delay: static_delay };
        assert_eq!(static_strategy.calculate_retry_delay(1), static_delay);
        assert_eq!(static_strategy.calculate_retry_delay(5), static_delay);
        
        let base_delay = Duration::from_millis(10);
        let max_delay = Duration::from_millis(100);
        let exp_strategy = RetryStrategy::Exponential { base: base_delay, max: max_delay };
        
        // First attempt: 10ms
        assert_eq!(exp_strategy.calculate_retry_delay(0), base_delay);
        // Second attempt: 20ms
        assert_eq!(exp_strategy.calculate_retry_delay(1), Duration::from_millis(20));
        // Third attempt: 40ms
        assert_eq!(exp_strategy.calculate_retry_delay(2), Duration::from_millis(40));
        // Should be capped at max_delay
        assert_eq!(exp_strategy.calculate_retry_delay(10), max_delay);
    }

    #[tokio::test]
    async fn test_connection_status() {
        let addr = SocketAddr::from_str("127.0.0.1:0").unwrap();
        let retry_strategy = RetryStrategy::Static { delay: Duration::from_millis(100) };
        let network = Network::new(1, addr, retry_strategy).await.unwrap();
        
        assert!(!network.is_connected(999));
        assert!(network.get_connected_nodes().is_empty());
    }

    #[tokio::test]
    async fn test_broadcast_with_no_connections() {
        let addr = SocketAddr::from_str("127.0.0.1:0").unwrap();
        let retry_strategy = RetryStrategy::Static { delay: Duration::from_millis(100) };
        let mut network = Network::new(1, addr, retry_strategy).await.unwrap();
        
        let message = RaftMessage::RequestVote(crate::messages::RequestVote {
            term: 1,
            candidate_id: 1,
            last_log_index: 0,
            last_log_term: 0,
        });
        
        let results = network.broadcast(message).await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_message_serialization() {
        let message = RaftMessage::RequestVote(crate::messages::RequestVote {
            term: 1,
            candidate_id: 2,
            last_log_index: 5,
            last_log_term: 1,
        });
        
        let json = serde_json::to_string(&message).unwrap();
        let deserialized: RaftMessage = serde_json::from_str(&json).unwrap();
        
        assert_eq!(format!("{:?}", message), format!("{:?}", deserialized));
    }

    // Integration test: Test actual TCP communication
    #[tokio::test]
    async fn test_tcp_message_exchange() {
        // Start a simple TCP server
        let server_addr = SocketAddr::from_str("127.0.0.1:0").unwrap();
        let listener = TcpListener::bind(server_addr).await.unwrap();
        let server_addr = listener.local_addr().unwrap();
        
        // Spawn server task
        let server_task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 1024];
            let n = socket.read(&mut buffer).await.unwrap();
            let received = String::from_utf8_lossy(&buffer[..n]);
            
            // Echo back
            socket.write_all(received.as_bytes()).await.unwrap();
        });
        
        // Connect client
        let mut client = TcpStream::connect(server_addr).await.unwrap();
        let test_message = "Hello, Raft!";
        client.write_all(test_message.as_bytes()).await.unwrap();
        
        // Read response
        let mut buffer = [0; 1024];
        let n = client.read(&mut buffer).await.unwrap();
        let response = String::from_utf8_lossy(&buffer[..n]);
        
        assert_eq!(response, test_message);
        
        server_task.await.unwrap();
    }
}