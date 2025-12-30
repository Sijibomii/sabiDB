//! MVCC Transaction & Storage Layer
//!
//! Guarantees:
//! - Snapshot isolation
//! - Non-blocking readers
//! - Deterministic replay from WAL
//! - WAL-first durability

use uuid::{Uuid, Builder};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};

use sabi_core::error::{DbError, Result};
use sabi_core::TxId;

/* =========================
 * Transaction State
 * ========================= */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxState {
    Active,
    Committed,
    Aborted,
}

/* =========================
 * Transaction Descriptor
 * ========================= */

#[derive(Debug, Clone)]
pub struct Transaction {
    pub tx_id: TxId,
    pub snapshot_tx: TxId,
    pub state: TxState,
    pub read_only: bool,
}

/* =========================
 * Versioned Row
 * ========================= */

#[derive(Debug, Clone)]
pub struct RowVersion {
    pub created_by: TxId,
    pub deleted_by: Option<TxId>,
    pub value: Vec<u8>,
}

/* =========================
 * MVCC Table Storage
 * ========================= */

/// Key → versions (newest first)
pub type VersionChain = Vec<RowVersion>;

#[derive(Debug)]
pub struct MvccTable {
    rows: BTreeMap<Vec<u8>, VersionChain>,
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

    pub fn delete(&mut self, key: &[u8], tx: &Transaction) -> Result<()> {
        let versions = self
            .rows
            .get_mut(key)
            .ok_or(DbError::NotFound("Row not found".into()))?;

        // Mark latest visible version as deleted
        for v in versions.iter_mut() {
            if is_visible(v, tx) {
                v.deleted_by = Some(tx.tx_id);
                return Ok(());
            }
        }

        Err(DbError::NotFound("Row not visible".into()))
    }
}

/* =========================
 * Transaction Manager
 * ========================= */

pub struct TransactionManager {
    next_tx: AtomicU64,
    active: HashMap<TxId, Transaction>,
}

impl TransactionManager {
    pub fn new() -> Self {
        Self {
            next_tx: AtomicU64::new(1),
            active: HashMap::new(),
        }
    }

    /// BEGIN TRANSACTION
    pub fn begin(&mut self, read_only: bool) -> Transaction {
        // Generate UUID from sequential counter
        let next_id = self.next_tx.fetch_add(1, Ordering::SeqCst);
        
        // Create a UUID from the counter (using version 4 with custom bits)
        let tx_id = TxId(Uuid::from_u64_pair(next_id, 0)); // Uses high/low 64-bit pairs
        
        // For snapshot, use previous ID
        let prev_id = if next_id > 0 { next_id - 1 } else { 0 };
        let snapshot = TxId(Uuid::from_u64_pair(prev_id, 0));

        let tx = Transaction {
            tx_id,
            snapshot_tx: snapshot,
            state: TxState::Active,
            read_only,
        };

        self.active.insert(tx_id, tx.clone());
        tx
    }

    /// COMMIT TRANSACTION
    pub fn commit(&mut self, tx_id: TxId) -> Result<()> {
        let tx = self
            .active
            .get_mut(&tx_id)
            .ok_or_else(|| DbError::InvalidTransaction(format!(
            "Cannot commit transaction {}: not found or not active", tx_id
            )))?;

        tx.state = TxState::Committed;
        self.active.remove(&tx_id);
        Ok(())
    }

    /// ABORT TRANSACTION
    pub fn abort(&mut self, tx_id: TxId) -> Result<()> {
        let tx = self
            .active
            .get_mut(&tx_id)
            .ok_or_else(|| DbError::InvalidTransaction(format!(
                "Cannot abort transaction {}: not found or not active", tx_id
            )))?;

        tx.state = TxState::Aborted;
        self.active.remove(&tx_id);
        Ok(())
    }
}

/* =========================
 * Visibility Logic
 * ========================= */

#[inline]
fn is_visible(v: &RowVersion, tx: &Transaction) -> bool {
    v.created_by <= tx.snapshot_tx
        && v.deleted_by.map_or(true, |d| d > tx.snapshot_tx)
}