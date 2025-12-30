use std::fmt;

#[derive(Debug)]
pub enum DbError {
    SyntaxError(String),
    SerializationFailure(String),
    Internal(String),
    Corruption(String),
    NotFound(String),
    InvalidTransaction(String),
    Io(std::io::Error),
}

pub type Result<T> = std::result::Result<T, DbError>;

impl std::error::Error for DbError {}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::SyntaxError(msg) => write!(f, "Syntax Error: {}", msg),
            DbError::SerializationFailure(msg) => write!(f, "Serialization Failure: {}", msg),
            DbError::Internal(msg) => write!(f, "Internal Error: {}", msg),
            DbError::Corruption(msg) => write!(f, "Corruption: {}", msg),
            DbError::NotFound(msg) => write!(f, "Not Found: {}", msg),
            DbError::InvalidTransaction(msg) => write!(f, "Invalid Transaction: {}", msg),
            DbError::Io(err) => write!(f, "IO Error: {}", err),
        }
    }
}

impl From<std::io::Error> for DbError {
    fn from(err: std::io::Error) -> Self {
        DbError::Io(err)
    }
}