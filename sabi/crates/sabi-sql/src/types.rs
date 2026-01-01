use std::fmt;
use serde::{Serialize, Deserialize};

use crate::error::SqlError;

/// Supported SQL data types
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataType {
    Integer,
    Text,
    Boolean,
    // TODO: Add more types: Float, DateTime, Binary, etc.
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataType::Integer => write!(f, "INTEGER"),
            DataType::Text => write!(f, "TEXT"),
            DataType::Boolean => write!(f, "BOOLEAN"),
        }
    }
}

/// Runtime values
#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum Value {
    Null,
    Integer(i64),
    Text(String),
    Boolean(bool),
    // TODO: Add Float(f64), Binary(Vec<u8>), etc.
}

impl Value {

    pub fn compare(&self, other: &Value) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (Value::Null, Value::Null) => Some(std::cmp::Ordering::Equal),
            (Value::Null, _) | (_, Value::Null) => None, // NULL is not comparable
            (Value::Integer(a), Value::Integer(b)) => Some(a.cmp(b)),
            (Value::Text(a), Value::Text(b)) => Some(a.cmp(b)),
            (Value::Boolean(a), Value::Boolean(b)) => Some(a.cmp(b)),
            _ => None, 
        }
    }
    
    pub fn lt(&self, other: &Value) -> bool {
        self.compare(other).map(|ord| ord == std::cmp::Ordering::Less).unwrap_or(false)
    }
    
    pub fn le(&self, other: &Value) -> bool {
        self.compare(other).map(|ord| ord != std::cmp::Ordering::Greater).unwrap_or(false)
    }
    
    pub fn gt(&self, other: &Value) -> bool {
        self.compare(other).map(|ord| ord == std::cmp::Ordering::Greater).unwrap_or(false)
    }
    
    pub fn ge(&self, other: &Value) -> bool {
        self.compare(other).map(|ord| ord != std::cmp::Ordering::Less).unwrap_or(false)
    }

    pub fn data_type(&self) -> Option<DataType> {
        match self {
            Value::Null => None,
            Value::Integer(_) => Some(DataType::Integer),
            Value::Text(_) => Some(DataType::Text),
            Value::Boolean(_) => Some(DataType::Boolean),
        }
    }
    
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Value::Null => vec![0],
            Value::Integer(i) => {
                let mut bytes = vec![1]; // Tag for integer
                bytes.extend_from_slice(&i.to_le_bytes());
                bytes
            }
            Value::Text(s) => {
                let mut bytes = vec![2]; // Tag for text
                bytes.extend_from_slice(&(s.len() as u32).to_le_bytes());
                bytes.extend_from_slice(s.as_bytes());
                bytes
            }
            Value::Boolean(b) => {
                vec![3, *b as u8] // Tag for boolean + value
            }
        }
    }
    
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SqlError> {
        if bytes.is_empty() {
            return Ok(Value::Null);
        }
        
        match bytes[0] {
            0 => Ok(Value::Null),
            1 => {
                if bytes.len() < 9 {
                    return Err(SqlError::Internal("Invalid integer encoding".into()));
                }
                let int_bytes: [u8; 8] = bytes[1..9].try_into()
                    .map_err(|_| SqlError::Internal("Invalid integer bytes".into()))?;
                Ok(Value::Integer(i64::from_le_bytes(int_bytes)))
            }
            2 => {
                if bytes.len() < 5 {
                    return Err(SqlError::Internal("Invalid text encoding".into()));
                }
                let len_bytes: [u8; 4] = bytes[1..5].try_into()
                    .map_err(|_| SqlError::Internal("Invalid length bytes".into()))?;
                let len = u32::from_le_bytes(len_bytes) as usize;
                
                if bytes.len() < 5 + len {
                    return Err(SqlError::Internal("Text truncated".into()));
                }
                let text = String::from_utf8(bytes[5..5 + len].to_vec())
                    .map_err(|e| SqlError::Internal(format!("Invalid UTF-8: {}", e)))?;
                Ok(Value::Text(text))
            }
            3 => {
                if bytes.len() < 2 {
                    return Err(SqlError::Internal("Invalid boolean encoding".into()));
                }
                Ok(Value::Boolean(bytes[1] != 0))
            }
            _ => Err(SqlError::Internal("Unknown value type tag".into())),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => write!(f, "NULL"),
            Value::Integer(i) => write!(f, "{}", i),
            Value::Text(s) => write!(f, "'{}'", s.replace("'", "''")),
            Value::Boolean(b) => write!(f, "{}", b),
        }
    }
}

/// Column definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnDef {
    pub name: String,
    pub data_type: DataType,
    pub nullable: bool,
    pub primary_key: bool,
}

/// Table schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<ColumnDef>,
    pub primary_key: Option<Vec<String>>,
}