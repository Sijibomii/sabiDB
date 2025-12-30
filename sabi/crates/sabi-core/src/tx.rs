use uuid::{Uuid, Timestamp};
use serde::{Serialize, Deserialize};
use std::cmp::{Ord, Ordering, PartialOrd};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TxId(pub Uuid);

impl PartialOrd for TxId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TxId {
    // this is the reason we can do total ordering on TxId
    fn cmp(&self, other: &Self) -> Ordering {
        // Extract Unix timestamps and compare them
        let ts1 = self.extract_timestamp();
        let ts2 = other.extract_timestamp();
        
        // Compare seconds first, then nanoseconds
        match ts1.0.cmp(&ts2.0) {
            Ordering::Equal => ts1.1.cmp(&ts2.1), // Compare nanoseconds if seconds equal
            ordering => ordering,
        }
    }
}

impl TxId {
    /// Extract timestamp as (seconds, nanoseconds) tuple
    fn extract_timestamp(&self) -> (u64, u32) {
        self.0.get_timestamp()
            .map(|ts| ts.to_unix())
            .unwrap_or((0, 0))
    }
    

    pub fn new() -> Self {
        TxId(Uuid::now_v7())
    }
    
    pub fn nil() -> Self {
        TxId(Uuid::nil())
    }
    
    /// Get the timestamp of this transaction ID as SystemTime
    pub fn timestamp(&self) -> Option<SystemTime> {
        self.0.get_timestamp().and_then(|ts| {
            let (secs, nanos) = ts.to_unix();
            UNIX_EPOCH.checked_add(std::time::Duration::new(secs, nanos))
        })
    }
    
    /// Create from a specific timestamp (useful for testing/replay)
    pub fn from_timestamp(secs: u64, nanos: u32) -> Self {
        let context = uuid::NoContext;
        let ts = Timestamp::from_unix(context, secs, nanos);
        TxId(Uuid::new_v7(ts))
    }
    
    /// Check if this is a nil transaction ID
    pub fn is_nil(&self) -> bool {
        self.0.is_nil()
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