use std::collections::BTreeMap;
use sabi_core::{DbError, TxId};
use uuid::Uuid;

use crate::mvcc::{VersionChain, is_visible};
use crate::{btree::BTree, mvcc::{MvccTable, RowVersion, Transaction, TransactionManager}, page::{Page, PageType}, page_file::PageFile, wal::WalWriter};

pub struct StorageEngine {
    // Primary index (B+Tree mapping keys to data pages)
    // Maps composite keys to data page IDs
    pub primary_index: BTree,
    
    // MVCC table storage (stores actual data with version chains)
    // In-memory MVCC tables
    pub tables: BTreeMap<String, MvccTable>,
    
    // Transaction manager
    // Manages active transactions
    pub tx_manager: TransactionManager,
    
    // Page allocator
    // Manages disk pages
    pub pages: PageFile,
    
    // Write-ahead log
    // Write-ahead logger for durability
    pub wal: WalWriter,
}

impl StorageEngine {
    pub fn new(pages: PageFile, wal: WalWriter) -> Result<Self, DbError> {
        let btree = BTree::new(pages.clone(), wal.clone())?;
        
        Ok(Self {
            primary_index: btree,
            tables: BTreeMap::new(),
            tx_manager: TransactionManager::new(),
            pages,
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

    /*
        // Memory table: "Alice" → [v1@tx1]
        // Page 100: [v1@tx1]
        // B+Tree: "table\0Alice" → 100

        // Memory table: "Alice" → [v1@tx1, v2@tx2]  ← Both versions in memory!
        // Page 101: [v1@tx1, v2@tx2]  ← Serializes BOTH versions to NEW page
        // B+Tree: "table\0Alice" → 101  ← Now points to page 101
     */
    
    // INSERT - MVCC-aware
    pub fn insert(&mut self, table_name: &str, key: Vec<u8>, value: Vec<u8>, tx: &Transaction) -> Result<(), DbError> {
        // 1. Allocate page for MVCC data
        let data_page_id = self.pages.allocate_page()?;
        
        // 2. Store MVCC version in the data page
        let table = self.tables.entry(table_name.to_string())
            .or_insert_with(MvccTable::new);
        
        table.insert(key.clone(), value, tx);
        
        // 3. Serialize MVCC data to page
        // This only serializes that specific key's versions, not the entire table. So there's no issue of double serialization
        let serialized = Self::serialize_mvcc_data(&table, &key)?;

        let mut page = Page::new(data_page_id, PageType::Data);
        page.payload = serialized;
        self.pages.write_page(&page)?;
        
        // 4. Update B-Tree index (key → data_page_id)
        let composite_key = create_composite_key(table_name, &key);
        self.primary_index.insert(composite_key, data_page_id, tx.tx_id)?;
        
        // 5. Log to WAL (already done in BTree::insert via WalWriter)
        Ok(())
    }
    
    // GET - MVCC-aware
    pub fn get(&mut self, table_name: &str, key: &[u8], tx: &Transaction) -> Result<Option<Vec<u8>>, DbError> {

        let composite_key = create_composite_key(table_name, key);

        // Look up data page ID from B-Tree
        let data_page_id = match self.primary_index.get(&composite_key)? {
            Some(id) => id,
            None => return Ok(None),
        };
        
        // Read the data page
        let page = self.pages.read_page(data_page_id)?;
        
        // Deserialize MVCC versions
        let versions = self.deserialize_mvcc_data(&page)?;
        
        // Find visible version for this transaction
        for version in versions {
            if is_visible(&version, tx) {
                return Ok(Some(version.value));
            }
        }
        
        Ok(None)
    }
    
    // DELETE - MVCC-aware (tombstone)
    pub fn delete(&mut self, table_name: &str, key: &[u8], tx: &Transaction) -> Result<(), DbError> {
        // 1. Get data page ID

        let composite_key = create_composite_key(table_name, key);

        let data_page_id = match self.primary_index.get(&composite_key)? {
            Some(id) => id,
            None => return Err(DbError::NotFound("Key not found".into())),
        };
        
        // 2. Read current data
        let page = self.pages.read_page(data_page_id)?;
        let mut versions = self.deserialize_mvcc_data(&page)?;
        
        // 3. Mark latest visible version as deleted
        for version in versions.iter_mut() {
            if is_visible(version, tx) {
                version.deleted_by = Some(tx.tx_id);
                break;
            }
        }
        
        // 4. Write back updated versions
        let serialized = Self::serialize_versions(&versions)?;
        let mut page = Page::new(data_page_id, PageType::Data);
        page.payload = serialized;
        self.pages.write_page(&page)?;
        
        // 5. Log deletion to WAL (B-Tree doesn't actually delete, just marks tombstone)
        self.primary_index.wal.log_btree_delete(key, tx.tx_id)?;
        
        Ok(())
    }
    
    // RANGE SCAN - MVCC-aware
    pub fn range_scan(
        &mut self,
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
        let key_page_pairs = self.primary_index.range(&composite_start, &composite_end)?;
        
        // 2. For each composite key, extract original key and check visibility
        for (composite_key, page_id) in key_page_pairs {
            // Extract the original key from the composite key
            let original_key = extract_key_from_composite(&composite_key)
                .ok_or_else(|| DbError::Corruption("Invalid composite key format".into()))?
                .to_vec();
            
            let page = self.pages.read_page(page_id)?;
            let versions = self.deserialize_mvcc_data(&page)?;
            
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

    pub fn get_table_entry(&self, table_name: &str, composite_key: &[u8]) -> Option<&VersionChain> {
        let original_key = extract_key_from_composite(composite_key)?;
        self.tables.get(table_name)?.rows.get(original_key)
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

    fn deserialize_mvcc_data(&self, page: &Page) -> Result<Vec<RowVersion>, DbError> {
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