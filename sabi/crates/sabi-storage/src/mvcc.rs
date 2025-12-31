//! MVCC Transaction & Storage Layer (Multi-Version Concurrency Control (MVCC))
//!
//! Guarantees:
//! - Snapshot isolation
//! - Non-blocking readers
//! - Deterministic replay from WAL
//! - WAL-first durability

use uuid::{Uuid};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, RwLock};
use std::sync::atomic::{AtomicU64, Ordering};

use sabi_core::error::{DbError, Result};
use sabi_core::TxId;


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxState {
    Active,     // Transaction is in progress
    Committed,  // Transaction successfully completed
    Aborted,    // Transaction rolled back
}

#[derive(Debug, Clone)]
pub struct Transaction {
    pub tx_id: TxId,           // Unique ID for this transaction
    pub snapshot_tx: TxId,     // Most recent committed transaction when this started
    pub state: TxState,        // Current state
    pub read_only: bool,       // Whether transaction only reads
    pub tx_number: u64,     // Sequential transaction number
}


#[derive(Debug, Clone)]
pub struct RowVersion {
    pub created_by: TxId,      // Which transaction created this version
    pub deleted_by: Option<TxId>, // Which transaction deleted this version (tombstone)
    pub value: Vec<u8>,        // Actual row data
}

// Instead of updating rows in-place, create new versions. Old versions remain for active readers.

/// Key → versions (newest first)
pub type VersionChain = Vec<RowVersion>;

/* =========================
    Key: "user:1"
Versions: [
    RowVersion { created_by: tx3, deleted_by: None, value: "Alice v3" },   ← Latest
    RowVersion { created_by: tx2, deleted_by: Some(tx3), value: "Alice v2" },
    RowVersion { created_by: tx1, deleted_by: Some(tx2), value: "Alice v1" },
]
 * ========================= */
#[derive(Debug)]
pub struct MvccTable {
    pub rows: BTreeMap<Vec<u8>, VersionChain>, 
}

impl MvccTable {
    pub fn new() -> Self {
        Self {
            rows: BTreeMap::new(),
        }
    }

    pub fn read(&self, key: &[u8], tx: &Transaction) -> Option<Vec<u8>> {
        let versions = self.rows.get(key)?;

        versions.iter().find_map(|v| {
            if is_visible(v, tx) {
                Some(v.value.clone())
            } else {
                None
            }
        })
    }

    pub fn insert(&mut self, key: Vec<u8>, value: Vec<u8>, tx: &Transaction) {
        let versions = self.rows.entry(key).or_insert_with(Vec::new);

        versions.insert(
            0,
            RowVersion {
                created_by: tx.tx_id,
                deleted_by: None,
                value,
            },
        );
    }
    // Soft Delete: Marks version as deleted but doesn't remove it.
    pub fn delete(&mut self, key: &[u8], tx: &Transaction) -> Result<()> {
        let versions = self
            .rows
            .get_mut(key)
            .ok_or(DbError::NotFound("Row not found".into()))?;

        // Mark latest visible version as deleted
        for v in versions.iter_mut() {
            if is_visible(v, tx) {
                v.deleted_by = Some(tx.tx_id); // Mark as deleted (tombstone)
                return Ok(());
            }
        }

        Err(DbError::NotFound("Row not visible".into()))
    }
}


pub struct TransactionManager {
    next_tx: AtomicU64,          // Counter for sequential transaction IDs
    active: Mutex<HashMap<TxId, Transaction>>,  // Active transactions
    latest_committed: AtomicU64,
    tx_registry: RwLock<Vec<Option<TxId>>>,
}

impl TransactionManager {
    pub fn new() -> Self {
        Self {
            next_tx: AtomicU64::new(1),
            active: Mutex::new(HashMap::new()),

            // I could have avoided storing tx_registry and latest_committed but the txId is a uuid and its hard to get the last snapshot id
            tx_registry: RwLock::new(vec![Some(TxId::nil())]),  // Index 0 = nil transaction
            latest_committed: AtomicU64::new(0),
        }
    }

    /// BEGIN TRANSACTION : Each transaction sees database as it was when previous transaction committed
    pub fn begin(&self, read_only: bool) -> Transaction {
        // increment next_tx atomically but returns old value
        let tx_number = self.next_tx.fetch_add(1, Ordering::SeqCst);
        
        // Create timestamped ID
        let tx_id = TxId(Uuid::now_v7());
        
        // Store in registry - must write lock first
        {
            let mut registry = self.tx_registry.write().unwrap();
            // Ensure registry is big enough
            if registry.len() <= tx_number as usize {
                registry.resize(tx_number as usize + 1, None);
            }
            registry[tx_number as usize] = Some(tx_id);
        }
        
        // Get snapshot (previous transaction)
        let snapshot = {
            let registry = self.tx_registry.read().unwrap();
            if tx_number > 1 {  // Note: > 1 because tx_number starts at 1
                // Get the actual previous transaction ID
                registry[(tx_number - 1) as usize].unwrap_or(TxId::nil())
            } else {
                // tx_number = 1 gets snapshot of nil/initial transaction
                TxId::nil()
            }
        };
        
        // Create transaction
        let tx = Transaction {
            tx_id,
            snapshot_tx: snapshot,
            state: TxState::Active,
            read_only,
            tx_number,
        };
        
        // Store in active transactions
        self.active.lock().unwrap().insert(tx_id, tx.clone());
        
        tx
    }

    /// COMMIT TRANSACTION
    pub fn commit(&self, tx_id: TxId) -> Result<()> {
        // Acquires exclusive lock on active transactions. Prevents concurrent commit/abort operations
        let mut active = self.active.lock().unwrap();
        
        // Find and update the transaction
        // Ensures transaction exists and is active. Prevents committing already committed/aborted transactions
        let tx = active.get_mut(&tx_id)
            .ok_or_else(|| DbError::InvalidTransaction(format!(
                "Cannot commit transaction {}: not found or not active", tx_id
            )))?;
        
        // Validate transaction state
        if tx.state != TxState::Active {
            return Err(DbError::InvalidTransaction(format!(
                "Transaction {} is not active (state: {:?})", tx_id, tx.state
            )));
        }
        
        // Update state
        tx.state = TxState::Committed;
        
        // Update latest committed transaction number
        self.latest_committed.store(tx.tx_number, Ordering::SeqCst);
        
        // Remove from active
        active.remove(&tx_id);
        
        Ok(())
    }

    /// ABORT TRANSACTION
    pub fn abort(&self, tx_id: TxId) -> Result<()> {
        let mut active = self.active.lock().unwrap();
        
        let tx = active.get_mut(&tx_id)
            .ok_or_else(|| DbError::InvalidTransaction(format!(
                "Cannot abort transaction {}: not found or not active", tx_id
            )))?;
        
        if tx.state != TxState::Active {
            return Err(DbError::InvalidTransaction(format!(
                "Transaction {} is not active (state: {:?})", tx_id, tx.state
            )));
        }
        
        tx.state = TxState::Aborted;
        active.remove(&tx_id);
        
        Ok(())
    }
    
}


#[inline]
pub fn is_visible(v: &RowVersion, tx: &Transaction) -> bool {
    v.created_by <= tx.snapshot_tx
        && v.deleted_by.map_or(true, |d| d > tx.snapshot_tx)
}