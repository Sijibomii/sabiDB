use sabi_core::TxId;
use sabi_core::error::{DbError, Result};

use crate::page::{Page, PageType, PageId};
use crate::page_file::PageFile;
use crate::wal::{WalRecord, WalWriter};

/* ============================================================
    B -Tree+ Overview
    for internal node

    Each key acts as a separator between child subtrees
    For n keys, you need n+1 children because:
    One child for keys less than the first key
    One child between each pair of keys
    One child for keys greater than the last key

                    [20, 35, 50]
                   /    |    |    \
                  /     |    |     \
           <20     20-35   35-50    >50

    This node has:

    3 keys (20, 35, 50)

    4 children (pointers to subtrees)


    Leaf nodes are where the actual data resides in a B+ tree.

    Internal node: keys + child pointers
    keys: [20, 35, 50]
    children: [ptr0, ptr1, ptr2, ptr3]

    Leaf node: keys + actual values  
    keys: [20, 35, 50]
    values: [data20, data35, data50]

    Search for key 40:
    Start at root [25, 55]

    40 > 25 → go right child

    Reach internal node [40, 50] 

    40 ≥ 40 → go to appropriate leaf

    Find key 40 in Leaf3, return its value


    Range Query for keys 30-45:
    Find key 30 (locates Leaf2)

    Scan Leaf2: find 30

    Follow "next" pointer to Leaf3

    Scan Leaf3 until key > 45

    Return values for keys 30, 40


    ======= insertion =======

    Insert key 35 with value data35:

    Root: [25, 55]
    Go to right of 25, left of 55
    Reach Leaf2: [25, 30] | Next → Leaf3

    Leaf2 before: [25, 30]
    Leaf2 after:  [25, 30, 35]  (if capacity allows)

    Leaf2: [25, 30, 35, 40]  ← OVERFLOW!

    Split at midpoint:
    - Left leaf: [25, 30] | Next → Right leaf
    - Right leaf: [35, 40] | Next → Leaf3

    Promote smallest key from right leaf to parent:
    Parent before: [25, 55]
    Parent after:  [25, 35, 55]


    ========== deletion ==========

    Delete key 30:

    Leaf2: [25, 30, 35] → Remove 30 → [25, 35]

    If min keys = 2 and Leaf2 now has 1 key:

    Leaf1: [10, 20] | Next → Leaf2
    Leaf2: [25]     | Next → Leaf3  ← UNDERFLOW!

    Option 1: Redistribute with sibling
    Leaf1 gives 20 to Leaf2: [10] and [20, 25]
    Update parent separator from 25 to 20

    Option 2: Merge with sibling  
    Merge Leaf2 into Leaf1: [10, 20, 25]
    Remove Leaf2, update parent and next pointers


 * ============================================================
 */




/* ============================================================
 * MVCC decides which version is visible
 * B-Tree decides where the data lives
 * SQL
    ↓
    MVCC (visibility, tx rules)
    ↓
    B-Tree (logical index)
    ↓
    PageFile (4KB pages)
    ↓
    File
 * ============================================================
 */

const MAX_KEYS: usize = 128;
const BTREE_TABLE_ID: usize = 1;

// todo: should this be public?? check later

#[derive(Debug, Clone)]
pub struct NodeHeader {
    is_leaf: bool,
    key_count: u16,
    parent: Option<PageId>,
}

#[derive(Debug, Clone)]
pub struct LeafNode {
    header: NodeHeader,
    next: Option<PageId>,
    keys: Vec<Vec<u8>>,
    values: Vec<PageId>,
}

#[derive(Debug)]
pub struct InternalNode {
    header: NodeHeader,
    keys: Vec<Vec<u8>>,
    children: Vec<PageId>,
}

#[derive(Debug)]
pub enum Node {
    Leaf(LeafNode),
    Internal(InternalNode),
}

// extend WalWriter impl
impl WalWriter {
    pub fn log_btree_insert(&mut self, key: &[u8], value: PageId, tx_id: TxId) -> Result<()> {
        let record = WalRecord::Put {
            tx: tx_id,
            table_id:  BTREE_TABLE_ID as u32,
            key: key.to_vec(),
            value: value.to_le_bytes().to_vec(),
        };
        
        self.append(record)
    }

    pub fn log_btree_delete(&mut self, key: &[u8], tx_id: TxId) -> Result<()> {
        let record = WalRecord::Delete {
            tx: tx_id,
            table_id:  BTREE_TABLE_ID as u32,
            key: key.to_vec(),
        };
        self.append(record)
    }

    // pub fn log_btree_split_leaf(&mut self, leaf_id: PageId, tx_id: TxId) -> Result<()> {
    //     let record = WalRecord::Put {
    //         tx: tx_id,
    //         table_id: 2,
    //         key: b"split_leaf".to_vec(),
    //         value: leaf_id.to_le_bytes().to_vec(),
    //     };
        
    //     self.append(record)
    // }
    
    // pub fn log_btree_split_internal(&mut self, node_id: PageId) -> Result<()> {
    //     let dummy_tx = TxId(uuid::Uuid::nil());
    //     let record = WalRecord::Put {
    //         tx: dummy_tx,
    //         table_id: 3,
    //         key: b"split_internal".to_vec(),
    //         value: node_id.to_le_bytes().to_vec(),
    //     };
        
    //     self.append(record)
    // }
    
    // pub fn log_btree_new_root(&mut self, left: PageId, right: PageId) -> Result<()> {
    //     let dummy_tx = TxId(uuid::Uuid::nil());
    //     let mut value = Vec::new();
    //     value.extend_from_slice(&left.to_le_bytes());
    //     value.extend_from_slice(&right.to_le_bytes());
        
    //     let record = WalRecord::Put {
    //         tx: dummy_tx,
    //         table_id: 4,
    //         key: b"new_root".to_vec(),
    //         value,
    //     };
        
    //     self.append(record)
    // }
}


// extra page implementations specific to BTree nodes
impl Page {

    // Converts raw page bytes to Node structure
     pub fn deserialize_btree(&self) -> Result<Node> {
        if self.header.page_type != PageType::BTreeLeaf && 
           self.header.page_type != PageType::BTreeInternal {
            return Err(DbError::Corruption("Not a BTree page".into()));
        }
        
        let is_leaf = self.header.page_type == PageType::BTreeLeaf;
        
        // Read from payload
        let mut cursor = 0;
        
        // Read key_count
        if cursor + 2 > self.payload.len() {
            return Err(DbError::Corruption("Truncated BTree page".into()));
        }

        let key_count = u16::from_le_bytes(
            [self.payload[cursor], self.payload[cursor + 1]]
        );
        cursor += 2;
        
        // Read parent
        let parent = if cursor + 9 <= self.payload.len() && self.payload[cursor] != 0 {
            cursor += 1;
            let parent_bytes: [u8; 8] = self.payload[cursor..cursor + 8]
                .try_into()
                .map_err(|_| DbError::Corruption("Invalid parent ID".into()))?;
            cursor += 8;
            Some(u64::from_le_bytes(parent_bytes))
        } else {
            cursor += 1; // Skip the 0 byte
            None
        };
        
        let header = NodeHeader {
            is_leaf,
            key_count,
            parent,
        };
        
        if is_leaf {
            // Read leaf node
            let next = if cursor + 9 <= self.payload.len() && self.payload[cursor] != 0 {
                cursor += 1;
                let next_bytes: [u8; 8] = self.payload[cursor..cursor + 8]
                    .try_into()
                    .map_err(|_| DbError::Corruption("Invalid next pointer".into()))?;
                cursor += 8;
                Some(u64::from_le_bytes(next_bytes))
            } else {
                cursor += 1;
                None
            };
            
            let mut keys = Vec::new();
            let mut values = Vec::new();
            
            for _ in 0..key_count {
                // Read key length
                if cursor + 2 > self.payload.len() {
                    return Err(DbError::Corruption("Truncated key length".into()));
                }
                let key_len = u16::from_le_bytes(
                    [self.payload[cursor], self.payload[cursor + 1]]
                ) as usize;
                cursor += 2;
                
                // Read key
                if cursor + key_len > self.payload.len() {
                    return Err(DbError::Corruption("Truncated key".into()));
                }
                let key = self.payload[cursor..cursor + key_len].to_vec();
                cursor += key_len;
                
                // Read value (PageId)
                if cursor + 8 > self.payload.len() {
                    return Err(DbError::Corruption("Truncated value".into()));
                }
                let value_bytes: [u8; 8] = self.payload[cursor..cursor + 8]
                    .try_into()
                    .map_err(|_| DbError::Corruption("Invalid value".into()))?;
                let value = u64::from_le_bytes(value_bytes);
                cursor += 8;
                
                keys.push(key);
                values.push(value);
            }
            
            Ok(Node::Leaf(LeafNode {
                header,
                next,
                keys,
                values,
            }))
        } else {
            // Read internal node
            let mut keys = Vec::new();
            let mut children = Vec::new();
            
            // Read first child
            if cursor + 8 > self.payload.len() {
                return Err(DbError::Corruption("Truncated first child".into()));
            }
            let first_child_bytes: [u8; 8] = self.payload[cursor..cursor + 8]
                .try_into()
                .map_err(|_| DbError::Corruption("Invalid first child".into()))?;
            let first_child = u64::from_le_bytes(first_child_bytes);
            cursor += 8;
            children.push(first_child);
            
            for _ in 0..key_count {
                // Read key length
                if cursor + 2 > self.payload.len() {
                    return Err(DbError::Corruption("Truncated key length".into()));
                }
                let key_len = u16::from_le_bytes(
                    [self.payload[cursor], self.payload[cursor + 1]]
                ) as usize;
                cursor += 2;
                
                // Read key
                if cursor + key_len > self.payload.len() {
                    return Err(DbError::Corruption("Truncated key".into()));
                }
                let key = self.payload[cursor..cursor + key_len].to_vec();
                cursor += key_len;
                
                // Read child
                if cursor + 8 > self.payload.len() {
                    return Err(DbError::Corruption("Truncated child".into()));
                }
                let child_bytes: [u8; 8] = self.payload[cursor..cursor + 8]
                    .try_into()
                    .map_err(|_| DbError::Corruption("Invalid child".into()))?;
                let child = u64::from_le_bytes(child_bytes);
                cursor += 8;
                
                keys.push(key);
                children.push(child);
            }
            
            Ok(Node::Internal(InternalNode {
                header,
                keys,
                children,
            }))
        }
    }
    

    /*
        Memory Layout Example
        Leaf Node Serialization:
        [PageType] [KeyCount:u32] [ParentPresent:u8] [Parent:u64?] 
        [NextPresent:u8] [Next:u64?] [(KeyLen:u16, Key:bytes, Value:u64)...]

        Internal Node Serialization:
        [PageType] [KeyCount:u32] [ParentPresent:u8] [Parent:u64?] 
        [FirstChild:u64] [(KeyLen:u16, Key:bytes, Child:u64)...]
     */

    // Converts Node structure to raw page bytes
    pub fn serialize_btree(&mut self, node: &Node) -> Result<()> {
        // Clear payload
        self.payload.clear();
        
        match node {
            Node::Leaf(leaf) => {
                self.header.page_type = PageType::BTreeLeaf;
                
                // Write key_count
                self.payload.extend_from_slice(&leaf.header.key_count.to_le_bytes());
                
                // Write parent
                if let Some(parent) = leaf.header.parent {
                    self.payload.push(1); // Mark as present
                    self.payload.extend_from_slice(&parent.to_le_bytes());
                } else {
                    self.payload.push(0); // Mark as absent
                }
                
                // Write next pointer
                if let Some(next) = leaf.next {
                    self.payload.push(1); // Mark as present
                    self.payload.extend_from_slice(&next.to_le_bytes());
                } else {
                    self.payload.push(0); // Mark as absent
                }
                
                // Write keys and values
                for (key, value) in leaf.keys.iter().zip(leaf.values.iter()) {
                    // Write key length
                    let key_len = key.len().min(u16::MAX as usize) as u16;
                    self.payload.extend_from_slice(&key_len.to_le_bytes());
                    
                    // Write key
                    self.payload.extend_from_slice(key);
                    
                    // Write value
                    self.payload.extend_from_slice(&value.to_le_bytes());
                }
            }
            
            Node::Internal(internal) => {
                self.header.page_type = PageType::BTreeInternal;
                
                // Write key_count
                self.payload.extend_from_slice(&internal.header.key_count.to_le_bytes());
                
                // Write parent
                if let Some(parent) = internal.header.parent {
                    self.payload.push(1); // Mark as present
                    self.payload.extend_from_slice(&parent.to_le_bytes());
                } else {
                    self.payload.push(0); // Mark as absent
                }
                
                // Write first child
                // Internal nodes have n+1 children for n keys so first child is stored separately, then each subsequent child follows a key
                // Child[0] | Key[0] | Child[1] | Key[1] | Child[2] | ... | Key[n-1] | Child[n]
                self.payload.extend_from_slice(&internal.children[0].to_le_bytes());
                
                // Write keys and remaining children
                for (key, child) in internal.keys.iter().zip(internal.children[1..].iter()) {
                    // Write key length
                    let key_len = key.len().min(u16::MAX as usize) as u16;
                    self.payload.extend_from_slice(&key_len.to_le_bytes());
                    
                    // Write key
                    self.payload.extend_from_slice(key);
                    
                    // Write child
                    self.payload.extend_from_slice(&child.to_le_bytes());
                }
            }
        }
        
        self.header.payload_len = self.payload.len() as u32;
        Ok(())
    }
    
    //  A parent pointer tells a node which page contains its parent node in the tree hierarchy
    pub fn read_parent(&self) -> Option<PageId> {
        // Quick check: not a BTree page
        if self.header.page_type != PageType::BTreeLeaf && 
           self.header.page_type != PageType::BTreeInternal {
            return None;
        }
        
        if self.payload.len() < 3 {
            return None;
        }
        
        // Skip key_count (2 bytes)
        if self.payload[2] != 0 && self.payload.len() >= 11 {
            // read the parent details
            let parent_bytes: [u8; 8] = match self.payload[3..11].try_into() {
                Ok(bytes) => bytes,
                Err(_) => return None,
            };
            Some(u64::from_le_bytes(parent_bytes))
        } else {
            None
        }
    }
    
    pub fn write_parent(&mut self, parent: PageId) -> Result<()> {
        // Quick check: not a BTree page
        if self.header.page_type != PageType::BTreeLeaf && 
           self.header.page_type != PageType::BTreeInternal {
            return Err(DbError::Internal("Not a BTree page".into()));
        }
        
        if self.payload.len() < 3 {
            return Err(DbError::Corruption("Page too small for parent".into()));
        }
        
        // Ensure we have enough space
        if self.payload.len() < 11 {
            // Resize payload
            self.payload.resize(11, 0);
        }
        
        // Mark parent as present and write it into the parent position
        self.payload[2] = 1;
        self.payload[3..11].copy_from_slice(&parent.to_le_bytes());
        
        Ok(())
    }
}


pub struct BTree {
    root: PageId,
    pages: PageFile,
    wal: WalWriter,
}

impl BTree {
        
        pub fn new(mut pages: PageFile, wal: WalWriter) -> Result<Self> {
        let root = pages.allocate_page()?;
        let leaf = LeafNode {
            header: NodeHeader {
                is_leaf: true,
                key_count: 0,
                parent: None,
            },
            next: None,
            keys: vec![],
            values: vec![],
        };

        // Write node details to disk
        write_node(&mut pages, root, &Node::Leaf(leaf))?;
        Ok(Self { root, pages, wal })
    }

    // PageId where data is stored for a given key.
    pub fn get(&mut self, key: &[u8]) -> Result<Option<PageId>> {
        let mut current = self.root;

        loop {
            // Read page from disk
            let node = read_node(&mut self.pages, current)?;
            match node {
                Node::Leaf(leaf) => {
                    for (i, k) in leaf.keys.iter().enumerate() {
                        if k.as_slice() == key {  // Compare key bytes
                            // Found - return PageId
                            return Ok(Some(leaf.values[i]));
                        }
                    }
                    return Ok(None);
                }
                Node::Internal(internal) => {
                    // Find which child to follow based on key
                    let idx = find_child(&internal.keys, key);
                    // change current cursor child page
                    current = internal.children[idx];
                }
            }
        }
    }

    pub fn range(
        &mut self,
        start: &[u8],
        end: &[u8],
    ) -> Result<Vec<(Vec<u8>, PageId)>> {
        let mut out = Vec::new(); // Results accumulator
        let mut current = self.find_leaf(start)?; // Find first leaf containing 'start'

        loop {
            let leaf = match read_node(&mut self.pages, current)? {
                Node::Leaf(l) => l,
                _ => unreachable!(),
            };

            for (k, v) in leaf.keys.iter().zip(leaf.values.iter()) {
                // Use slice comparison instead of vector comparison to check if its within range
                if k.as_slice() >= start && k.as_slice() <= end {
                    out.push((k.clone(), *v));
                }
            }

            // Follow linked list to next leaf
            match leaf.next {
                Some(next) => current = next,
                None => break,
            }
        }

        Ok(out)
    }

    pub fn insert(&mut self, key: Vec<u8>, value: PageId, tx_id: TxId) -> Result<()> {

        self.wal.log_btree_insert(&key, value, tx_id)?;

        let leaf_id = self.find_leaf(&key)?;
        let mut leaf = match read_node(&mut self.pages, leaf_id)? {
            Node::Leaf(l) => l,
            _ => unreachable!(),
        };

        // Insert key-value pair in sorted order
        insert_sorted(&mut leaf.keys, &mut leaf.values, key.clone(), value);
        leaf.header.key_count += 1;

        // Check if leaf does not overflow
        if leaf.keys.len() <= MAX_KEYS {
            // update leaf on disk
            write_node(&mut self.pages, leaf_id, &Node::Leaf(leaf))?;
            return Ok(());
        }

        // it overflowed, need to split
        self.split_leaf(leaf_id, leaf)
    }

    fn split_leaf(&mut self, leaf_id: PageId, leaf: LeafNode) -> Result<()> {
        // no need to log internal operations like split
        // self.wal.log_btree_split_leaf(leaf_id)?;

        // Split point (middle)
        let mid = leaf.keys.len() / 2;

        // allocate new leaf page
        let new_leaf_id = self.pages.allocate_page()?;

        let new_leaf = LeafNode {
            header: NodeHeader {
                is_leaf: true,
                key_count: (leaf.keys.len() - mid) as u16,
                parent: leaf.header.parent,
            },
            next: leaf.next,
            keys: leaf.keys[mid..].to_vec(),
            values: leaf.values[mid..].to_vec(),
        };

        let mut left = leaf;
        left.keys.truncate(mid);
        left.values.truncate(mid);
        left.next = Some(new_leaf_id);
        left.header.key_count = mid as u16;

        // write both leaf nodes to disk
        write_node(&mut self.pages, leaf_id, &Node::Leaf(left))?;
        write_node(&mut self.pages, new_leaf_id, &Node::Leaf(new_leaf.clone()))?;

        let separator = new_leaf.keys[0].clone();
        self.insert_into_parent(leaf_id, separator, new_leaf_id)
    }

    // Parent with new child, handles parent splits.
    fn insert_into_parent(
        &mut self,
        left: PageId,
        key: Vec<u8>,
        right: PageId,
    ) -> Result<()> {
        // Get parent of left child
        let parent_id = match parent_of(&mut self.pages, left)? {
            Some(p) => p,
            None => {
                // No parent = create new root
                return self.new_root(left, key, right);
            }
        };

        let mut parent = match read_node(&mut self.pages, parent_id)? {
            Node::Internal(i) => i,
            _ => unreachable!(),
        };

        // Find insertion position
        let pos = find_child(&parent.keys, &key);
        // Insert separator key
        parent.keys.insert(pos, key);
        // Insert right child pointer
        parent.children.insert(pos + 1, right);
        parent.header.key_count += 1;

        // check that parent does not overflow
        if parent.keys.len() <= MAX_KEYS {
            write_node(&mut self.pages, parent_id, &Node::Internal(parent))?;
            return Ok(());
        }

        // if it does split!
        self.split_internal(parent_id, parent)
    }


    // Overflowing internal node, promotes middle key.
    fn split_internal(
        &mut self,
        node_id: PageId,
        node: InternalNode,
    ) -> Result<()> {
        // no need to log internal operations like split
        // self.wal.log_btree_split_internal(node_id)?;

         // Split point (middle)
        let mid = node.keys.len() / 2;
        // Key to promote to parent - the middle key
        let promote = node.keys[mid].clone();

        // New internal node
        let right_id = self.pages.allocate_page()?;

        // create right half
        let right = InternalNode {
            header: NodeHeader {
                is_leaf: false,
                key_count: (node.keys.len() - mid - 1) as u16,
                parent: node.header.parent,
            },
            keys: node.keys[mid + 1..].to_vec(),
            children: node.children[mid + 1..].to_vec(),
        };

        // previous not becomes left here
        let mut left = node;
        left.keys.truncate(mid);
        left.children.truncate(mid + 1);
        left.header.key_count = mid as u16;
        // write both internal nodes to disk
        write_node(&mut self.pages, node_id, &Node::Internal(left))?;
        write_node(&mut self.pages, right_id, &Node::Internal(right))?;

        self.insert_into_parent(node_id, promote, right_id)
    }

    // New root when tree grows in height.
    fn new_root(
        &mut self,
        left: PageId,
        key: Vec<u8>,
        right: PageId,
    ) -> Result<()> {
        // no need to log internal operations like split
        // self.wal.log_btree_new_root(left, right)?;

        let root_id = self.pages.allocate_page()?; // New root page

        // Create root internal node
        let root = InternalNode {
            header: NodeHeader {
                is_leaf: false,
                key_count: 1,
                parent: None, // Root has no parent
            },
            // Single separator key
            keys: vec![key],
            // Point to two children
            children: vec![left, right],
        };

        set_parent(&mut self.pages, left, root_id)?;
        set_parent(&mut self.pages, right, root_id)?;

        write_node(&mut self.pages, root_id, &Node::Internal(root))?;
        self.root = root_id;
        Ok(())
    }

    fn find_leaf(&mut self, key: &[u8]) -> Result<PageId> {
        let mut current = self.root;
        loop {
            let node = read_node(&mut self.pages, current)?;
            match node {
                Node::Leaf(_) => return Ok(current),
                Node::Internal(i) => {
                    let idx = find_child(&i.keys, key);
                    current = i.children[idx];
                }
            }
        }
    }
}

fn find_child(keys: &[Vec<u8>], key: &[u8]) -> usize {
    keys.iter()
        .position(|k| key < k.as_slice())
        .unwrap_or(keys.len())
}

fn insert_sorted(
    keys: &mut Vec<Vec<u8>>,
    values: &mut Vec<PageId>,
    key: Vec<u8>,
    value: PageId,
) {
    let pos = find_child(keys, &key);
    keys.insert(pos, key);
    values.insert(pos, value);
}

fn read_node(pages: &mut PageFile, id: PageId) -> Result<Node> {
    let page = pages.read_page(id)?;
    page.deserialize_btree()
}

fn write_node(pages: &mut PageFile, id: PageId, node: &Node) -> Result<()> {
    let mut page = Page::new(id, PageType::BTreeLeaf);
    page.serialize_btree(node)?;
    pages.write_page(&page)
}

fn parent_of(pages: &mut PageFile, id: PageId) -> Result<Option<PageId>> {
    let page = pages.read_page(id)?;
    Ok(page.read_parent())
}

fn set_parent(pages: &mut PageFile, id: PageId, parent: PageId) -> Result<()> {
    let mut page = pages.read_page(id)?;
    page.write_parent(parent)?;
    pages.write_page(&page)
}