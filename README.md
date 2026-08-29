# mtprsto

Telegram **MTProto 2.0** client library for Rust — a ground-up reimplementation
of the protocol (not a wrapper around a hidden HTTP API), supporting both
**bot** and **user** authorization. Designed as a successor to
[`grammers`](https://github.com/Lonami/grammers) with a builder API, typed
errors, and a pluggable session layer.

## Features

- **Pure MTProto 2.0** — abridged/intermediate framing, obfuscated transport,
  DH key exchange, AES-256-IGE message encryption.
- **High-level `Client`** — builder config, connect, bot login, send messages,
  dialogs, updates, message deletion.
- **SenderPool** — multi-connection pool handling encryption, transport
  framing, decryption, and RPC acks.
- **WebSocket fallback** (optional `ws` feature) — `TransportPolicy::Auto`
  switches new connections to `wss://` after 2 TCP failures on a DC within
  5 minutes, and returns to TCP once it works again. Default is TCP-only;
  enable with `features = ["ws"]`.
- **Pluggable session storage** — persist `auth_key`, server salt, and DC
  behind a small trait. JSON file backend included; implement
  [`SessionStorage`](src/session.rs) for SQLite, Postgres, Redis, or anything
  else (see [`examples/session_storage.rs`](examples/session_storage.rs)).
- **Atomic session writes** — write-temp + fsync + rename, so a crash never
  corrupts a session file.
- **Typed errors** — `FloodWait`, `FileReferenceExpired`, `InvalidCode`, and
  friends instead of stringly-typed failures.

## Status

Work in progress. Transport, auth-key exchange, bot/user authorization,
RPC invocation through the pool, session persistence, and update decoding all
work. File upload/download, callback queries, and channel-admin RPCs are on
the roadmap (see [`SPEC.md`](SPEC.md) for the full gap table).

## Installation

```toml
[dependencies]
mtprsto = { path = "path/to/mtprsto" }  # not yet on crates.io
tokio = { version = "1", features = ["full"] }
```

## Getting started

You need an **API ID** and **API hash** from <https://my.telegram.org>
(under *API development tools*).

### Send a message as a bot

```rust,no_run
use mtprsto::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Client::builder()
        .api_id(12345)                     // from my.telegram.org
        .api_hash("your_api_hash")
        .session("bot.session")            // JSON file; auth key survives restarts
        .build()?;

    // First run performs the DH handshake and bot login automatically.
    client.connect().await?;
    client.authorize_bot("123456:ABC-DEF_your_bot_token").await?;

    // Peer can be a user ID, chat ID, channel ID, or @username.
    let msg = client.send("durov", "Hello from mtprsto!").await?;
    println!("sent message id {msg:?}");
    Ok(())
}
```

### Custom session storage

Any backend implementing `SessionStorage` can replace the JSON file:

```rust
use mtprsto::client::Client;
use mtprsto::session::{SessionData, SessionStorage, SessionStore};
use mtprsto::error::Result;
use std::path::PathBuf;

/// Example: store sessions inside a single SQLite database.
/// (Sketch — wire the SQL calls of your choice.)
struct SqliteStore { db_path: PathBuf }

impl SessionStorage for SqliteStore {
    fn load(&mut self) -> Result<Option<SessionData>> { /* SELECT ... */ Ok(None) }
    fn save(&mut self, data: &SessionData) -> Result<()> { /* INSERT OR REPLACE */ Ok(()) }
    fn delete(&mut self) -> Result<()> { /* DELETE */ Ok(()) }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::builder()
        .api_id(12345)
        .api_hash("your_api_hash")
        .session_storage(Box::new(SqliteStore { db_path: "sessions.db".into() }))
        .build()?;
    Ok(())
}
```

Full runnable example — including an in-memory store and the
`spawn_blocking`/`block_on` pattern for async database drivers:
[`examples/session_storage.rs`](examples/session_storage.rs).

Run it:

```sh
cargo run --example session_storage
```

### Demo and smoke tests

The repository ships an example that exercises the stack directly:

```sh
# Offline crypto and TL round-trip checks (no network)
cargo run --example demo -- --demo

# Authorize as a bot against the real DCs
TELEGRAM_API_ID=12345 TELEGRAM_API_HASH=abcdef... cargo run --example demo -- --bot-token 123456:TOKEN

# User authorization (interactive code entry)
TELEGRAM_API_ID=12345 TELEGRAM_API_HASH=abcdef... cargo run --example demo -- --user-phone +15551234567
```

## Architecture

```
┌────────────────────────────────────────────────┐
│ Client (builder, high-level RPC helpers)       │
├────────────────────────────────────────────────┤
│ SenderPool (multi-connection, acks, retries)   │
├───────────────────────────┬────────────────────┤
│ MtProtoSession            │ SessionStorage     │
│ (msg ids, salts,          │ (JSON file impl +  │
│  AES-IGE encryption)      │  custom backends)  │
├───────────────────────────┴────────────────────┤
│ transport (TCP, abridged/intermediate framing) │
├────────────────────────────────────────────────┤
│ crypto (AES-IGE, SHA-1/256, DH, RSA)           │
└────────────────────────────────────────────────┘
```

| Module | Purpose |
|---|---|
| [`client`](src/client.rs) | High-level `Client`: connect, bot login, `send`, `get_me`, `get_dialogs`, `delete_messages`, raw invoke |
| [`pool`](src/pool.rs) | `SenderPool`: pooled connections, RPC correlation, acks |
| [`mtproto`](src/mtproto.rs) | MTProto 2.0 session: message IDs, salts, seq numbers, encryption |
| [`crypto`](src/crypto.rs) | AES-256-IGE, RSA, Diffie–Hellman, SHA-1/SHA-256, MD5 |
| [`transport`](src/transport.rs) | TCP transport with abridged/intermediate framing |
| [`serialize`](src/serialize.rs) | TL reader/writer (little-endian TL primitives) |
| [`session`](src/session.rs) | `SessionData` + `SessionStorage` trait + JSON store |
| [`api`](src/api.rs) | Auth-key creation and authorization flows |
| [`types`](src/types.rs) | Generated TL types (constructors, enums) |
| [`updates`](src/updates.rs) | Update dispatching (`UpdateDispatcher`) |
| [`error`](src/error.rs) | Typed error taxonomy incl. `FloodWait`, `FileReferenceExpired` |
| [`rpc`](src/rpc.rs) | Response decoding helpers |
| [`ws`](src/ws.rs) | Obfuscated2-over-WebSocket fallback (feature `ws`) |

## Session storage

The session holds the 256-byte auth key (base64), server salt, session ID,
DC ID, and user ID. Format:

```json
{
  "auth_key": "<base64>",
  "server_salt": 123456789,
  "session_id": 987654321,
  "server_time_offset": 0,
  "dc_id": 2,
  "user_id": 12345678,
  "api_layer": 175,
  "version": 1
}
```

Saves are atomic (`*.tmp<PID>` + fsync + rename). By default the client
persists to `~/.mtprsto/session.json` when no path is given, so the auth key
is reused across runs. Pass `.session("path.json")` or a custom
`.session_storage(...)` to override.

## Testing

```sh
cargo test                          # unit + doc tests
cargo run --example demo -- --demo  # offline protocol self-check
```

## License

Private project — all rights reserved. (Adjust before publishing.)
