SQL Queries
    ↓
MVCC Layer (visibility, transaction rules)
    ↓
B+Tree Layer (logical index - THIS CODE)
    ↓
PageFile Layer (4KB page management)
    ↓
File System (actual disk storage)

The B+Tree sits in the middle: it provides fast key-value lookup while storing data in fixed-size pages.

// Example: Storing user records
struct User {
    id: u64,
    name: String,
    email: String,
}


1. SQL: INSERT INTO users VALUES (1, 'Alice', 'alice@email.com')
2. MVCC: Creates transaction, assigns TxId
3. Serialization: Convert User to bytes
4. PageFile: Allocate page for user data → returns PageId 123
5. BTree: Insert key=b"user:1", value=PageId(123)
6. WAL: Log the operation for durability

// Create BTree for user_id index
let mut pages = PageFile::open("users.db")?;
let wal = WalWriter::open("users.wal")?;
let mut user_index = BTree::new(pages, wal)?;

// Insert users
user_index.insert(b"user:1".to_vec(), 100)?;  // Page 100 stores Alice's data
user_index.insert(b"user:2".to_vec(), 101)?;  // Page 101 stores Bob's data
user_index.insert(b"user:3".to_vec(), 102)?;  // Page 102 stores Charlie's data

// Point lookup
let page_id = user_index.get(b"user:2")?;  // Returns Some(101)
let page = pages.read_page(page_id.unwrap())?;
let user_data = deserialize_user(&page.payload)?;

// Range query: users 1-3
let results = user_index.range(b"user:1", b"user:3")?;
// Returns [(b"user:1", 100), (b"user:2", 101), (b"user:3", 102)]


-- This creates a B+Tree internally
CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    name TEXT
);

-- BTree stores: key=b"users:id:1", value=PageId(where row data lives)



CREATE INDEX idx_users_name ON users(name);

-- BTree stores: key=b"users:name:Alice", value=PageId(of user record)



CREATE INDEX idx_users_email_domain ON users(email, domain);

-- BTree key: b"users:email:alice@company.com:domain:company.com"