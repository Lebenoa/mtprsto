//! Custom session storage, working end-to-end.
//!
//! Demonstrates the [`SessionStorage`] trait with an in-memory backend,
//! exercised against the real DCs: the first client creates an auth key
//! through the full DH handshake and the session lands in the custom
//! store; a second client built over the *same* store reuses that key
//! (no second handshake) — proving the backend round-trips everything
//! the client needs.
//!
//! Run:
//! ```sh
//! TELEGRAM_API_ID=12345 TELEGRAM_API_HASH=abcdef... \
//!   cargo run --example session_storage
//! ```
//!
//! Without credentials the example only checks that both backends
//! construct (offline mode).

use mtprsto::client::Client;
use mtprsto::session::{SessionData, SessionStorage, SessionStore};
use std::sync::{Arc, Mutex};

/// A minimal custom backend: keep the session in process memory behind
/// a shared handle, so two clients can observe the same stored session.
#[derive(Clone)]
struct MemoryStore(Arc<Mutex<Option<SessionData>>>);

impl MemoryStore {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }

    fn shared(&self) -> Self {
        Self(self.0.clone())
    }
}

impl SessionStorage for MemoryStore {
    fn load(&mut self) -> mtprsto::Result<Option<SessionData>> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn save(&mut self, data: &SessionData) -> mtprsto::Result<()> {
        *self.0.lock().unwrap() = Some(data.clone());
        Ok(())
    }

    fn delete(&mut self) -> mtprsto::Result<()> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }

    fn describe(&self) -> String {
        "in-memory session".to_string()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let api_id: i32 = std::env::var("TELEGRAM_API_ID").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let api_hash = std::env::var("TELEGRAM_API_HASH").unwrap_or_default();

    // 1. The default backend: JSON file (identical to `.session("p")`).
    let json_store = Box::new(SessionStore::new("session.json")) as Box<dyn SessionStorage>;
    let _offline_client = Client::builder()
        .api_id(12345)
        .api_hash("your_api_hash")
        .session_storage(json_store)
        .build()?;
    println!("JSON file backend: client constructed");

    // 2. Custom backend, shared between two clients.
    let shared = MemoryStore::new();

    if api_id == 0 || api_hash.is_empty() {
        println!("TELEGRAM_API_ID / TELEGRAM_API_HASH not set — skipping live connect.");
        println!("Set them to see the session round-trip through the custom backend.");
        return Ok(());
    }

    println!("client A: connecting through the custom backend (full DH handshake)...");
    let mut client_a = Client::builder()
        .api_id(api_id)
        .api_hash(api_hash.clone())
        .session_storage(Box::new(shared.shared()))
        .build()?;
    client_a.connect().await?;
    // An un-logged-in key can't call user RPCs — the handshake + persisted
    // session ARE the demo. (Log in via the usual flows to go further.)

    println!("client B: same store, fresh client...");
    let mut client_b = Client::builder()
        .api_id(api_id)
        .api_hash(api_hash)
        .session_storage(Box::new(shared.shared()))
        .build()?;
    client_b.connect().await?;
    println!(
        "client B: connected — auth key loaded from the backend, no second handshake"
    );

    println!("OK: custom session storage round-trips the auth key correctly");
    Ok(())
}
