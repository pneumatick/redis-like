# Redis-like

A Redis-inspired, in-memory key–value server written in **Rust**. This project implements a growing subset of Redis commands and RESP2 behavior, with a focus on network protocol design, asynchronous TCP handling, concurrency, internal data representation, and key expiration.

Redis-like is an **educational systems-programming project**, not a production-ready replacement for Redis. It implements the server and protocol handling from scratch rather than embedding an existing Redis server.

## Features

- **Asynchronous TCP networking:** Tokio accepts client connections and handles each connection in a lightweight task.
- **I/O-independent RESP2 request parser:** Decodes arrays of bulk strings from byte slices; reports complete, incomplete, or invalid frames without performing network I/O.
- **Buffered connections:** Uses `BytesMut` to accumulate fragmented requests and preserve subsequent pipelined requests, with asynchronous response writes.
- **Binary-safe string storage:** Keys and string values can contain arbitrary bytes. Integer-valued strings may use a dedicated internal `i64` representation.
- **Shared database:** An `Arc<RwLock<Database>>` coordinates concurrent client access. Read–modify–write operations use exclusive locking.
- **Conditional and expiring writes:** `SET` supports `NX`, `XX`, `EX`, and `PX`.
- **Lazy and active expiration:** Expired keys are treated as absent on access; a dedicated cleanup thread periodically samples expiration deadlines and reclaims expired entries.
- **Integer arithmetic:** `INCR` and `DECR` perform checked signed 64-bit operations.
- **Automated testing:** Command behavior, RESP2 framing, fragmented and pipelined requests, Tokio connections, and concurrency are covered by Rust tests.

## Getting started

### Requirements

- Rust and Cargo with support for **Rust edition 2024** (Rust **1.85+**).
- Optional: [`redis-cli`](https://redis.io/docs/latest/develop/tools/cli/) to interact with the server. The workspace's `client` crate is still a placeholder.

### Build and run

```bash
git clone https://github.com/pneumatick/redis-like.git
cd redis-like
cargo run -p server
```

The server listens on **`127.0.0.1:43210`**. This endpoint is currently hard-coded in `server/src/main.rs`.

In another terminal, connect with RESP2 enabled:

```bash
redis-cli -2 -h 127.0.0.1 -p 43210
```

Example session (enter these commands at the `redis-cli` prompt):

```text
PING
SET greeting hello
GET greeting
SET session token NX EX 60
TTL session
SET counter 1
INCR counter
DECR counter
PEXPIRE session 5000
PERSIST session
DEL greeting counter session
```

Or send one command without opening an interactive session:

```bash
redis-cli -2 -h 127.0.0.1 -p 43210 SET message hello
redis-cli -2 -h 127.0.0.1 -p 43210 GET message
```

Only the commands listed below are implemented. Some Redis clients may issue additional commands automatically and receive an unknown-command response.

## Supported commands

| Command | Behavior |
| --- | --- |
| `PING [message]` | Check connectivity; optionally echo a bulk-string message. |
| `SET key value [EX seconds \| PX milliseconds] [NX \| XX]` | Store or replace a string, optionally with a deadline or an existence condition. |
| `GET key` | Return the value as a bulk string, or null for a missing or expired key. |
| `DEL key [key ...]` | Remove one or more keys; return the number removed. |
| `EXPIRE key seconds` | Set or update a deadline using seconds. |
| `PEXPIRE key milliseconds` | Set or update a deadline using milliseconds. |
| `TTL key` | Get remaining lifetime in seconds; `-1` means no expiration and `-2` means missing or expired. |
| `PERSIST key` | Remove a key's expiration. |
| `INCR key` | Increment an integer-valued string by one. |
| `DECR key` | Decrement an integer-valued string by one. |

For `SET`, expiration and existence conditions can appear in either supported order (for example, `SET k v EX 60 NX` or `SET k v NX EX 60`). Conflicting or duplicate options are rejected. A successful ordinary `SET` clears an existing expiration, while a failed conditional write leaves the existing entry unchanged.

This is a **subset of Redis**, not full command or protocol compatibility.

## Architecture

The project is organized as a Cargo workspace:

```text
redis-like/
├── Cargo.toml
├── common/
│   └── src/
│       ├── lib.rs
│       ├── resp.rs                  # RESP2 parser, reply types, encoder
│       └── resp_tests.rs            # Parser and encoding tests
├── server/
│   └── src/
│       ├── main.rs                  # Tokio listener and cleanup worker
│       ├── connection.rs            # Buffered asynchronous TCP I/O
│       ├── command.rs               # Argument validation and command execution
│       ├── db.rs                    # In-memory database and expiration logic
│       ├── command_tests.rs         # Command behavior and concurrency tests
│       └── connection_tokio_tests.rs # Connection and network integration tests
└── client/
    └── src/
        └── main.rs                  # Placeholder for a future CLI
```

### Request lifecycle

```text
Redis-compatible client
        │ TCP / RESP2
        ▼
Tokio TCP listener (main.rs)
        │ spawns one task per connection
        ▼
Buffered Connection (connection.rs)
        │ accumulates bytes in BytesMut
        ▼
RESP2 parser (common/src/resp.rs)
        │ complete / incomplete / malformed frame
        ▼
Command executor (command.rs)
        │ synchronously executes the operation
        ▼
Shared database (db.rs)
        │ Arc<RwLock<Database>>
        ▼
RESP2 reply encoder → async TCP write
```

The listener and socket operations are asynchronous; **database commands remain synchronous**. Each command's database locks are released before awaiting response writes. The shared `RwLock` allows concurrent readers or a single writer; operations that must read and modify a key, such as `SET NX` and `INCR`, execute under a write lock.

### RESP2 framing

`common::resp::parse_request(&[u8])` does **not** read from a socket or depend on an async runtime. It returns one of three outcomes:

- **Complete:** Parsed argument bytes and the number of input bytes consumed.
- **Incomplete:** More bytes are needed to finish a request.
- **Invalid:** Malformed data or a limit violation.

`Connection` handles the surrounding async I/O: it reads bytes into a `BytesMut` buffer, tries parsing, and advances the buffer only when a complete request is available. This handles TCP fragmentation and preserves any following pipelined requests.

Responses use the existing `RespValue` model and RESP2 serializer. They are serialized into a reusable buffer and written asynchronously. Incoming commands are currently expected as **arrays of non-null bulk strings**, rather than arbitrary RESP2 messages or inline commands.

The parser enforces bounds on argument count, individual bulk-string size, total request payload size, and numeric header length. Current defaults include a maximum of **1,024 arguments**, **1 MiB per bulk string**, and **4 MiB total bulk-string payload per request**. These are implementation-specific limits, not Redis's defaults.

### Storage and expiration

The database maintains two data structures:

- `HashMap<Vec<u8>, Entry>` holds the primary key–value entries.
- `IndexMap<Vec<u8>, SystemTime>` holds expiration deadlines for keys with a TTL.

`Value::String` currently supports both `StringValue::Bytes(Vec<u8>)` and `StringValue::Integer(i64)`. This distinction concerns *internal encoding*: integer-valued keys are still Redis strings, and `GET` returns their decimal byte representation.

Expiration uses two complementary mechanisms:

1. **Lazy expiration:** Commands treat expired keys as absent and remove them when appropriate. Read operations can inspect expiration under a read lock and acquire a write lock for cleanup when needed.
2. **Active expiration:** An ordinary background OS thread runs approximately every **100 ms**, samples up to **20 expiration entries per batch**, and repeats a batch when more than **25%** of sampled entries were removed. This worker shares the database lock with client tasks.

The separate expiration index allows cleanup to sample only keys with deadlines rather than scanning all persistent keys. The current cleanup scheduling and lock-holding behavior remain experimental.

## Testing

Run all workspace tests:

```bash
cargo test --workspace
```

Run each crate separately:

```bash
cargo test -p common
cargo test -p server
```

Run the focused suites (their module names reflect the current source registrations):

```bash
# I/O-independent RESP2 parser and response encoding
cargo test -p common resp::resp_tests

# Command semantics, expiration, and atomicity
cargo test -p server command::command_tests

# Tokio connection and TCP integration behavior
cargo test -p server command::tokio_tests
```

The test suites cover normal and invalid command arguments, conditional writes, arithmetic, expiration, binary payloads, partially received frames, multiple requests per connection, EOF behavior, and concurrent clients. Tokio network tests use **ephemeral local ports**, so the main server does not need to be running for them.

## Current limitations

- **No persistence:** All data is lost when the server stops. AOF/RDB storage and recovery have not been implemented.
- **Only string data:** Lists, sets, hashes, sorted sets, streams, transactions, and other Redis facilities are not yet supported.
- **Partial RESP2 compatibility:** The request parser handles arrays of bulk strings, not inline commands, RESP3, or arbitrary nested input types.
- **Limited Redis command coverage:** Many standard commands, including `PTTL`, are not yet implemented; some edge cases may differ from Redis.
- **Experimental concurrency:** Async networking does not make database execution nonblocking; short synchronous operations still use a shared `std::sync::RwLock`. Multi-key operations are not yet guaranteed to reproduce all Redis atomicity semantics.
- **No operational hardening:** Connection limits, idle/read timeouts, authentication, TLS, configurable listening endpoints, and graceful shutdown are not implemented.
- **Experimental cleanup:** Active expiration is sampled in a separate thread and does not yet enforce a strict per-cycle CPU or lock-holding budget.
- **No standalone client:** The `client` crate is currently a placeholder.

**Do not expose this server to untrusted networks or use it for important data.** Its default loopback binding is intentional.

## Roadmap

- [x] In-memory binary-safe string storage
- [x] RESP2 request parsing and response serialization
- [x] Conditional `SET` with `NX`, `XX`, `EX`, and `PX`
- [x] `EXPIRE`, `PEXPIRE`, `TTL`, and `PERSIST`
- [x] `INCR` and `DECR`
- [x] Lazy expiration and sampled active cleanup
- [x] Tokio-based concurrent TCP connections
- [x] I/O-independent incremental framing with `BytesMut`
- [x] Command, parser, and asynchronous network tests
- [ ] Add connection limits, timeouts, graceful shutdown, and cleanup work budgets
- [ ] Improve Redis compatibility and add further string commands (`PTTL`, `INCRBY`, etc.)
- [ ] Add other Redis data types and commands
- [ ] Implement append-only persistence and restart recovery
- [ ] Build a dedicated Rust client CLI
- [ ] Benchmark throughput, latency, memory, and lock contention
- [ ] Add stress testing, fuzzing, configuration, and automated CI

## Motivation

Redis is a useful reference system for learning about wire protocols, in-memory databases, asynchronous networking, synchronization, expiration algorithms, and persistence. This project develops those ideas incrementally in Rust, prioritizing understandable components and observable behavior over feature completeness.

Redis is a project and trademark of Redis Ltd. This repository is an independent educational implementation and is not affiliated with or endorsed by Redis Ltd.
