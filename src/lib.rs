//! # mtprsto — Telegram MTProto 2.0 for Rust
//!
//! A high-performance MTProto 2.0 client library supporting both
//! user and bot authorization. Designed as a successor to grammers
//! with better DX, typed errors, and a builder API.
//!
//! ## Quick start
//!
//! ```no_run
//! use mtprsto::client::Client;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut client = Client::builder()
//!     .api_id(12345)
//!     .api_hash("your_api_hash")
//!     .session("session.json")
//!     .build()?;
//!
//! // Bot authorization
//! client.connect().await?;
//! client.authorize_bot("123456:ABC-DEF...").await?;
//!
//! // Send a message
//! let msg = client.send("user_id_or_username", "Hello!").await?;
//! println!("Sent message ID: {}", msg);
//! # Ok(())
//! }
//! ```

pub mod api;
pub mod client;
pub mod crypto;
pub mod error;
pub mod mtproto;
pub mod pool;
pub mod rpc;
pub mod serialize;
pub mod session;
pub mod transport;
pub mod types;
pub mod updates;

// Re-exports for convenience
pub use error::{Error, Result};
pub use types::{
    UserId, ChatId, ChannelId, AccessHash, MsgId,
    InputPeer, InputUser, InputChannel,
};
pub use client::Client;
pub use session::SessionStore;
pub use updates::{UpdateDispatcher, DispatchMode};
