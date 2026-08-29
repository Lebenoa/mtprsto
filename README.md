# mtprsto

Telegram **MTProto 2.0** client library for Rust — a ground-up reimplementation
of the protocol (not a wrapper around a hidden HTTP API), supporting both
**bot** and **user** authorization. Designed as a successor to
[`grammers`](https://github.com/Lonami/grammers) with a builder API, typed
errors, and a pluggable session layer.

## Features

- **Pure MTProto 2.0** — abridged/intermediate framing, obfuscated transport,
  DH key exchange, AES-256-IGE message encryption.
- **High-level `Client`** — builder config, bot/user login, `send`,
  `send_file`, message builder (`.reply_to().silent()`), history iterator,
  dialogs, updates pump, callback queries, channel admin, file
  upload/download, raw invoke.
- **SenderPool** — multi-connection pool (1 main + aux, adaptive scaling),
  batched acks (≥16 pending or 10 s), ping/pong keepalive with silent-
  disconnect reconnect, transparent bad-server-salt retry, periodic salt
  refresh, and `bad_msg_notification` / `rpc_answer_*` classification.
- **WebSocket fallback** (optional `ws` feature) — `TransportPolicy::Auto`
  switches new connections to `wss://` after 2 TCP failures on a DC within
  5 minutes, and returns to TCP once it works again. Default is TCP-only;
  enable with `features = ["ws"]`.
- **Files** — chunked `upload.saveFilePart`/`saveBigFilePart` with parallel
  workers; `upload.getFile` downloads with optional parallel range fetching
  (`DownloadConfig`, BS-5) and CDN-redirect detection.
- **Nearest-DC selection** — bootstraps on DC 2, asks `help.getNearestDc`
  and re-handshakes to the closest DC before authorizing. Pin a DC with
  `.dc_id(n)` to opt out (test DCs, pinned deployments). `USER_MIGRATE_X`
  migrations during bot login are followed automatically.
- **Updates** — `UpdateDispatcher` with pts/seq/qts tracking, gap recovery
  via `updates.getDifference`, automatic channel resync on
  `UpdateChannelTooLong`, and `mpsc` channel or handler dispatch.
- **Pluggable session storage** — persist `auth_key`, server salt, DC, and
  peer access hashes behind a small trait. JSON file backend included;
  implement [`SessionStorage`](src/session.rs) for SQLite, Postgres, Redis,
  or anything else (see [`examples/session_storage.rs`](examples/session_storage.rs)).
- **Atomic session writes** — write-temp + fsync + rename, so a crash never
  corrupts a session file.
- **Typed errors** — `FloodWait`, `FileReferenceExpired`, `BadMessage`,
  `RpcDropped`, `InvalidCode`, and friends instead of stringly-typed
  failures.
- **2FA (SRP)** and QR login token flows included.

## Status

Protocol-complete for the ii-drive migration surface (see
[`SPEC.md`](SPEC.md)). Remaining backlog: CDN file download (redirects are
detected but not fetched), IPv6 DC table, and throughput benchmarks vs
grammers.

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
| [`pool`](src/pool.rs) | `SenderPool`: pooled connections, RPC correlation, batched acks, keepalive |
| [`mtproto`](src/mtproto.rs) | MTProto 2.0 session: message IDs, salts, seq numbers, encryption |
| [`crypto`](src/crypto.rs) | AES-256-IGE, RSA, Diffie–Hellman, SRP (2FA), SHA-1/SHA-256, MD5 |
| [`transport`](src/transport.rs) | TCP transport with abridged/intermediate framing |
| [`serialize`](src/serialize.rs) | TL reader/writer (little-endian TL primitives) |
| [`session`](src/session.rs) | `SessionData` + `SessionStorage` trait + JSON store |
| [`api`](src/api.rs) | Auth-key creation and authorization flows |
| [`file`](src/file.rs) | Chunked upload/download, `DownloadConfig`, `upload.file` parsing |
| [`types`](src/types/) | Hand-written TL types (constructors, enums) |
| [`updates`](src/updates.rs) | Update dispatching (`UpdateDispatcher`) |
| [`ergonomics`](src/ergonomics.rs) | Message/send-file builders, history iterator |
| [`resilience`](src/resilience.rs) | Flood limiter, file-ref cache, DC rotator |
| [`error`](src/error.rs) | Typed error taxonomy incl. `FloodWait`, `BadMessage` |
| [`rpc`](src/rpc.rs) | TL payload builders for the §7 RPC surface |
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
  "api_layer": 223,
  "peer_cache": { "12345678": -1001234567890 },
  "version": 1
}
```

`peer_cache` stores resolved peer access hashes (channel/user id → hash) so
admin ops don't pay a `channels.getChannels` round trip after restart.

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
