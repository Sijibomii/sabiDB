use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};
use sabi_core::{DbError, TxId};
use uuid::Uuid;

use crate::mvcc::{VersionChain, is_visible};
use crate::{btree::BTree, mvcc::{MvccTable, RowVersion, Transaction, TransactionManager}, page::{Page, PageType}, page_file::PageFile, wal::WalWriter};

/// Thread-safe storage engine with internal locking.
///
/// All fields are wrapped in Arc for shared ownership, allowing the engine
/// to be cloned and shared across threads. Internal synchronization is handled
/// via Mutex/RwLock on individual components.
pub struct StorageEngine {
    // Primary index (B+Tree mapping keys to data pages)
    pub primary_index: Arc<Mutex<BTree>>,

    // MVCC table storage (stores actual data with version chains)
    pub tables: Arc<RwLock<BTreeMap<String, MvccTable>>>,

    // Transaction manager (already internally thread-safe)
    pub tx_manager: Arc<TransactionManager>,

    // Page allocator (already Clone via internal Arc)
    pub pages: Arc<RwLock<PageFile>>,

    // Write-ahead log (already Clone via internal Arc)
    pub wal: WalWriter,
}

impl Clone for StorageEngine {
    fn clone(&self) -> Self {
        Self {
            primary_index: Arc::clone(&self.primary_index),
            tables: Arc::clone(&self.tables),
            tx_manager: Arc::clone(&self.tx_manager),
            pages: Arc::clone(&self.pages),
            wal: self.wal.clone(),
        }
    }
}

impl StorageEngine {
    pub fn new(pages: PageFile, wal: WalWriter) -> Result<Self, DbError> {
        let btree = BTree::new(pages.clone(), wal.clone())?;

        Ok(Self {
            primary_index: Arc::new(Mutex::new(btree)),
            tables: Arc::new(RwLock::new(BTreeMap::new())),
            tx_manager: Arc::new(TransactionManager::new()),
            pages: Arc::new(RwLock::new(pages)),
            wal,
        })
    }
    
    // Begin transaction
    pub fn begin(&self, read_only: bool) -> Transaction {
        self.tx_manager.begin(read_only)
    }
    
    // Commit transaction
    pub fn commit(&self, tx_id: TxId) -> Result<(), DbError> {
        self.tx_manager.commit(tx_id)?;
        self.wal.flush()?;  // Ensure WAL is persisted
        Ok(())
    }
    
    // Abort transaction
    pub fn abort(&self, tx_id: TxId) -> Result<(), DbError> {
        self.tx_manager.abort(tx_id)?;
        // Note: WAL records will be ignored on replay for aborted transactions
        Ok(())
    }

    /// Create a new table
    pub fn create_table(&self, table_name: &str, if_not_exists: bool) -> Result<(), DbError> {
        let mut tables = self.tables.write()
            .map_err(|_| DbError::Internal("Tables lock poisoned".into()))?;

        if if_not_exists && tables.contains_key(table_name) {
            return Ok(());
        }

        tables.insert(table_name.to_string(), MvccTable::new());
        Ok(())
    }

    /*
        // Memory table: "Alice" → [v1@tx1]
        // Page 100: [v1@tx1]
        // B+Tree: "table\0Alice" → 100

        // Memory table: "Alice" → [v1@tx1, v2@tx2]  ← Both versions in memory!
        // Page 101: [v1@tx1, v2@tx2]  ← Serializes BOTH versions to NEW page
        // B+Tree: "table\0Alice" → 101  ← Now points to page 101
     */
    
    // INSERT - MVCC-aware
    pub fn insert(&self, table_name: &str, key: Vec<u8>, value: Vec<u8>, tx: &Transaction) -> Result<(), DbError> {
        // 1. Allocate page for MVCC data
        let data_page_id = {
            let mut pages = self.pages.write()
                .map_err(|_| DbError::Internal("Pages lock poisoned".into()))?;
            pages.allocate_page()?
        };

        // 2. Store MVCC version in the data page
        let serialized = {
            let mut tables = self.tables.write()
                .map_err(|_| DbError::Internal("Tables lock poisoned".into()))?;
            let table = tables.entry(table_name.to_string())
                .or_insert_with(MvccTable::new);
            table.insert(key.clone(), value, tx);

            // 3. Serialize MVCC data to page
            Self::serialize_mvcc_data(table, &key)?
        };

        // Write page
        {
            let mut pages = self.pages.write()
                .map_err(|_| DbError::Internal("Pages lock poisoned".into()))?;
            let mut page = Page::new(data_page_id, PageType::Data);
            page.payload = serialized;
            pages.write_page(&page)?;
        }

        // 4. Update B-Tree index (key → data_page_id)
        let composite_key = create_composite_key(table_name, &key);
        {
            let mut index = self.primary_index.lock()
                .map_err(|_| DbError::Internal("Primary index lock poisoned".into()))?;
            index.insert(composite_key, data_page_id, tx.tx_id)?;
        }

        // 5. Log to WAL (already done in BTree::insert via WalWriter)
        Ok(())
    }
    
    // GET - MVCC-aware
    pub fn get(&self, table_name: &str, key: &[u8], tx: &Transaction) -> Result<Option<Vec<u8>>, DbError> {
        let composite_key = create_composite_key(table_name, key);

        // Look up data page ID from B-Tree
        let data_page_id = {
            let mut index = self.primary_index.lock()
                .map_err(|_| DbError::Internal("Primary index lock poisoned".into()))?;
            match index.get(&composite_key)? {
                Some(id) => id,
                None => return Ok(None),
            }
        };

        // Read the data page
        let page = {
            let mut pages = self.pages.write()
                .map_err(|_| DbError::Internal("Pages lock poisoned".into()))?;
            pages.read_page(data_page_id)?
        };

        // Deserialize MVCC versions
        let versions = Self::deserialize_mvcc_data(&page)?;

        // Find visible version for this transaction
        for version in versions {
            if is_visible(&version, tx) {
                return Ok(Some(version.value));
            }
        }

        Ok(None)
    }
    
    // DELETE - MVCC-aware (tombstone)
    pub fn delete(&self, table_name: &str, key: &[u8], tx: &Transaction) -> Result<(), DbError> {
        let composite_key = create_composite_key(table_name, key);

        // 1. Get data page ID
        let data_page_id = {
            let mut index = self.primary_index.lock()
                .map_err(|_| DbError::Internal("Primary index lock poisoned".into()))?;
            match index.get(&composite_key)? {
                Some(id) => id,
                None => return Err(DbError::NotFound("Key not found".into())),
            }
        };

        // 2. Read current data and update versions
        let serialized = {
            let mut pages = self.pages.write()
                .map_err(|_| DbError::Internal("Pages lock poisoned".into()))?;
            let page = pages.read_page(data_page_id)?;
            let mut versions = Self::deserialize_mvcc_data(&page)?;

            // 3. Mark latest visible version as deleted
            for version in versions.iter_mut() {
                if is_visible(version, tx) {
                    version.deleted_by = Some(tx.tx_id);
                    break;
                }
            }

            Self::serialize_versions(&versions)?
        };

        // 4. Write back updated versions
        {
            let mut pages = self.pages.write()
                .map_err(|_| DbError::Internal("Pages lock poisoned".into()))?;
            let mut page = Page::new(data_page_id, PageType::Data);
            page.payload = serialized;
            pages.write_page(&page)?;
        }

        // 5. Log deletion to WAL
        self.wal.log_btree_delete(key, tx.tx_id)?;

        Ok(())
    }
    
    // RANGE SCAN - MVCC-aware
    pub fn range_scan(
        &self,
        table_name: &str,
        start: &[u8],
        end: &[u8],
        tx: &Transaction,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        let mut results = Vec::new();

        // Create composite range bounds
        let composite_start = create_composite_key(table_name, start);
        let composite_end = create_composite_key(table_name, end);

        // 1. Get key-page_id pairs from B-Tree range (using composite keys)
        let key_page_pairs = {
            let mut index = self.primary_index.lock()
                .map_err(|_| DbError::Internal("Primary index lock poisoned".into()))?;
            index.range(&composite_start, &composite_end)?
        };

        // 2. For each composite key, extract original key and check visibility
        for (composite_key, page_id) in key_page_pairs {
            // Extract the original key from the composite key
            let original_key = extract_key_from_composite(&composite_key)
                .ok_or_else(|| DbError::Corruption("Invalid composite key format".into()))?
                .to_vec();

            let page = {
                let mut pages = self.pages.write()
                    .map_err(|_| DbError::Internal("Pages lock poisoned".into()))?;
                pages.read_page(page_id)?
            };
            let versions = Self::deserialize_mvcc_data(&page)?;

            // Find visible version
            for version in versions {
                if is_visible(&version, tx) && version.deleted_by.is_none() {
                    // Return the original key to the user, not the composite key
                    results.push((original_key, version.value));
                    break;
                }
            }
        }

        Ok(results)
    }

    pub fn get_table_entry(&self, table_name: &str, composite_key: &[u8]) -> Option<VersionChain> {
        let original_key = extract_key_from_composite(composite_key)?;
        let tables = self.tables.read().ok()?;
        tables.get(table_name)?.rows.get(original_key).cloned()
    }

    fn serialize_versions(versions: &[RowVersion]) -> Result<Vec<u8>, DbError> {
        let mut buf = Vec::new();
        
        // Write version count
        buf.extend_from_slice(&(versions.len() as u32).to_le_bytes());
        
        for version in versions {
            // Write created_by (UUID as 16 bytes)
            buf.extend_from_slice(version.created_by.0.as_bytes());
            
            // Write deleted_by (optional)
            if let Some(deleted_by) = version.deleted_by {
                buf.push(1);  // Present marker
                buf.extend_from_slice(deleted_by.0.as_bytes());
            } else {
                buf.push(0);  // Absent marker
            }
            
            // Write value length and value
            buf.extend_from_slice(&(version.value.len() as u32).to_le_bytes());
            buf.extend_from_slice(&version.value);
        }
        
        Ok(buf)
    }

    fn serialize_mvcc_data(table: &MvccTable, key: &[u8]) -> Result<Vec<u8>, DbError> {
        if let Some(versions) = table.rows.get(key) {
            // Call as associated function
            Self::serialize_versions(versions)
        } else {
            Ok(Vec::new())
        }
    }

    fn deserialize_mvcc_data(page: &Page) -> Result<Vec<RowVersion>, DbError> {
        let mut versions = Vec::new();
        let mut cursor = 0;
        
        if page.payload.len() < 4 {
            return Ok(versions);
        }
        
        // Read version count
        let count_bytes: [u8; 4] = page.payload[0..4].try_into()
            .map_err(|_| DbError::Corruption("Invalid version count".into()))?;
        let count = u32::from_le_bytes(count_bytes) as usize;
        cursor += 4;
        
        for _ in 0..count {
            // Read created_by (UUID)
            if cursor + 16 > page.payload.len() {
                return Err(DbError::Corruption("Truncated created_by".into()));
            }
            let created_by = Uuid::from_slice(&page.payload[cursor..cursor + 16])
                .map_err(|_| DbError::Corruption("Invalid UUID".into()))?;
            cursor += 16;
            
            // Read deleted_by marker
            if cursor >= page.payload.len() {
                return Err(DbError::Corruption("Truncated deleted_by marker".into()));
            }
            let deleted_by = if page.payload[cursor] == 1 {
                cursor += 1;
                if cursor + 16 > page.payload.len() {
                    return Err(DbError::Corruption("Truncated deleted_by".into()));
                }
                Some(Uuid::from_slice(&page.payload[cursor..cursor + 16])
                    .map_err(|_| DbError::Corruption("Invalid UUID".into()))?)
            } else {
                cursor += 1;
                None
            };
            if deleted_by.is_some() {
                cursor += 16;
            }
            
            // Read value
            if cursor + 4 > page.payload.len() {
                return Err(DbError::Corruption("Truncated value length".into()));
            }
            let len_bytes: [u8; 4] = page.payload[cursor..cursor + 4].try_into()
                .map_err(|_| DbError::Corruption("Invalid value length".into()))?;
            let value_len = u32::from_le_bytes(len_bytes) as usize;
            cursor += 4;
            
            if cursor + value_len > page.payload.len() {
                return Err(DbError::Corruption("Truncated value".into()));
            }
            let value = page.payload[cursor..cursor + value_len].to_vec();
            cursor += value_len;
            
            versions.push(RowVersion {
                created_by: TxId(created_by),
                deleted_by: deleted_by.map(TxId),
                value,
            });
        }
        
        Ok(versions)
    }
}

fn create_composite_key(table_name: &str, key: &[u8]) -> Vec<u8> {
    let mut composite = Vec::new();
    composite.extend_from_slice(table_name.as_bytes());
    composite.push(0); // separator
    composite.extend_from_slice(key);
    composite
}

fn extract_key_from_composite(composite: &[u8]) -> Option<&[u8]> {
    // Find the separator (0 byte)
    composite.iter().position(|&b| b == 0)
        .map(|pos| &composite[pos + 1..])
}

fn extract_key_from_composite_with_table(composite: &[u8]) -> Option<(&[u8], &[u8])> {
        // Find the separator (0 byte)
        let pos = composite.iter().position(|&b| b == 0)?;
        
        if pos == 0 || pos >= composite.len() - 1 {
            return None; // No table name or no key after separator
        }
        
        let table_name = &composite[..pos];
        let original_key = &composite[pos + 1..];
        Some((table_name, original_key))
    }