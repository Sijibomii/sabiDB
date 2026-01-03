//! WebSocket handler for subscriptions and real-time updates

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc};
use tokio::time::{interval};
use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use uuid::Uuid;
use serde_json::json;
use bytes::Bytes;

use crate::{
    SabiServer,
    protocol::*
};

/// WebSocket connection handler
pub struct WebSocketHandler {
    server: Arc<SabiServer>,
}

impl WebSocketHandler {
    pub fn new(server: Arc<SabiServer>) -> Self {
        Self { server }
    }
    
    /// Handle new WebSocket connection
    pub async fn handle_connection(self, socket: WebSocket) {
        let client_id = Uuid::now_v7();
        print!("New WebSocket connection: {}", client_id);
        
        // Update metrics
        {
            let mut metrics = self.server.metrics.lock().await;
            metrics.connections += 1;
        }
        
        // Register client
        {
            let mut tracker = self.server.client_tracker.write().await;
            tracker.add_client(client_id);
        }
        
        // Setup channels for communication
        let (tx, mut rx) = mpsc::channel::<Message>(100);
        
        // Spawn task to send messages to client
        let (mut write_socket, mut read_socket) = socket.split();
        let write_task = tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                if write_socket.send(message).await.is_err() {
                    break;
                }
            }
        });
        
        // Handle incoming messages
        let mut last_heartbeat = Instant::now();
        let mut heartbeat_interval = interval(Duration::from_secs(30));
        
        loop {
            tokio::select! {
                // Receive message from client
                result = read_socket.next() => {
                    match result {
                        Some(Ok(Message::Text(text))) => {
                            if let Err(e) = self.handle_message(client_id, &text, &tx).await {
                                print!("Error handling message from {}: {}", client_id, e);
                                break;
                            }
                        }
                        Some(Ok(Message::Ping(ping))) => {
                            // Send pong response
                            let _ = tx.send(Message::Pong(ping)).await;
                            last_heartbeat = Instant::now();
                        }
                        Some(Ok(Message::Close(_))) => {
                            print!("Client {} closed connection", client_id);
                            break;
                        }
                        Some(Err(e)) => {
                            print!("WebSocket error from {}: {}", client_id, e);
                            break;
                        }
                        None => {
                            print!("Client {} disconnected", client_id);
                            break;
                        }
                        _ => {
                            // Ignore other message types
                        }
                    }
                }
                
                // Send heartbeat
                _ = heartbeat_interval.tick() => {
                    if last_heartbeat.elapsed() > Duration::from_secs(60) {
                        print!("Client {} heartbeat timeout", client_id);
                        break;
                    }
                    
                    // Send ping
                    let _ = tx.send(Message::Ping(Bytes::new())).await;
                }
                
                // Check for shutdown
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    // Keep alive
                }
            }
        }
        
        // Cleanup
        print!("Cleaning up WebSocket connection for {}", client_id);
        
        // Remove client
        {
            let mut tracker = self.server.client_tracker.write().await;
            tracker.remove_client(client_id);
        }
        
        // Update metrics
        {
            let mut metrics = self.server.metrics.lock().await;
            metrics.connections = metrics.connections.saturating_sub(1);
        }
        
        // Cancel write task
        write_task.abort();
        
    }
    
    /// Handle incoming WebSocket message
    async fn handle_message(
        &self,
        client_id: Uuid,
        text: &str,
        tx: &mpsc::Sender<Message>,
    ) -> Result<(), String> {
        print!("Received message from {}: {}", client_id, text);
        
        // Parse envelope
        let envelope: MessageEnvelope = serde_json::from_str(text)
            .map_err(|e| format!("Invalid message envelope: {}", e))?;
        
        // Validate protocol version
        if envelope.version != PROTOCOL_VERSION {
            let error = ErrorResponse {
                code: ErrorCode::Internal,
                message: format!("Unsupported protocol version: {}", envelope.version),
                details: Some(json!({"supported_version": PROTOCOL_VERSION})),
            };
            
            self.send_error(tx, envelope.request_id, error).await;
            return Ok(());
        }
        
        // Handle message type
        match envelope.message_type {
            MessageType::Hello => {
                self.handle_hello(client_id, envelope.payload, tx).await?;
            }
            MessageType::Subscribe => {
                self.handle_subscribe(client_id, envelope.request_id, envelope.payload, tx).await?;
            }
            MessageType::Unsubscribe => {
                self.handle_unsubscribe(client_id, envelope.payload, tx).await?;
            }
            _ => {
                let error = ErrorResponse {
                    code: ErrorCode::SyntaxError,
                    message: format!("Unsupported message type: {:?}", envelope.message_type),
                    details: None,
                };
                self.send_error(tx, envelope.request_id, error).await;
            }
        }
        
        Ok(())
    }
    
    /// Handle HELLO message (handshake)
    async fn handle_hello(
        &self,
        client_id: Uuid,
        payload: serde_json::Value,
        tx: &mpsc::Sender<Message>,
    ) -> Result<(), String> {
        let hello: HelloMessage = serde_json::from_value(payload)
            .map_err(|e| format!("Invalid HELLO message: {}", e))?;
        
        // Update client state
        {
            let mut tracker = self.server.client_tracker.write().await;
            tracker.update_last_seen_tx(client_id, hello.last_seen_tx);
        }
        
        // Get current transaction ID
        let current_tx = *self.server.current_tx_id.read().await;
        
        // Send WELCOME response
        let welcome = WelcomeMessage {
            current_tx,
        };
        
        let envelope = MessageEnvelope {
            version: PROTOCOL_VERSION,
            message_type: MessageType::Welcome,
            request_id: None,
            payload: serde_json::to_value(welcome)
                .map_err(|e| format!("Failed to serialize WELCOME: {}", e))?,
        };
        
        let message = Message::Text(
            serde_json::to_string(&envelope) 
                // .into() converts a value from one type to another, provided that: The source type implements Into<TargetType> trait Or the target type implements From<SourceType> trait (these two are reciprocal)
                /*
                // When you write:
                let string: String = "hello".to_string();
                let utf8_bytes: Utf8Bytes = string.into();

                // This works because Utf8Bytes likely implements:
                impl From<String> for Utf8Bytes {
                    fn from(s: String) -> Self {
                        Utf8Bytes::new(s.into_bytes())
                    }
                }

                // Or String implements:
                impl Into<Utf8Bytes> for String {
                    fn into(self) -> Utf8Bytes {
                        Utf8Bytes::new(self.into_bytes())
                    }
}
                 */
                .map_err(|e| format!("Failed to serialize envelope: {}", e))?.into() 
        );
        
        tx.send(message).await
            .map_err(|e| format!("Failed to send WELCOME: {}", e))?;
        
        print!("Sent WELCOME to client {}, current_tx={}", client_id, current_tx);
        
        // Replay missed updates if needed
        if let Some(last_seen_tx) = hello.last_seen_tx {
            if last_seen_tx < current_tx {
                self.replay_missed_updates(client_id, last_seen_tx, current_tx, tx).await?;
            }
        }
        
        Ok(())
    }
    
    /// Handle SUBSCRIBE message
    async fn handle_subscribe(
        &self,
        client_id: Uuid,
        request_id: Option<String>,
        payload: serde_json::Value,
        tx: &mpsc::Sender<Message>,
    ) -> Result<(), String> {
        let subscribe: SubscribeRequest = serde_json::from_value(payload)
            .map_err(|e| format!("Invalid SUBSCRIBE message: {}", e))?;
        
        // Create subscription
        let subscription_id = self.server.create_subscription(
            client_id,
            &subscribe.query,
            subscribe.args,
        ).await
        .map_err(|e| e.to_string())?;
        
        // Send ACK
        let ack_payload = SubscribeResponse {
            subscription_id: subscription_id.clone(),
        };
        
        let envelope = MessageEnvelope {
            version: PROTOCOL_VERSION,
            message_type: MessageType::Ack,
            request_id,
            payload: serde_json::to_value(ack_payload)
                .map_err(|e| format!("Failed to serialize ACK: {}", e))?,
        };
        
        let message = Message::Text(
            serde_json::to_string(&envelope)
                .map_err(|e| format!("Failed to serialize envelope: {}", e))?.into()
        );
        
        tx.send(message).await
            .map_err(|e| format!("Failed to send ACK: {}", e))?;
        
        print!("Created subscription {} for client {}", subscription_id, client_id);
        
        Ok(())
    }
    
    /// Handle UNSUBSCRIBE message
    async fn handle_unsubscribe(
        &self,
        client_id: Uuid,
        payload: serde_json::Value,
        _: &mpsc::Sender<Message>,
    ) -> Result<(), String> {
        let unsubscribe: UnsubscribeRequest = serde_json::from_value(payload)
            .map_err(|e| format!("Invalid UNSUBSCRIBE message: {}", e))?;
        
        // Remove subscription
        self.server.remove_subscription(client_id, &unsubscribe.subscription_id).await
            .map_err(|e| e.to_string())?;
        
        print!("Removed subscription {} for client {}", unsubscribe.subscription_id, client_id);
        
        Ok(())
    }
    
    /// Replay missed updates to client
    async fn replay_missed_updates(
        &self,
        client_id: Uuid,
        from_tx: u64,
        to_tx: u64,
        tx: &mpsc::Sender<Message>,
    ) -> Result<(), String> {
        print!("Replaying updates for client {} from tx {} to {}", client_id, from_tx, to_tx);
        
        // In a real implementation, you would:
        // 1. Query WAL for changes between from_tx and to_tx
        // 2. Filter changes relevant to this client's subscriptions
        // 3. Send UPDATE messages for each relevant change
        
        // For now, just send a placeholder update
        let update = UpdateMessage {
            subscription_id: "replay".to_string(),
            tx_id: to_tx,
            rows: vec![
                json!({
                    "type": "replay",
                    "from": from_tx,
                    "to": to_tx,
                    "message": "Catch-up complete"
                })
            ],
        };
        
        let envelope = MessageEnvelope {
            version: PROTOCOL_VERSION,
            message_type: MessageType::Update,
            request_id: None,
            payload: serde_json::to_value(update)
                .map_err(|e| format!("Failed to serialize UPDATE: {}", e))?,
        };
        
        let message = Message::Text(
            serde_json::to_string(&envelope)
                .map_err(|e| format!("Failed to serialize envelope: {}", e))?.into()
        );
        
        tx.send(message).await
            .map_err(|e| format!("Failed to send replay UPDATE: {}", e))?;
        
        Ok(())
    }
    
    /// Send error message to client
    async fn send_error(
        &self,
        tx: &mpsc::Sender<Message>,
        request_id: Option<String>,
        error: ErrorResponse,
    ) {
        let envelope = MessageEnvelope {
            version: PROTOCOL_VERSION,
            message_type: MessageType::Error,
            request_id,
            payload: serde_json::to_value(error).unwrap_or_default(),
        };
        
        if let Ok(text) = serde_json::to_string(&envelope) {
            let _ = tx.send(Message::Text(text.into())).await;
        }
    }
}