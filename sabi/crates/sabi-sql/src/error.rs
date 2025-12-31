use thiserror::Error;
use sabi_core::error::DbError;

#[derive(Error, Debug)]
pub enum SqlError {
    #[error("Parse error: {0}")]
    ParseError(String),
    
    #[error("Planner error: {0}")]
    PlannerError(String),
    
    #[error("Execution error: {0}")]
    ExecutionError(String),
    
    #[error("Type error: expected {expected}, got {actual}")]
    TypeError { expected: String, actual: String },
    
    #[error("Column not found: {0}")]
    ColumnNotFound(String),
    
    #[error("Table not found: {0}")]
    TableNotFound(String),
    
    #[error("Constraint violation: {0}")]
    ConstraintViolation(String),
    
    #[error("Internal error: {0}")]
    Internal(String),
    
    #[error("Storage error: {0}")]
    Storage(#[from] DbError),
}

impl From<sqlparser::parser::ParserError> for SqlError {
    fn from(err: sqlparser::parser::ParserError) -> Self {
        SqlError::ParseError(err.to_string())
    }
}