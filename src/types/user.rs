//! Accessor helpers on the GENERATED user types (schema parsing lives
//! in `user_gen.rs`).

use super::ids::{AccessHash, UserId};
use super::user_gen::User;

impl User {
    /// Identity shortcut: works across `User`/`Empty` variants.
    pub fn id(&self) -> UserId {
        match self {
            User::User { id, .. } => *id,
            User::Empty { id } => *id,
        }
    }

    pub fn access_hash(&self) -> Option<AccessHash> {
        match self {
            User::User { access_hash, .. } => *access_hash,
            User::Empty { .. } => None,
        }
    }

    pub fn username(&self) -> Option<&str> {
        match self {
            User::User { username, .. } => username.as_deref(),
            User::Empty { .. } => None,
        }
    }

    pub fn first_name(&self) -> Option<&str> {
        match self {
            User::User { first_name, .. } => first_name.as_deref(),
            User::Empty { .. } => None,
        }
    }

    pub fn phone(&self) -> Option<&str> {
        match self {
            User::User { phone, .. } => phone.as_deref(),
            User::Empty { .. } => None,
        }
    }

    pub fn is_bot(&self) -> bool {
        match self {
            User::User { bot, .. } => *bot,
            _ => false,
        }
    }
}
