use serde::{Serialize, Deserialize};
use crate::tx::TxId;

/// Client → Server messages
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    Query {
        request_id: TxId,
        sql: String,
    },
    Subscribe {
        request_id: TxId,
        sql: String,
    },
    Notify {
        request_id: TxId,
        sql: String,
    },
}

/// Server → Client messages
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    Ok {
        request_id: TxId,
        payload: serde_json::Value,
    },
    Error {
        request_id: TxId,
        payload: ErrorPayload,
    },
    Update {
        subscription_id: TxId,
        payload: serde_json::Value,
    },
}

/// error payload
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    SyntaxError,
    SerializationFailure,
    Internal,
}
