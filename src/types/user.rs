//! Accessor helpers on the GENERATED user types (schema parsing lives
//! in `user_gen.rs`).

use super::ids::{AccessHash, UserId};
use super::user_gen::User;

impl User {
    /// Identity shortcut: works across `User`/`Empty` variants.
    #[must_use]
    pub const fn id(&self) -> UserId {
        match self {
            Self::User { id, .. } | Self::Empty { id } => *id,
        }
    }

    #[must_use]
    pub const fn access_hash(&self) -> Option<AccessHash> {
        match self {
            Self::User { access_hash, .. } => *access_hash,
            Self::Empty { .. } => None,
        }
    }

    #[must_use]
    pub fn username(&self) -> Option<&str> {
        match self {
            Self::User { username, .. } => username.as_deref(),
            Self::Empty { .. } => None,
        }
    }

    #[must_use]
    pub fn first_name(&self) -> Option<&str> {
        match self {
            Self::User { first_name, .. } => first_name.as_deref(),
            Self::Empty { .. } => None,
        }
    }

    /// Display name: first and last name joined by a space, either half
    /// optional (grammers' `full_name` semantics).
    #[must_use]
    pub fn full_name(&self) -> String {
        let first = self.first_name().unwrap_or("");
        let last = match self {
            Self::User { last_name, .. } => last_name.as_deref().unwrap_or(""),
            Self::Empty { .. } => "",
        };
        match (first, last) {
            ("", "") => String::new(),
            (first, "") => first.to_string(),
            ("", last) => last.to_string(),
            (first, last) => format!("{first} {last}"),
        }
    }

    #[must_use]
    pub fn phone(&self) -> Option<&str> {
        match self {
            Self::User { phone, .. } => phone.as_deref(),
            Self::Empty { .. } => None,
        }
    }

    #[must_use]
    pub const fn is_bot(&self) -> bool {
        match self {
            Self::User { bot, .. } => *bot,
            Self::Empty { .. } => false,
        }
    }
}
