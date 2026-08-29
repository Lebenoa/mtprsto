//! Session storage backends: trait definition and example implementations
//! the user can follow to plug in their own (SQLite, Postgres, SurrealDB, ...).
//!
//! Run with: `cargo run --example session_storage`

use mtprsto::client::Client;
use mtprsto::error::Result;
use mtprsto::session::{SessionData, SessionStorage, SessionStore};
use std::sync::Mutex;

/// A minimal custom backend: keep the session in process memory.
///
/// Shows the required shape of every `SessionStorage` implementation.
struct MemoryStore(Mutex<Option<SessionData>>);

impl MemoryStore {
    fn new() -> Self {
        Self(Mutex::new(None))
    }
}

impl SessionStorage for MemoryStore {
    fn load(&mut self) -> Result<Option<SessionData>> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn save(&mut self, data: &SessionData) -> Result<()> {
        *self.0.lock().unwrap() = Some(data.clone());
        Ok(())
    }

    fn delete(&mut self) -> Result<()> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }

    fn describe(&self) -> String {
        "in-memory session".to_string()
    }
}

/// Skeleton showing where an async database backend (Postgres, SurrealDB, ...)
/// would integrate. The trait is synchronous, so blocking drivers belong in
/// `tokio::task::spawn_blocking`; async drivers can use `tokio::runtime::Handle`
/// to drive their future to completion, as sketched below.
///
/// ```ignore
/// struct PostgresStore {
///     handle: tokio::runtime::Handle,
///     url: String,
/// }
///
/// impl SessionStorage for PostgresStore {
///     fn load(&mut self) -> Result<Option<SessionData>> {
///         self.handle.block_on(async {
///             // let row: Option<SessionRow> = sqlx::query_as(...).fetch_optional(&pool).await?;
///             // Ok(row.map(SessionData::from))
///             todo!()
///         })
///     }
///     // save / delete follow the same block_on pattern
/// #   fn save(&mut self, _: &SessionData) -> Result<()> { todo!() }
/// #   fn delete(&mut self) -> Result<()> { todo!() }
/// }
/// ```
fn main() -> Result<()> {
    // 1. Default backend: JSON file (identical to `.session("path.json")`).
    let json_store = Box::new(SessionStore::new("session.json")) as Box<dyn SessionStorage>;

    let _client = Client::builder()
        .api_id(12345)
        .api_hash("your_api_hash")
        .session_storage(json_store)
        .build()?;

    // 2. Custom backend: anything implementing SessionStorage.
    let _client2 = Client::builder()
        .api_id(12345)
        .api_hash("your_api_hash")
        .session_storage(Box::new(MemoryStore::new()))
        .build()?;

    println!("both backends constructed; call .connect().await to use them");
    Ok(())
}
