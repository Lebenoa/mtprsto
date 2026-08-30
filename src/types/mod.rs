//! TL type library for the Telegram API.
//!
//! Split per domain: one child module per §-section of the original
//! `types.rs`. Everything is re-exported here, so `crate::types::*`
//! resolves exactly as before the split — no public API change.

mod builders;
mod chat;
mod constructors;
mod dialog;
mod file_input;
/// Schema-generated parsers (tools/gentl.py) — namespaced to avoid
/// collisions with the hand-written types below.
pub mod tl_gen;
mod ids;
mod input;
mod message;
mod peer;
mod photo;
mod replies;
mod reply_markup;
mod reply_types;
mod updates;
mod user;

pub use builders::*;
pub use chat::*;
pub use constructors::*;
pub use dialog::*;
pub use file_input::*;
pub use ids::*;
pub use input::*;
pub use message::*;
pub use peer::*;
pub use photo::*;
pub use replies::*;
pub use reply_markup::*;
pub use reply_types::*;
pub use updates::*;
pub use user::*;

#[cfg(test)]
mod tests;

