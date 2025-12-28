# sabiDB Wire Protocol v1

This document defines the wire protocol between **sabiDB Server** and the **TypeScript SDK**.

The protocol is designed to be:
- Deterministic
- Low-latency
- Versioned
- WAL-ordered
- Reactive

---

## 1. Transport Overview

| Channel        | Purpose                     |
|---------------|-----------------------------|
| HTTP/1.1      | Queries, mutations, admin   |
| WebSocket     | Subscriptions, live updates |

---

## 2. Protocol Invariants

All protocol messages MUST satisfy:

1. Every mutation is assigned a **monotonic transaction ID (tx_id)**
2. All updates are ordered by `tx_id`
3. Clients may reconnect and resume from last `tx_id`
4. Server state can be rebuilt purely from WAL
5. Protocol is **versioned** and backward-compatible

---

## 3. Message Envelope

All messages use a common envelope.

```json
{
  "v": 1,
  "type": "QUERY | MUTATION | SUBSCRIBE | UNSUBSCRIBE | UPDATE | ERROR | ACK",
  "request_id": "uuid",
  "payload": {}
}
```

## 4. Transaction Identity

Every committed mutation produces:

```json
{
  "tx_id": 184467,
  "timestamp": 1735351345123
}
```

tx_id is strictly increasing
timestamp is informational only

## 5. HTTP API (Queries & Mutations)

### 5.1 Execute Query

POST /query

```json
{
  "sql": "SELECT id, name FROM users WHERE active = true",
  "args": [],
  "at_tx": 184400
}
```

Response

```json
{
  "rows": [
    { "id": 1, "name": "Sijibomi" }
  ],
  "read_tx": 184512
}
```

### 5.2 Execute Mutation

POST /mutate

```json
{
  "sql": "INSERT INTO users(name) VALUES (?)",
  "args": ["Ada"],
  "client_tx": 184511
}
```

client_tx => Last known committed tx

Response

```json
{
  "tx_id": 184512,
  "affected_rows": 1
}
```

## 6. Deterministic Function Execution

POST /fn/{name}

```json
{
  "args": {
    "userId": 42,
    "email": "ada@example.com"
  }
}
```

Response

```json
{
  "tx_id": 184513,
  "result": {
    "success": true
  }
}
```
Functions run inside a transaction
All writes are WAL-logged
No side effects allowed

## 7. WebSocket Protocol (Realtime)

### 7.1 Connection Handshake

```json
{
  "type": "HELLO",
  "last_seen_tx": 184500
}
```

Server Response

```json
{
  "type": "WELCOME",
  "current_tx": 184520
}
```

If last_seen_tx < current_tx, server may replay missed updates.

## 8. Subscriptions

### 8.1 Subscribe

```json
{
  "type": "SUBSCRIBE",
  "request_id": "uuid",
  "payload": {
    "query": "SELECT * FROM users WHERE active = true",
    "args": []
  }
}
```

Server ACK

```json
{
  "type": "ACK",
  "request_id": "uuid",
  "payload": {
    "subscription_id": "sub-123"
  }
}
```

### 8.2 Live Update

```json
{
  "type": "UPDATE",
  "payload": {
    "subscription_id": "sub-123",
    "tx_id": 184521,
    "rows": [
      { "id": 7, "name": "Zainab", "active": true }
    ]
  }
}
```

### 8.3 Unsubscribe

```json
{
  "type": "UNSUBSCRIBE",
  "payload": {
    "subscription_id": "sub-123"
  }
}
```

## 9. Error Handling

this is the format of errors send to client if an issue occured processing the request

```json
{
  "type": "ERROR",
  "request_id": "uuid",
  "payload": {
    "code": "SERIALIZATION_FAILURE | SYNTAX_ERROR | INTERNAL",
    "message": "Human-readable error"
  }
}
```

### 9.1 Direction and Responsibility

ERROR messages are always sent by the server

Clients never emit ERROR envelopes

Each error corresponds to exactly one client request

The request_id allows the client to deterministically associate the error with the originating request.

### 9.2 Error Codes

The code field classifies the error into a small, deterministic set of categories.

#### SYNTAX_ERROR

Occurs when:

1. SQL parsing fails
2. Invalid SQL grammar is detected
3. Referenced tables or columns do not exist

Properties:
Deterministic
Safe to surface immediately to the user
Retrying will not succeed
No WAL entry is created

#### SERIALIZATION_FAILURE

Occurs when:

MVCC detects a write-write conflict
Snapshot isolation rules are violated
Concurrent transactions cannot be safely serialized

Properties:

Deterministic given transaction ordering

The transaction is aborted before commit

No WAL entry is created

Clients are expected to retry

This is equivalent to PostgreSQL’s 40001 error class.


#### INTERNAL

Occurs when:

An unexpected invariant is violated
A bug or corrupted state is encountered
The server cannot safely continue execution

Properties:

Indicates a server-side failure

Retrying may or may not succeed

No WAL entry is created

Typically logged with high severity

## 10. Resume & Replay Semantics

Clients MUST store:
last_committed_tx
active_subscriptions


On reconnect:
Send HELLO with last tx
Server replays missing updates
Subscriptions resume automatically

## 11. Ordering Guarantees

UPDATE messages are strictly ordered by tx_id
No UPDATE is sent before commit
Queries see consistent snapshots