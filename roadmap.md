# sabiDB Roadmap

This roadmap outlines the development phases for **sabiDB**, a deterministic, temporal-first SQL database with reactive compute, built from scratch in Rust.

The focus is on:
- WAL-first durability
- MVCC with snapshot isolation
- B+Tree storage
- SQL-lite execution
- Convex-style deterministic runtime
- Reactive queries with a TypeScript SDK

Distributed systems, LSM trees, and cloud-native concerns are intentionally out of scope.

---

## Overview

- **Scope**: Single-node database + client SDK
- **Primary Goal**: Deep understanding of database internals
- **Secondary Goal**: Product-quality developer experience
- **Total Timeline**: ~15–17 weeks

---

## Phase 1: Storage Foundations (Weeks 1–3)

**Goal:** Establish durability, crash safety, and disk layout

### Milestones

#### Write-Ahead Log (WAL)
- Binary WAL format (versioned, checksummed)
- Record types: BEGIN, PUT, DELETE, COMMIT, ABORT
- Sequential append-only writes
- fsync semantics
- Crash recovery via WAL replay

**Deliverable:**  
Crash-safe durable writes with WAL as the source of truth

---

#### Page & File Management
- Fixed-size page layout (4KB / 8KB)
- Page headers & checksums
- Page allocation & free list
- File abstraction (no mmap initially)

**Deliverable:**  
Persistent structured pages on disk

---

### Success Criteria
- Database recovers after `kill -9`
- Only committed data visible
- WAL fully replays state

---

## Phase 2: MVCC & Transactions (Weeks 4–5)

**Goal:** Snapshot isolation with deterministic transactions

### Milestones

- Monotonic transaction ID generator
- Versioned rows (`created_by`, `deleted_by`)
- MVCC visibility rules
- Read-only transaction optimization
- BEGIN / COMMIT / ABORT lifecycle

**Deliverable:**  
Concurrent readers & writers without blocking

---

### Success Criteria
- No dirty or non-repeatable reads
- Deterministic replay from WAL
- Readers never block writers

---

## Phase 3: B+Tree Storage Engine (Weeks 6–7)

**Goal:** Efficient indexed storage over pages

### Milestones

- B+Tree internal & leaf node layout
- Insert, search, range scan
- Node splitting & propagation
- WAL-logged structural changes

**Explicitly NOT included**
- LSM trees
- Compaction
- Bloom filters

**Deliverable:**  
Persistent, indexed access paths

---

### Success Criteria
- Index recovers correctly after crash
- Predictable read/write latency
- Correct WAL-driven rebuild

---

## Phase 4: SQL-lite Engine (Weeks 8–10)

**Goal:** Declarative querying over MVCC storage

### Milestones

#### SQL Parsing
- CREATE TABLE
- INSERT
- SELECT
- DELETE
- WHERE expressions
- Basic data types (INT, TEXT, BOOL)

#### Query Execution
- AST → Logical plan
- Table scan & index scan
- Filter & projection operators
- Volcano-style executor
- Snapshot-based execution

**Explicitly NOT included**
- Joins (initially)
- Cost-based optimization

**Deliverable:**  
Working SQL queries over transactional data

---

### Success Criteria
- Deterministic query execution
- Correct historical reads
- Clean parser / planner / executor separation

---

## Phase 5: Deterministic Runtime (Convex-like) (Weeks 11–12)

**Goal:** Transactional compute layer

### Milestones

- Deterministic function execution model
- Transaction-scoped execution context
- Automatic WAL logging
- Full replay from WAL
- Strong execution invariants

**Rules**
- No randomness
- No system time
- No external I/O

**Deliverable:**  
Replayable server-side functions

---

### Success Criteria
- Re-executing WAL yields identical state
- All mutations go through runtime
- Application logic is auditable

---

## Phase 6: Reactive Queries (Weeks 13–14)

**Goal:** Live queries that update automatically

### Milestones

- Query dependency tracking (keys → queries)
- Change detection on transaction commit
- Incremental query re-evaluation
- In-memory subscription registry

**Deliverable:**  
Live reactive queries inside the server

---

### Success Criteria
- Updates emitted immediately on commit
- No missed or duplicated updates
- Deterministic replay of subscriptions

---

## Phase 7: TypeScript SDK (Weeks 15–16)

**Goal:** Developer-friendly client for reactive apps

### Milestones

#### Connection & Transport
- HTTP for queries & mutations
- WebSocket for subscriptions
- Automatic reconnect logic
- Heartbeats & backpressure handling

#### Query Interface
- Raw SQL execution
- Typed query helpers (optional, minimal)
- Subscription API (`subscribe`, `unsubscribe`)
- Transaction-scoped function calls

#### Reactive API
- Stream-based subscriptions
- Batched updates
- Replay from transaction ID or timestamp

**Explicitly NOT included**
- ORM abstractions
- Client-side query planners
- Complex caching layers

**Deliverable:**  
Usable TypeScript SDK for browser & Node.js

---

### Success Criteria
- Subscribe and receive updates < 100ms
- Clean, minimal API surface
- No client-side nondeterminism

---

## Explicitly Out of Scope

- Replication & consensus (Raft)
- Distributed transactions
- LSM Trees
- Cloud-native scaling
- PostgreSQL wire protocol
- ORMs and heavy abstractions

---

## Final Outcome

By the end of this roadmap, sabiDB will be:

- A WAL-first, MVCC-based SQL database
- Deterministic and replayable
- Capable of reactive applications
- Paired with a clean TypeScript SDK

sabiDB prioritizes **correctness, clarity, and learning** over feature count.


