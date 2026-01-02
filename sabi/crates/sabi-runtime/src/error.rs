use thiserror::Error;
use sabi_core::error::DbError;
use serde_json::Error as JsonError;
use std::sync::PoisonError;

#[derive(Error, Debug)]
pub enum RuntimeError {
    #[error("Determinism violation: {0}")]
    DeterminismViolation(String),
    
    #[error("Function execution error: {0}")]
    FunctionError(String),
    
    #[error("Serialization error: {0}")]
    SerializationError(#[from] JsonError),
    
    #[error("Storage error: {0}")]
    StorageError(#[from] DbError),
    
    #[error("Invalid function: {0}")]
    InvalidFunction(String),
    
    #[error("Circular dependency detected")]
    CircularDependency,
    
    #[error("Query not found: {0}")]
    QueryNotFound(String),
    
    #[error("Mutation not found: {0}")]
    MutationNotFound(String),
    
    #[error("Action not found: {0}")]
    ActionNotFound(String),
    
    #[error("Subscription error: {0}")]
    SubscriptionError(String),
    
    #[error("Lock poisoned: {0}")]
    LockPoisoned(String),
}

impl<T> From<PoisonError<T>> for RuntimeError {
    fn from(err: PoisonError<T>) -> Self {
        RuntimeError::LockPoisoned(err.to_string())
    }
}