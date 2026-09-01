//! Newtype ID wrappers (`UserId`, `ChatId`, `ChannelId`, `AccessHash`,
//! `MsgId`, ...).

use std::fmt;

// §5. Newtype ID wrappers (DX-5 from spec §12.1)
// ===========================================================================

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(pub i64);

        impl From<i64> for $name {
            fn from(v: i64) -> Self { Self(v) }
        }

        impl From<$name> for i64 {
            fn from(v: $name) -> Self { v.0 }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

define_id!(
    /// Telegram user ID. Distinct from `ChannelId` even though both wrap `i64`.
    UserId
);
define_id!(
    /// Telegram chat (group) ID.
    ChatId
);
define_id!(
    /// Telegram channel/supergroup ID.
    ChannelId
);
define_id!(
    /// Access hash — required to interact with users/channels the client doesn't share a chat with.
    AccessHash
);
define_id!(
    /// Message ID — uniquely identifies a message within a chat.
    MsgId
);
define_id!(
    /// Photo ID.
    PhotoId
);
define_id!(
    /// Document/file ID.
    DocumentId
);
define_id!(
    /// File reference — used to authorize file downloads; expires over time.
    FileRef
);

// ===========================================================================
