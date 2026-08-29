//! User, UserStatus, ProfilePhoto.

use super::*;
use crate::error::{Error, Result};
use crate::serialize::TLReader;
#[allow(unused_imports)]
use std::fmt;

// §7 User types
// ===========================================================================

/// A Telegram user.
// The full-user variant is inherently wide; boxing would complicate every
// match for no real-world win — the enum is built once per response.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum User {
    /// Full user with all fields.
    User {
        id: UserId,
        access_hash: Option<AccessHash>,
        first_name: Option<String>,
        last_name: Option<String>,
        username: Option<String>,
        phone: Option<String>,
        photo: Option<ProfilePhoto>,
        status: Option<UserStatus>,
        bot: bool,
        min: bool,
        scam: bool,
        fake: bool,
    },
    /// Empty user (deleted or unknown).
    Empty {
        id: UserId,
    },
}

impl User {
    pub fn id(&self) -> UserId {
        match self {
            User::User { id, .. } | User::Empty { id } => *id,
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
            User::Empty { .. } => false,
        }
    }

    /// Parse a User from a TL reader.
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            USER => {
                // user#31774388 flags:# ... flags2:# ... id:long ...
                // The flags2 word is ALWAYS serialized (before id) — it
                // keys its own conditional fields.
                let flags = r.read_i32()?;
                let _flags2 = r.read_i32()?;
                let id = UserId(r.read_i64()?);
                let access_hash = if flags & (1 << 0) != 0 {
                    Some(AccessHash(r.read_i64()?))
                } else {
                    None
                };
                let first_name = if flags & (1 << 1) != 0 {
                    Some(String::from_utf8(r.read_bytes()?)?)
                } else {
                    None
                };
                let last_name = if flags & (1 << 2) != 0 {
                    Some(String::from_utf8(r.read_bytes()?)?)
                } else {
                    None
                };
                let username = if flags & (1 << 3) != 0 {
                    Some(String::from_utf8(r.read_bytes()?)?)
                } else {
                    None
                };
                let phone = if flags & (1 << 4) != 0 {
                    Some(String::from_utf8(r.read_bytes()?)?)
                } else {
                    None
                };
                let photo = if flags & (1 << 5) != 0 {
                    Some(ProfilePhoto::read_from(r)?)
                } else {
                    None
                };
                let status = if flags & (1 << 6) != 0 {
                    Some(UserStatus::read_from(r)?)
                } else {
                    None
                };
                let bot = flags & (1 << 14) != 0;
                let min = flags & (1 << 20) != 0;
                let scam = flags & (1 << 24) != 0;
                let fake = flags & (1 << 25) != 0;

                Ok(User::User {
                    id, access_hash, first_name, last_name,
                    username, phone, photo, status,
                    bot, min, scam, fake,
                })
            }
            USER_EMPTY => {
                let id = UserId(r.read_i64()?);
                Ok(User::Empty { id })
            }
            other => Err(Error::Serialization(format!(
                "unknown User constructor {other:#x}"
            ))),
        }
    }
}

/// User online status.
#[derive(Debug, Clone)]
pub enum UserStatus {
    Online { expires: i32 },
    Offline { was_online: i32 },
    Recently,
    LastWeek,
    LastMonth,
    Empty,
}

impl UserStatus {
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            USER_STATUS_OFFLINE => {
                let was_online = r.read_i32()?;
                Ok(UserStatus::Offline { was_online })
            }
            USER_STATUS_ONLINE => {
                let expires = r.read_i32()?;
                Ok(UserStatus::Online { expires })
            }
            USER_STATUS_RECENTLY => Ok(UserStatus::Recently),
            USER_STATUS_LAST_WEEK => Ok(UserStatus::LastWeek),
            USER_STATUS_LAST_MONTH => Ok(UserStatus::LastMonth),
            USER_STATUS_EMPTY => Ok(UserStatus::Empty),
            other => Err(Error::Serialization(format!(
                "unknown UserStatus constructor {other:#x}"
            ))),
        }
    }
}

/// Profile photo.
#[derive(Debug, Clone)]
pub enum ProfilePhoto {
    Photo {
        photo_id: PhotoId,
        dc_id: i32,
    },
    Empty,
}

impl ProfilePhoto {
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            USER_PROFILE_PHOTO | CHAT_PHOTO => {
                let photo_id = PhotoId(r.read_i64()?);
                let _volume_id = r.read_i64()?;
                let _dc_id = r.read_i32()?;
                Ok(ProfilePhoto::Photo { photo_id, dc_id: 0 })
            }
            USER_PROFILE_PHOTO_EMPTY | CHAT_PHOTO_EMPTY => Ok(ProfilePhoto::Empty),
            other => Err(Error::Serialization(format!(
                "unknown ProfilePhoto constructor {other:#x}"
            ))),
        }
    }
}

// ===========================================================================
