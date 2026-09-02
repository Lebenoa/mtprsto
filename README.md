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
  `send_file` (documents and compressed photos), message builder
  (`.reply_to().silent().no_webpage()`), file builder (`.as_photo()
  .caption().reply_to().silent()`), history iterator, dialogs, updates
  pump, callback queries, channel admin, `forward_messages`, `set_typing`,
  `pin_message`, `join_channel` / `import_chat_invite`, file
  upload/download, raw invoke, and typed `invoke::<T>()` decoding.
- **SenderPool** — multi-connection pool (1 main + aux, adaptive scaling),
  batched acks (≥16 pending or 10 s), ping/pong keepalive with silent-
  disconnect reconnect, transparent bad-server-salt retry, periodic salt
  refresh, `bad_msg_notification` / `rpc_answer_*` classification, and
  **bounded network reads** (30 s timeout → reconnect + re-send) so a
  blackholed connection can never wedge the client.
- **Bot-API-style `-100` channel ids** — `send`, `send_file`, `delete_history`,
  and the whole peer-resolution surface accept `-100…` strings; the access
  hash is resolved from the persisted session cache or a `channels.getChannels`
  bootstrap and cached automatically.
- **Peer-safe fluent builders** — `client.message(...)`, `send_file(...)`,
  and `messages(...)` return `Result` builders: an unresolvable peer
  surfaces as an `Err` through `?` instead of a panic.
- **Photo ergonomics** — `Photo::largest_size()` / `Photo::location()`
  produce a ready-to-download `FileLocation`, and `Message::photo()`
  pulls the photo out of a message in one call.
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

## Branches & API layers

All branches share one wire dialect and one codebase. `docs-layer` is the
development trunk; the other branches are stable snapshots of it:

| Branch | `API_LAYER` | Role |
|---|---|---|
| `docs-layer` | 223 | development trunk; all wire fixes and the live sweep land here first |
| `main` | 223 | release snapshot — tracks docs-layer; adopts a newer published layer only after a live-sweep pass proves it |
| `dev-layer` | 225 | playground snapshot — adopts a newer published layer first; a dialect change is promoted to docs-layer only after live verification |

Layer adoption path: the [layer changelog](https://core.telegram.org/api/layers)
publishes an entry per layer, and **layer 225 is the current documentation
release** (224 has an entry too, and most of the ctor re-issues this codebase
aliases happened there). Layers beyond the latest changelog entry exist only
as drafts (tdlib master) — the retired per-branch dialect forks targeted one
of those drafts, and the undocumented drift caused real parse bugs.

Note that core.telegram.org's `/schema` endpoints ignore `?layer=` and serve
the last fully published dialect (223 today). The published schema for a
documented layer is assembled instead by applying the per-layer diff dumps
from the changelog page:

```sh
python tools/update_schema.py 225            # scrape base + diffs, verify
python tools/update_schema.py --audit-aliases
```

Then regenerate the parsers
(`python tools/gentl.py tools/schema_l225.tl --domain <name> --compat --out …`,
`--all` for `gen_fns.rs`), set `API_LAYER`, reconcile the curated constants
(`gentl --diff`), and run the live sweep before promoting. `CTOR_ALIASES` in
`tools/gentl.py` stays minimal by design: an alias is kept only when the old
id's shape is layout-compatible with the schema ctor it maps to, or when it
is a wire-verified divergence (production DCs answer with the tdlib-draft
`channel#d49f34c6` regardless of the negotiated layer);
`python tools/update_schema.py --audit-aliases` re-checks that invariant.

## Installation

```toml
[dependencies]
mtprsto = { git = "https://github.com/Lebenoa/mtprsto", branch = "master" }
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
    let client = Client::builder()
        .api_id(12345)                     // from my.telegram.org
        .api_hash("your_api_hash")
        .session("bot.session")            // JSON file; auth key survives restarts
        .build()?;

    // First run performs the DH handshake and bot login automatically.
    client.connect().await?;
    client.authorize_bot("123456:ABC-DEF_your_bot_token").await?;

    // Peer can be a user ID, chat ID, @username, or a Bot-API-style
    // -100… channel id (access hash resolved/persisted automatically).
    let msg = client.send("durov", "Hello from mtprsto!").await?;
    println!("sent message id {msg:?}");
    Ok(())
}
```

### Custom session storage

Any backend implementing `SessionStorage` can replace the JSON file. The
trait has four methods — `load`, `save`, `delete`, `describe` — and the
client drives them on every connect, auth-key change, and salt refresh:

```rust
use mtprsto::client::Client;
use mtprsto::session::{SessionData, SessionStorage};
use std::path::PathBuf;

/// Example: store sessions inside a single SQLite database.
/// (Sketch — wire the SQL calls of your choice.)
struct SqliteStore { db_path: PathBuf }

impl SessionStorage for SqliteStore {
    fn load(&mut self) -> mtprsto::Result<Option<SessionData>> { /* SELECT */ Ok(None) }
    fn save(&mut self, data: &SessionData) -> mtprsto::Result<()> { /* UPSERT */ Ok(()) }
    fn delete(&mut self) -> mtprsto::Result<()> { /* DELETE */ Ok(()) }
    fn describe(&self) -> String { "sqlite session".into() }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::builder()
        .api_id(12345)
        .api_hash("your_api_hash")
        .session_storage(Box::new(SqliteStore { db_path: "sessions.db".into() }))
        .build()?;
    client.connect().await?;
    Ok(())
}
```

A fully **working** example — an in-memory store shared by two clients,
proving the auth key round-trips without a second handshake — plus the
`spawn_blocking`/`block_on` pattern for async database drivers:
[`examples/session_storage.rs`](examples/session_storage.rs).

Run it (needs credentials to connect; offline it just checks construction):

```sh
TELEGRAM_API_ID=12345 TELEGRAM_API_HASH=abcdef... cargo run --example session_storage
```

### Advanced examples

| Example | What it shows |
|---|---|
| [`advanced_rpc.rs`](examples/advanced_rpc.rs) | raw `invoke_raw`, `invokeAfterMsg` ordering, `invokeWithoutUpdates`, `getFutureSalts`, transient-retry loops |
| [`callback_buttons.rs`](examples/callback_buttons.rs) | reading inline keyboards off messages and pressing buttons via `getBotCallbackAnswer` (user session, auto-loads the demo login) |
| [`channel_admin.rs`](examples/channel_admin.rs) | channel create / re-fetch / invite / `editAdmin` rights / participants / leave (user session, auto-loads the demo login) |
| [`list_peers.rs`](examples/list_peers.rs) | listing dialogs: users and channels printed with ready-to-use `-100` send ids (user session) |
| [`send_to_channel.rs`](examples/send_to_channel.rs) | sending text/files to a channel addressed by its Bot-API `-100` id, with hash auto-resolution and a confirm gate (user session) |
| [`file_transfer.rs`](examples/file_transfer.rs) | parallel upload workers and parallel range downloads (`DownloadConfig`) |
| [`updates_listener.rs`](examples/updates_listener.rs) | the update pump: pts/seq/qts gap recovery, `UpdateChannelTooLong` resync |
| [`session_storage.rs`](examples/session_storage.rs) | custom `SessionStorage` backends (SQLite-style, in-memory) |

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

### Tuning protocol behavior

Keepalive, ack batching, salt refresh, and anti-fingerprinting padding are
configurable (defaults follow gotd/Telegram-Desktop practice — random
padding is **on** by default so message lengths aren't fingerprintable):

```rust
use mtprsto::pool::ProtocolConfig;

let mut protocol = ProtocolConfig::default();
protocol.ping_interval = std::time::Duration::from_secs(60);
protocol.compress_threshold = 1024; // gzip_packed for larger payloads (0 = off)
protocol.random_padding = false; // only if you really need deterministic sizes

let client = Client::builder()
    .api_id(12345)
    .api_hash("abcdef...")
    .protocol_config(protocol)
    .build()?;
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
| [`client`](src/client.rs) | High-level `Client`: connect, bot login, `send`, `send_to_peer`, `get_me`, `get_dialogs`, `delete_messages`, raw invoke |
| [`client_wrappers`](src/client_wrappers.rs) | Typed RPC wrappers: channel admin, profile photos, multi-media, callback answers, history cleanup |
| [`pool`](src/pool.rs) | `SenderPool`: pooled connections, RPC correlation, batched acks, keepalive, bounded reads |
| [`mtproto`](src/mtproto.rs) | MTProto 2.0 session: message IDs, salts, seq numbers, encryption |
| [`crypto`](src/crypto.rs) | AES-256-IGE, RSA, Diffie–Hellman, SRP (2FA), SHA-1/SHA-256, MD5 |
| [`transport`](src/transport.rs) | TCP transport with abridged/intermediate framing |
| [`serialize`](src/serialize.rs) | TL reader/writer (little-endian TL primitives) |
| [`session`](src/session.rs) | `SessionData` + `SessionStorage` trait + JSON store |
| [`api`](src/api.rs) | Auth-key creation and authorization flows |
| [`file`](src/file.rs) | Chunked upload/download, `DownloadConfig`, `upload.file` parsing |
| [`types`](src/types/) | TL types: generated unions + builders (`tools/gentl.py` from the assembled `tools/schema_l*.tl` — see `tools/update_schema.py`) plus curated models and dialect alias arms |
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
  "api_layer": 225,
  "peer_cache": { "1234567890": 987654321098765432 },
  "version": 1
}
```

`api_layer` records the dialect the session was created under (branch-dependent
— see [Branches & API layers](#branches--api-layers)).

`peer_cache` stores resolved peer access hashes (channel/user id → hash) so
admin ops don't pay a `channels.getChannels` round trip after restart — and
it's what makes Bot-API-style `-100…` id strings resolvable: `resolve_peer`
looks the hash up here, falling back to a `channels.getChannels` bootstrap
when missing.

Saves are atomic (`*.tmp<PID>` + fsync + rename). By default the client
persists to `~/.mtprsto/session.json` when no path is given, so the auth key
is reused across runs. Pass `.session("path.json")` or a custom
`.session_storage(...)` to override.

## Testing

```sh
cargo test                          # unit + doc tests
cargo run --example demo -- --demo  # offline protocol self-check
```
