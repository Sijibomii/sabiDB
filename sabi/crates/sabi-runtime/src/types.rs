use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use uuid::Uuid;

/// JavaScript-like values that can be stored/returned
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JsValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsValue>),
    Object(HashMap<String, JsValue>),
}

impl JsValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            JsValue::Null => "null",
            JsValue::Bool(_) => "boolean",
            JsValue::Number(_) => "number",
            JsValue::String(_) => "string",
            JsValue::Array(_) => "array",
            JsValue::Object(_) => "object",
        }
    }
}

/// Result of function execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionResult {
    pub value: JsValue,
    pub keys_read: Vec<String>,
    pub keys_written: Vec<String>,
    pub duration_ns: u64,
}

/// Query function definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryDef {
    pub name: String,
    pub function: String,  // JavaScript source code
    pub args: Vec<JsValue>,
}

/// Mutation function definition  
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationDef {
    pub name: String,
    pub function: String,
    pub args: Vec<JsValue>,
}

/// Action function definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionDef {
    pub name: String,
    pub function: String,
    pub args: Vec<JsValue>,
}

/// Transaction context
#[derive(Debug, Clone)]
pub struct TransactionContext {
    pub tx_id: Uuid,
    pub start_time: u64,  // Logical timestamp
    pub is_read_only: bool,
}

/// Execution environment with deterministic primitives
#[derive(Debug, Clone)]
pub struct DeterministicEnv {
    pub random_seed: u64,     // Derived from tx_id + WAL position
    pub logical_time: u64,    // Monotonic counter
    pub context: TransactionContext,
}

impl DeterministicEnv {
    /// Generate deterministic random number
    pub fn random(&mut self) -> f64 {
        // Xorshift algorithm - deterministic
        let mut x = self.random_seed;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.random_seed = x;
        (x as f64) / (u64::MAX as f64)
    }
    
    /// Get deterministic "current time" (logical time)
    pub fn now(&self) -> u64 {
        self.logical_time
    }
}