

/*
The WAL is an append-only log that records every database operation before it's applied to the main database
If the system crashes, all committed transactions can be replayed from the log
The WAL becomes the single source of truth for state reconstruction
Operations are durable once synced to disk

Header & Format

File starts with magic bytes SABIWAL and version number
Binary format with checksums (CRC32) for corruption detection
Each record has: [length | payload | checksum]

Record Types
Five transaction lifecycle operations:

Begin - Start transaction
Put - Insert/update key-value pair
Delete - Remove key
Commit - Transaction succeeds (triggers fsync)
Abort - Transaction fails

Writer

Appends records sequentially
Syncs to disk only on Commit (durability boundary)
Serializes records with transaction ID, table ID, keys, and values

Reader & Recovery

Validates file format and checksums
replay_wal() reconstructs database state by:

Tracking active/committed/aborted transactions
Filtering out aborted or incomplete transactions
Returning only committed operations for replay
*/

//! Write-Ahead Log (WAL)
//!
//! This module implements a WAL-first storage foundation.
//! The WAL is the source of truth for all state transitions.
//!
//! Guarantees:
//! - Sequential append-only writes
//! - Checksummed records
//! - Crash-safe commits
//! - Deterministic replay
//! - Versioned binary format

use std::collections::{HashSet};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::{Arc, RwLock};
use uuid::Uuid;
use crc32fast::Hasher;

use sabi_core::error::{DbError, Result};
use sabi_core::TxId;


const WAL_MAGIC: &[u8; 7] = b"SABIWAL";
const WAL_VERSION: u16 = 1;

/// WAL file header layout:
/// [ magic (7) | version (2) | reserved (2) ]
const WAL_HEADER_SIZE: usize = 11;

// repr(u8) in Rust is an attribute that forces an enum (or struct) to have a memory layout identical to an 8-bit unsigned integer
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalRecordType {
    Begin = 1,
    Put = 2,
    Delete = 3,
    Commit = 4,
    Abort = 5,
}

impl WalRecordType {
    fn from_u8(v: u8) -> Result<Self> {
        match v {
            1 => Ok(Self::Begin),
            2 => Ok(Self::Put),
            3 => Ok(Self::Delete),
            4 => Ok(Self::Commit),
            5 => Ok(Self::Abort),
            _ => Err(DbError::Corruption(format!(
                "Unknown WAL record type {}",
                v
            ))),
        }
    }
}


#[derive(Debug, Clone)]
pub enum WalRecord {
    Begin {
        tx: TxId,
    },
    Put {
        tx: TxId,
        table_id: u32,
        key: Vec<u8>,
        value: Vec<u8>,
    },
    Delete {
        tx: TxId, 
        table_id: u32,
        key: Vec<u8>,
    },
    Commit {
        tx: TxId,
    },
    Abort {
        tx: TxId,
    },
}

pub struct WalWriter {
    file: Arc<RwLock<File>>,
}

impl Clone for WalWriter {
    fn clone(&self) -> Self {
        Self{file: Arc::clone(&self.file)}
    }
}

impl WalWriter {
    pub fn flush(&self) -> io::Result<()> {
        let mut file = self.file.write().unwrap();
        file.flush()
    }
    
    pub fn write_all(&self, buf: &[u8]) -> io::Result<()> {
        let mut file = self.file.write().unwrap();
        file.write_all(buf)
    }
    
    pub fn sync_all(&self) -> io::Result<()> {
        let file = self.file.write().unwrap();
        file.sync_all()
    }
    
    pub fn sync_data(&self) -> io::Result<()> {
        let file = self.file.write().unwrap();
        file.sync_data()
    }
}

impl WalWriter {
    /// Open or create a WAL file.
    /// If the file is empty, write the WAL header.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(path)?;

        if file.metadata()?.len() == 0 {
            file.write_all(WAL_MAGIC)?;
            file.write_all(&WAL_VERSION.to_le_bytes())?;
            file.write_all(&0u16.to_le_bytes())?;
            file.sync_all()?;
        }

        Ok(Self { file: Arc::new(RwLock::new(file)) })
    }

    /// Append a WAL record.
    ///
    /// Layout:
    /// [ length (u32) | payload | checksum (u32) ]
    ///
    /// checksum = CRC32(payload)
    pub fn append(&self, record: WalRecord) -> Result<()> {
        let payload = serialize_record(&record);

        // Creates a CRC32 checksum of the payload. this checksum is used to verify the integrity of the data when reading it back. It Protects against data corruption
        let mut hasher = Hasher::new();
        hasher.update(&payload);
        let checksum = hasher.finalize();

        let len = payload.len() as u32;

        let mut file = self.file.write().unwrap();
        
        // write details to the file
        file.write_all(&len.to_le_bytes())?;
        file.write_all(&payload)?;
        file.write_all(&checksum.to_le_bytes())?;

        // fsync boundary:
        // durability guarantee at COMMIT
        if matches!(record, WalRecord::Commit { .. }) {
            // flushes OS buffers to disk to ensure data is physically stored. Only does this for commit transactions 
            //to optimize performance while ensuring durability

            /*
            let mut wal = WalWriter::open("database.wal")?;

            // Transaction 1
            wal.append(WalRecord::Insert { id: <uuid>, data: b"hello" })?;
            wal.append(WalRecord::Insert { id: <uuid>, data: b"world" })?;
            wal.append(WalRecord::Commit { tx_id: <uuid> })?;  // Flushed to disk!

            // Transaction 2 (in progress - not durable yet)
            wal.append(WalRecord::Update { id: <uuid>, data: b"updated" })?;
            // Crash here = transaction 2 lost (but 1 persists)
             */
            file.sync_data()?;
        }

        Ok(())
    }
}


pub struct WalReader {
    file: File,
}

impl WalReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut file = File::open(path)?;

        let mut header = [0u8; WAL_HEADER_SIZE];
        file.read_exact(&mut header)?;

        if &header[0..7] != WAL_MAGIC {
            return Err(DbError::Corruption("Invalid WAL magic".into()));
        }

        let version = u16::from_le_bytes([header[7], header[8]]);
        if version != WAL_VERSION {
            return Err(DbError::Corruption(format!(
                "Unsupported WAL version {}",
                version
            )));
        }

        Ok(Self { file })
    }

    /// Read next WAL record.
    /// Returns None on EOF.
    pub fn read_next(&mut self) -> Result<Option<WalRecord>> {
        // 4-byte buffer for length field
        let mut len_buf = [0u8; 4];
        // reads the length field byte-by-byte  
        let n = self.file.read(&mut len_buf)?;

        // end of file
        if n == 0 {
            return Ok(None);
        }

        // If we read less than 4 bytes (but more than 0), file is corrupted
        if n != 4 {
            return Err(DbError::Corruption("Truncated WAL length".into()));
        }

        // reads the length of the payload
        let len = u32::from_le_bytes(len_buf) as usize;

        // create a buffer to hold the payload
        let mut payload = vec![0u8; len];
        // read the payload into the buffer
        self.file.read_exact(&mut payload)?;

        // read the checksum (4 bytes)
        let mut checksum_buf = [0u8; 4];
        self.file.read_exact(&mut checksum_buf)?;
        let expected = u32::from_le_bytes(checksum_buf);

        // recompute the checksum of the payload and verify against expected
        let mut hasher = Hasher::new();
        hasher.update(&payload);
        let actual = hasher.finalize();

        if actual != expected {
            return Err(DbError::Corruption(
                "WAL checksum mismatch (possible crash)".into(),
            ));
        }

        Ok(Some(deserialize_record(&payload)?))
    }
}


fn serialize_record(record: &WalRecord) -> Vec<u8> {
    let mut buf = Vec::new();

    match record {
        WalRecord::Begin { tx } => {
            buf.push(WalRecordType::Begin as u8);
            // tx.0.to_bytes_le() ==> returns [u8; 16] - little-endian byte array representation of the UUID. 
            // least significant byte (LSB) of a multi-byte data type is stored at the lowest memory address
            buf.extend_from_slice(&tx.0.to_bytes_le());
        }
        WalRecord::Put {
            tx,
            table_id,
            key,
            value,
        } => {
            buf.push(WalRecordType::Put as u8);
            // put the txId as little-endian bytes
            buf.extend_from_slice(&tx.0.to_bytes_le());

            // put the table_id as little-endian bytes
            buf.extend_from_slice(&table_id.to_le_bytes());

            // store the length of the key and value as u32 in little-endian format
            buf.extend_from_slice(&(key.len() as u32).to_le_bytes());
            // push the key itself
            buf.extend_from_slice(key);
            
            // store the length of the value and push the value itself
            buf.extend_from_slice(&(value.len() as u32).to_le_bytes());
            buf.extend_from_slice(value);
        }
        WalRecord::Delete {
            tx,
            table_id,
            key,
        } => {
            buf.push(WalRecordType::Delete as u8);
            buf.extend_from_slice(&tx.0.to_bytes_le());
            buf.extend_from_slice(&table_id.to_le_bytes());

            buf.extend_from_slice(&(key.len() as u32).to_le_bytes());
            buf.extend_from_slice(key);
        }
        WalRecord::Commit { tx } => {
            buf.push(WalRecordType::Commit as u8);
            buf.extend_from_slice(&tx.0.to_bytes_le());
        }
        WalRecord::Abort { tx } => {
            buf.push(WalRecordType::Abort as u8);
            buf.extend_from_slice(&tx.0.to_bytes_le());
        }
    }

    buf
}


fn deserialize_record(payload: &[u8]) -> Result<WalRecord> {
    let mut cursor = 0;

    // 1. Read record type (1 byte)
    let record_type = WalRecordType::from_u8(payload[cursor])?;
    cursor += 1;

    // 2. Read transaction ID (16 bytes)
    let tx_bytes: [u8; 16] = payload[cursor..cursor + 16].try_into().unwrap();
    let tx = TxId(Uuid::from_bytes_le(tx_bytes));
    cursor += 16;

    // 3. Parse based on record type
    match record_type {
        WalRecordType::Begin => Ok(WalRecord::Begin { tx }),

        WalRecordType::Put => {
            // Read table_id (4 bytes)
            let table_id = u32::from_le_bytes(
                payload[cursor..cursor + 4].try_into().unwrap()
            );
            cursor += 4;

            // Read key length (4 bytes)
            let key_len = u32::from_le_bytes(
                payload[cursor..cursor + 4].try_into().unwrap()
            ) as usize;
            cursor += 4;

            //  Read key bytes
            let key = payload[cursor..cursor + key_len].to_vec();
            cursor += key_len;

            // Read value length (4 bytes)
            let val_len = u32::from_le_bytes(
                payload[cursor..cursor + 4].try_into().unwrap()
            ) as usize;
            cursor += 4;

            // Read value bytes
            let value = payload[cursor..cursor + val_len].to_vec();

            Ok(WalRecord::Put {
                tx,
                table_id,
                key,
                value,
            })
        }

        WalRecordType::Delete => {
            // Read table_id (4 bytes)
            let table_id = u32::from_le_bytes(
                payload[cursor..cursor + 4].try_into().unwrap()
            );
            cursor += 4;

            // Read key length (4 bytes)
            let key_len = u32::from_le_bytes(
                payload[cursor..cursor + 4].try_into().unwrap()
            ) as usize;
            cursor += 4;

            // Read key bytes
            let key = payload[cursor..cursor + key_len].to_vec();

            Ok(WalRecord::Delete {
                tx,
                table_id,
                key,
            })
        }

        WalRecordType::Commit => Ok(WalRecord::Commit { tx }),

        WalRecordType::Abort => Ok(WalRecord::Abort { tx }),
    }
}

/// Collect committed transactions and their records.
/// This will later drive MVCC recovery.
pub fn replay_wal(path: impl AsRef<Path>) -> Result<Vec<WalRecord>> {
    let mut reader = WalReader::open(path)?;

    let mut active = HashSet::new(); // Transactions in progress
    let mut committed = HashSet::new(); // Successfully completed transactions  
    let mut aborted = HashSet::new(); // Explicitly rolled back transactions
    let mut records = Vec::new(); // All records read from WAL

    while let Some(rec) = reader.read_next()? {
        match &rec {
            WalRecord::Begin { tx } => {
                active.insert(*tx);
            }
            WalRecord::Commit { tx } => {
                active.remove(tx);
                committed.insert(*tx);
            }
            WalRecord::Abort { tx } => {
                active.remove(tx);
                aborted.insert(*tx);
            }
            _ => {}
        }
        records.push(rec);
    }

    // Only return records belonging to committed transactions
    Ok(records
        .into_iter()
        .filter(|r| match r {
            WalRecord::Put { tx, .. } | WalRecord::Delete { tx, .. } => {
                committed.contains(tx)
            }
            WalRecord::Begin { tx } | WalRecord::Commit { tx } => {
                committed.contains(tx)
            }
            WalRecord::Abort { .. } => false,
        })
        .collect())
}
