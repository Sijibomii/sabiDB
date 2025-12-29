use uuid::Uuid;
use serde::{Serialize, Deserialize};

/// Unique transaction identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TxId(Uuid);

impl TxId {
    pub fn new() -> Self {
        TxId(Uuid::new_v4())
    }

    pub fn nil() -> Self {
        TxId(Uuid::nil())
    }
}

impl std::fmt::Display for TxId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::str::FromStr for TxId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uuid = Uuid::parse_str(s)?;
        Ok(TxId(uuid))
    }
}