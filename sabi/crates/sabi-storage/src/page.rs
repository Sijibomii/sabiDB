use crc32fast::Hasher;
use sabi_core::error::{DbError, Result};

/*
A page is the smallest unit of data that is transferred between main memory and secondary storage (usually a hard disk) 
in a paging-based virtual memory system. Pages are typically 4KB or 8KB in size and are managed by the operating system's 
memory management unit. Each page is mapped to a physical memory frame, and the mapping is tracked by the page table.
*/
pub const PAGE_SIZE: usize = 4096; // 4KB page size
pub const PAGE_MAGIC: &[u8; 4] = b"SABI";

#[repr(u16)] // Ensures exact 2-byte representation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageType {
    Free = 0,          // Unallocated page
    Data = 1,          // Raw data storage
    BTreeInternal = 2, // B-Tree internal node
    BTreeLeaf = 3,     // B-Tree leaf node
}   

pub type PageId = u64;

#[derive(Debug, Clone)]
pub struct PageHeader {
    pub page_type: PageType, // 2 bytes
    pub flags: u16,          // 2 bytes - custom flags (dirty, compressed, etc.)
    pub page_id: u64,        // 8 bytes - unique identifier
    pub lsn: u64,            // 8 bytes - Log Sequence Number (for WAL)
    pub payload_len: u32,    // 4 bytes - actual data size
}

#[derive(Debug, Clone)]
pub struct Page {
    pub header: PageHeader,
    pub payload: Vec<u8>, // actual data stored in the page
}

impl Page {
    pub fn new(page_id: u64, page_type: PageType) -> Self {
        Self {
            header: PageHeader {
                page_type,
                flags: 0,
                page_id,
                lsn: 0,
                payload_len: 0,
            },
            payload: Vec::new(),
        }
    }

    pub fn serialize(&self) -> Result<[u8; PAGE_SIZE]> {
        let mut buf = [0u8; PAGE_SIZE];

        // Write header fields
        let payload_length = self.payload.len() as u32;

        // page magic
        buf[0..4].copy_from_slice(PAGE_MAGIC);

        // page type
        buf[4..6].copy_from_slice(&(self.header.page_type as u16).to_le_bytes());

        // page flags
        buf[6..8].copy_from_slice(&self.header.flags.to_le_bytes());

        // page id
        buf[8..16].copy_from_slice(&self.header.page_id.to_le_bytes());

        // page lsn
        buf[16..24].copy_from_slice(&self.header.lsn.to_le_bytes());

        // payload length
        buf[24..28].copy_from_slice(&payload_length.to_le_bytes());

        let payload_start = 32; // header size is 32 bytes
        let payload_end = payload_start + self.payload.len();

        if payload_end > PAGE_SIZE {
            return Err(DbError::Corruption("Page payload overflow".into()));
        }

        // Write payload
        buf[payload_start..payload_end].copy_from_slice(&self.payload);

        // Compute checksum
        let mut hasher = Hasher::new();
        hasher.update(&buf[..28]); // header without checksum. that's why its 28
        // include payload in checksum
        hasher.update(&buf[payload_start..payload_end]);

        let checksum = hasher.finalize();
        // Write checksum at position 28-32
        buf[28..32].copy_from_slice(&checksum.to_le_bytes());

        Ok(buf)
    }

    pub fn deserialize(data: &[u8; PAGE_SIZE]) -> Result<Self> {
        // check magic value is correct
        if &data[0..4] != PAGE_MAGIC {
            return Err(DbError::Corruption("Invalid page magic".into()));
        }

        let page_type =
            PageType::try_from(u16::from_le_bytes([data[4], data[5]]))?;

        // read other header fields from little-endian bytes
        let flags = u16::from_le_bytes([data[6], data[7]]);
        let page_id = u64::from_le_bytes(data[8..16].try_into().unwrap());
        let lsn = u64::from_le_bytes(data[16..24].try_into().unwrap());
        let payload_len =
            u32::from_le_bytes(data[24..28].try_into().unwrap());

        let expected_checksum =
            u32::from_le_bytes(data[28..32].try_into().unwrap());

        // Verify checksum
        let mut hasher = Hasher::new();
        hasher.update(&data[..28]);
        hasher.update(&data[32..32 + payload_len as usize]);
        let actual_checksum = hasher.finalize();

        // if checksum does not match, return error
        if actual_checksum != expected_checksum {
            print!("Page checksum mismatch: actual={:?}, expected={:?} \n", actual_checksum, expected_checksum);
            return Err(DbError::Corruption("Page checksum mismatch".into()));
        }

        Ok(Page {
            header: PageHeader {
                page_type,
                flags,
                page_id,
                lsn,
                payload_len,
            },
            payload: data[32..32 + payload_len as usize].to_vec(),
        })
    }
}

impl TryFrom<u16> for PageType {
    type Error = DbError;

    fn try_from(value: u16) -> Result<Self> {
        match value {
            0 => Ok(PageType::Free),
            1 => Ok(PageType::Data),
            2 => Ok(PageType::BTreeInternal),
            3 => Ok(PageType::BTreeLeaf),
            _ => Err(DbError::Corruption("Unknown page type".into())),
        }
    }
}
