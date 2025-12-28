# sabiDB  
### A deterministic, temporal-first SQL database with reactive compute — built in Rust

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)  
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)  
[![Status](https://img.shields.io/badge/status-in%20development-yellow.svg)]()

**sabiDB** is a **WAL-first, MVCC-based SQL database** with a **Convex-inspired deterministic runtime**.  
It treats **time, transactions, and reactivity as first-class concepts**, enabling replayable history, time-travel queries, and live subscriptions — all built from scratch in Rust.

---

## Why sabiDB?

sabiDB is built with two goals:

1. **Deeply understand how production databases actually work**
2. **Build a usable, principled foundation for reactive applications**

---

## Core Principles

### Deterministic by Design
- All writes are logged in a strict **Write-Ahead Log (WAL)**
- All mutations are **pure, replayable transactions**
- Database state can be rebuilt entirely from the WAL
- Ideal for auditing, debugging, and future replication

### Temporal First
- Every write creates a new version (MVCC)
- Query the database as it existed at any point in time
- Built-in audit history without triggers or application logic

### Reactive Compute
- Queries can be subscribed to
- Changes automatically propagate to clients
- Server-side functions run transactionally (Convex-style)

---

## Features

### Core Database
- ✅ **SQL-lite Engine**
  - CREATE TABLE, INSERT, SELECT, DELETE
  - Deterministic execution
- ✅ **ACID Transactions**
  - Snapshot isolation
  - Crash-safe commits
- ✅ **MVCC**
  - Readers never block writers
  - Versioned rows with garbage collection
- ✅ **Write-Ahead Log (WAL)**
  - Binary, checksummed, replayable
  - Source of truth for recovery
- ✅ **B+Tree Storage Engine**
  - Page-based layout
  - Predictable latency
  - WAL-backed structural changes

---

### Temporal Queries

```sql
-- Query data as it existed in the past
SELECT * FROM users
AS OF TIMESTAMP '2024-01-01 00:00:00';

-- Inspect historical versions
SELECT * FROM users
FOR SYSTEM_TIME BETWEEN '2024-01-01' AND '2024-12-31'
WHERE id = 42;
```


### Reactive Queries

```
const sub = await sabi.subscribe(
  "SELECT * FROM orders WHERE status = 'pending'"
);

sub.on("update", rows => {
  console.log("Pending orders changed:", rows);
});
```

## Architecture

┌──────────────────────────────────────┐
│ Client SDK (Rust / TypeScript)       │
│  - SQL execution                     │
│  - Reactive subscriptions            │
└──────────────┬───────────────────────┘
               │ HTTP / WebSocket
┌──────────────▼───────────────────────┐
│ Reactive Runtime                     │
│  - Deterministic functions           │
│  - Query dependency tracking         │
│  - Change propagation                │
└──────────────┬───────────────────────┘
               │
┌──────────────▼───────────────────────┐
│ SQL Engine                           │
│  - Parser                            │
│  - Planner                           │
│  - Volcano-style executor            │
└──────────────┬───────────────────────┘
               │
┌──────────────▼───────────────────────┐
│ Transaction Manager                  │
│  - MVCC (snapshot isolation)         │
│  - Transaction lifecycle             │
│  - Visibility rules                  │
└──────────────┬───────────────────────┘
               │
┌──────────────▼───────────────────────┐
│ Storage Engine                       │
│  - Write-Ahead Log (WAL)             │
│  - Page manager                      │
│  - B+Tree indexes                    │
└──────────────────────────────────────┘


### Getting Started

### License