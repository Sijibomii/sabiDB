## Project Structure

sabi/
├── crates/
│   ├── sabi-core/          # Shared types, errors, protocol definitions
│   │   └── src/
│   │       ├── error.rs
│   │       ├── ids.rs
│   │       └── protocol.rs
│   │
│   ├── sabi-storage/       # Storage engine
│   │   └── src/
│   │       ├── wal.rs
│   │       ├── page.rs
│   │       ├── btree.rs
│   │       ├── lsm.rs
│   │       └── mvcc.rs
│   │
│   ├── sabi-sql/           # SQL parsing and execution
│   │   └── src/
│   │       ├── parser.rs
│   │       ├── planner.rs
│   │       └── executor.rs
│   │
│   └── sabi-server/        # Runtime & networking
│       └── src/
│           ├── main.rs
│           ├── protocol.rs
│           ├── runtime/
│           │   ├── tx.rs
│           │   ├── functions.rs
│           │   └── subscriptions.rs
│           └── transport/
│               └── websocket.rs
