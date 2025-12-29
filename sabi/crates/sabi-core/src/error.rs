use std::fmt;

#[derive(Debug)]
pub enum DbError {
    SyntaxError(String),
    SerializationFailure(String),
    Internal(String),
}

impl std::error::Error for DbError {}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::SyntaxError(msg) => write!(f, "Syntax Error: {}", msg),
            DbError::SerializationFailure(msg) => write!(f, "Serialization Failure: {}", msg),
            DbError::Internal(msg) => write!(f, "Internal Error: {}", msg),
        }
    }
}
