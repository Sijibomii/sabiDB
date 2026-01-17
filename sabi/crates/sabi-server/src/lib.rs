//! SabiDB HTTP/WebSocket server
//!
//! Implements the sabiDB Wire Protocol v1
//! - HTTP for queries/mutations
//! - WebSocket for subscriptions
//! - Deterministic function execution
//! - Reactive updates

pub mod http;
pub mod websocket;
pub mod protocol;
pub mod server;
pub mod client_tracker;

#[cfg(test)]
mod tests;

pub use server::SabiServer;
pub use protocol::*;