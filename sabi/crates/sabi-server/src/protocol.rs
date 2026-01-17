//! Wire protocol definitions
//! 
//! Implements sabiDB Wire Protocol v1

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::fmt;

/// Protocol version
pub const PROTOCOL_VERSION: u32 = 1;

/// Message envelope
#[derive(Debug, Serialize, Deserialize)]
pub struct MessageEnvelope {
    #[serde(rename = "v")]
    pub version: u32,
    #[serde(rename = "type")]
    pub message_type: MessageType,
    #[serde(rename = "request_id")]
    pub request_id: Option<String>,
    pub payload: serde_json::Value,
}

/// Message types
#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MessageType {
    Query,
    Mutation,
    Subscribe,
    Unsubscribe,
    Update,
    Error,
    Ack,
    Hello,
    Welcome,
}

/// Transaction identity
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TransactionIdentity {
    #[serde(rename = "tx_id")]
    pub tx_id: u64,
    pub timestamp: u64,
}

/// Query request
#[derive(Debug, Serialize, Deserialize)]
pub struct QueryRequest {
    pub sql: String,
    pub args: Vec<serde_json::Value>,
    #[serde(rename = "at_tx")]
    pub at_tx: Option<u64>,
}

/// Query response
#[derive(Debug, Serialize, Deserialize)]
pub struct QueryResponse {
    pub rows: Vec<serde_json::Value>,
    #[serde(rename = "read_tx")]
    pub read_tx: u64,
}

/// Mutation request
#[derive(Debug, Serialize, Deserialize)]
pub struct MutationRequest {
    pub sql: String,
    pub args: Vec<serde_json::Value>,
    #[serde(rename = "client_tx")]
    pub client_tx: u64,
}

/// Mutation response
#[derive(Debug, Serialize, Deserialize)]
pub struct MutationResponse {
    #[serde(rename = "tx_id")]
    pub tx_id: u64,
    #[serde(rename = "affected_rows")]
    pub affected_rows: u64,
}

/// Function execution request
#[derive(Debug, Serialize, Deserialize)]
pub struct FunctionRequest {
    pub name: String,
    pub args: serde_json::Value,
}

/// Function execution response
#[derive(Debug, Serialize, Deserialize)]
pub struct FunctionResponse {
    #[serde(rename = "tx_id")]
    pub tx_id: u64,
    pub result: serde_json::Value,
}

/// Subscribe request
#[derive(Debug, Serialize, Deserialize)]
pub struct SubscribeRequest {
    pub query: String,
    pub args: Vec<serde_json::Value>,
}

/// Subscribe response
#[derive(Debug, Serialize, Deserialize)]
pub struct SubscribeResponse {
    #[serde(rename = "subscription_id")]
    pub subscription_id: String,
}

/// Update message
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateMessage {
    #[serde(rename = "subscription_id")]
    pub subscription_id: String,
    #[serde(rename = "tx_id")]
    pub tx_id: u64,
    pub rows: Vec<serde_json::Value>,
}

/// Unsubscribe request
#[derive(Debug, Serialize, Deserialize)]
pub struct UnsubscribeRequest {
    #[serde(rename = "subscription_id")]
    pub subscription_id: String,
}

/// Hello message (WebSocket handshake)
#[derive(Debug, Serialize, Deserialize)]
pub struct HelloMessage {
    #[serde(rename = "last_seen_tx")]
    pub last_seen_tx: Option<u64>,
}

/// Welcome message (WebSocket handshake response)
#[derive(Debug, Serialize, Deserialize)]
pub struct WelcomeMessage {
    #[serde(rename = "current_tx")]
    pub current_tx: u64,
}

/// Error codes
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    SerializationFailure,
    SyntaxError,
    Internal,
    NotFound,
    Unauthorized,
    RateLimited,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorCode::SerializationFailure => write!(f, "SERIALIZATION_FAILURE"),
            ErrorCode::SyntaxError => write!(f, "SYNTAX_ERROR"),
            ErrorCode::Internal => write!(f, "INTERNAL"),
            ErrorCode::NotFound => write!(f, "NOT_FOUND"),
            ErrorCode::Unauthorized => write!(f, "UNAUTHORIZED"),
            ErrorCode::RateLimited => write!(f, "RATE_LIMITED"),
        }
    }
}

/// Error response
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl fmt::Display for ErrorResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.details {
            Some(details) => write!(f, "{}: {} - Details: {}", self.code, self.message, details),
            None => write!(f, "{}: {}", self.code, self.message),
        }
    }
}

/// Client state
#[derive(Debug, Clone)]
pub struct ClientState {
    pub client_id: Uuid,
    pub last_seen_tx: u64,
    pub connected_at: std::time::Instant,
    pub subscriptions: Vec<String>,
}

/// Server metrics
#[derive(Debug, Clone, serde::Serialize)]
pub struct ServerMetrics {
    pub connections: usize,
    pub subscriptions: usize,
    pub queries_per_second: f64,
    pub mutations_per_second: f64,
    pub current_tx_id: u64,
    pub uptime_seconds: u64,
}