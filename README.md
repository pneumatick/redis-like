# Redis-like

A work-in-progress, Redis-inspired in-memory key–value server written in **Rust**. The project recreates a subset of Redis's commands and behavior while exploring TCP networking, the Redis Serialization Protocol (RESP2), concurrent access, string encodings, and key expiration.

This is a learning and systems-programming project, **not a production-ready Redis replacement**. The emphasis is on building the core functionality from the ground up rather than wrapping an existing Redis server.

## Features

- **RESP2-compatible command framing:** accepts arrays of bulk strings over TCP and encodes RESP2 replies.
- **Concurrent connections:** serves multiple clients using threads and an `Arc<RwLock<Database>>` shared database.
- **Binary-safe keys and string values:** stores keys and values as byte sequences; string values may also use an internal integer representation after arithmetic operations.
- **Conditional writes:** `SET` supports `NX` and `XX`, with optional `EX` and `PX` expiration durations.
- **Key expiration:** supports lazy expiration on access and active cleanup using randomized sampling of keys with deadlines.
- **Integer operations:** includes `INCR` and `DECR` with checked arithmetic on existing numeric strings.
- **Tests:** includes protocol-level tests and command tests for `SET` options, expiration behavior, and concurrent conditional writes.

## Getting started

### Requirements

- Rust and Cargo, with support for the **Rust 2024 edition** (Rust 1.85 or newer).
- Optional: [`redis-cli`](https://redis.io/docs/latest/develop/tools/cli/) to interact with the server. A standalone client is not yet implemented in this workspace.

### Build and run

```bash
git clone https://github.com/pneumatick/redis-like.git
cd redis-like
cargo run -p server
```

The server currently listens on **`127.0.0.1:43210`**. The address and port are set in `server/src/main.rs` and are not yet configurable through command-line options.

In another terminal, connect using `redis-cli` in RESP2 mode:

```bash
redis-cli -2 -h 127.0.0.1 -p 43210
```

Try a few commands:

```redis
PING
SET greeting hello
GET greeting
SET session token EX 60 NX
TTL session
SET counter 1
INCR counter
DECR counter
PERSIST session
DEL greeting counter session
```

You can also run individual commands without entering interactive mode:

```bash
redis-cli -2 -h 127.0.0.1 -p 43210 SET message hello
redis-cli -2 -h 127.0.0.1 -p 43210 GET message
```

Only the commands listed below are implemented; some additional Redis commands issued automatically by particular clients may receive an unknown-command error.

## Supported commands

| Command | Description |
| --- | --- |
| `PING [message]` | Test connectivity; optionally echo a message. |
| `SET key value [EX seconds \| PX milliseconds] [NX \| XX]` | Create or overwrite a string, optionally with an expiration or existence condition. |
| `GET key` | Retrieve a string value; return null if the key is missing or expired. |
| `DEL key [key ...]` | Delete one or more keys. |
| `EXPIRE key seconds` | Set a key's expiration in seconds. |
| `PEXPIRE key milliseconds` | Set a key's expiration in milliseconds. |
| `TTL key` | Get the remaining lifetime in seconds (`-1` for persistent keys, `-2` for missing or expired keys). |
| `PERSIST key` | Remove a key's expiration. |
| `INCR key` | Increment an integer-valued string. |
| `DECR key` | Decrement an integer-valued string. |

`SET` accepts the option order supported by the current parser, such as `SET key value NX EX 60` and `SET key value EX 60 NX`. Conflicting or repeated options are rejected.

This is a **subset of Redis**, not a complete implementation of Redis's protocol, data types, or command semantics. See [Current limitations](#current-limitations).

## Architecture

The repository is a Cargo workspace with three crates:

```text
redis-like/
├── Cargo.toml          # Workspace definition
├── common/
│   └── src/
│       ├── lib.rs
│       └── resp.rs     # RESP2 request decoder and response encoder
├── server/
│   └── src/
│       ├── main.rs     # TCP listener, connection threads, cleanup worker
│       ├── command.rs  # Command parsing, validation, and execution
│       └── db.rs       # In-memory data model and database operations
└── client/
    └── src/
        └── main.rs     # Placeholder for a future client
```

### Request flow

1. The server accepts a TCP connection and starts a handler thread.
2. The shared RESP2 decoder reads a complete command from the byte stream.
3. The command layer validates arguments and dispatches the requested operation.
4. The command handler acquires the appropriate database lock and executes the operation.
5. The result is serialized as a RESP2 response and sent back to the client.

The server can process several commands over one connection. Database operations are protected by an `RwLock`, allowing multiple concurrent readers or one writer at a time. Read–modify–write operations such as conditional `SET` and `INCR` use exclusive access.

### Storage and expiration

Stored entries and expiration deadlines are maintained in separate maps:

- `HashMap<Vec<u8>, Entry>` stores keys and their values.
- `IndexMap<Vec<u8>, SystemTime>` stores deadlines **only for keys with an expiration**.

The value model distinguishes Redis string values from their internal encoding: string values can be held as raw bytes or as a signed integer. This leaves room for adding other Redis data types later.

Expiration is managed in two complementary ways:

- **Lazy expiration:** an expired key is treated as absent when it is accessed, with removal performed when appropriate.
- **Active expiration:** a dedicated cleanup thread runs approximately ten times per second, selecting small random samples from the expiration index and deleting expired keys. The current sampling and scheduling strategy is experimental and may be refined.

Because the expiration index contains only expiring keys, background cleanup need not scan every permanent key in the database.

## Testing

Run the full workspace test suite:

```bash
cargo test --workspace
```

Or run the server and protocol tests separately:

```bash
cargo test -p server
cargo test -p common
```

Current tests cover RESP2 framing and replies, `SET` options and their error cases, TTL behavior, and concurrent `SET NX` attempts. Additional end-to-end, fuzz, and performance tests are planned.

## Current limitations

- **In-memory only:** all stored data is lost when the server exits; persistence is not implemented.
- **Limited command and type support:** lists, sets, hashes, transactions, replication, and clustering are not implemented.
- **Partial RESP2 support:** command decoding expects arrays of bulk strings; inline requests and general-purpose decoding of every RESP2 type are not provided.
- **Experimental correctness:** command edge cases, including arithmetic on nonexistent keys, still need further compatibility testing against Redis.
- **Fixed local configuration:** the server listens on `127.0.0.1:43210` without authentication or configuration support.
- **Thread-per-connection model:** the implementation favors clarity over high-connection-count scalability.
- **No dedicated CLI yet:** `client` remains a placeholder; use a compatible Redis client for now.

Do not expose this server to untrusted networks or use it to store important data.

## Roadmap

- [x] Basic in-memory string storage
- [x] RESP2 request decoding and response encoding
- [x] Concurrent TCP client handling
- [x] Conditional `SET` with optional expiration
- [x] `EXPIRE`, `PEXPIRE`, `TTL`, and `PERSIST`
- [x] Lazy expiration and periodic active cleanup
- [x] Initial `INCR` and `DECR` support
- [ ] Harden edge cases and expand command/integration tests
- [ ] Improve active-expiration sampling and cleanup time budgets
- [ ] Add `PTTL`, additional string operations, and more Redis data types
- [ ] Implement persistence and restart recovery
- [ ] Build the client CLI
- [ ] Add configuration, benchmarks, and stress testing

## Why this project?

Redis provides an interesting reference point for exploring real systems concerns: byte-stream protocol parsing, data representation, concurrency, atomic operations, expiration, and persistence. This project aims to implement those ideas incrementally in Rust, with correctness and clear separation of responsibilities as primary goals.

Redis is a project and trademark of Redis Ltd. This repository is an independent educational implementation and is not affiliated with or endorsed by Redis Ltd.
