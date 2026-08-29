//! Chat and Channel variants plus admin/banned rights.

use super::*;
use crate::error::{Error, Result};
use crate::serialize::TLReader;
#[allow(unused_imports)]
use std::fmt;

// §7 Chat / Channel types
// ===========================================================================

/// A Telegram chat (basic group, supergroup, or channel).
#[derive(Debug, Clone)]
pub enum Chat {
    /// Basic group chat.
    Chat {
        id: ChatId,
        title: String,
        photo: Option<ProfilePhoto>,
        participants_count: i32,
        date: i32,
        version: i32,
        creator: bool,
        kicked: bool,
        left: bool,
        deactivated: bool,
    },
    /// Empty/deleted chat.
    Empty { id: ChatId },
    /// `chatForbidden#7328209` — the account cannot view this chat.
    Forbidden { id: ChatId, title: String },
    /// Channel or supergroup.
    Channel {
        id: ChannelId,
        access_hash: Option<AccessHash>,
        title: String,
        username: Option<String>,
        photo: Option<ProfilePhoto>,
        date: i32,
        version: i32,
        megagroup: bool,
        broadcast: bool,
        verified: bool,
        scam: bool,
        fake: bool,
        left: bool,
        signature_names_default: bool,
        admin_rights: Option<ChatAdminRights>,
        banned_rights: Option<ChatBannedRights>,
    },
    /// Forbidden channel.
    ChannelForbidden {
        id: ChannelId,
        access_hash: AccessHash,
        title: String,
        broadcast: bool,
        megagroup: bool,
    },
}

impl Chat {
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            CHAT => {
                let flags = r.read_i32()?;
                let id = ChatId(r.read_i64()?);
                let title = String::from_utf8(r.read_bytes()?)?;
                let photo = if flags & (1 << 3) != 0 {
                    Some(ProfilePhoto::read_from(r)?)
                } else {
                    None
                };
                let participants_count = r.read_i32()?;
                let date = r.read_i32()?;
                let version = r.read_i32()?;
                let creator = flags & (1 << 0) != 0;
                let kicked = flags & (1 << 1) != 0;
                let left = flags & (1 << 2) != 0;
                let deactivated = flags & (1 << 5) != 0;
                Ok(Chat::Chat {
                    id, title, photo, participants_count, date, version,
                    creator, kicked, left, deactivated,
                })
            }
            CHAT_EMPTY => {
                let id = ChatId(r.read_i64()?);
                Ok(Chat::Empty { id })
            }
            CHAT_FORBIDDEN => {
                // chatForbidden#7328209: id:int then title:string
                let id = ChatId(r.read_i64()?);
                let title = String::from_utf8(r.read_bytes()?)?;
                Ok(Chat::Forbidden { id, title })
            }
            CHANNEL => {
                let flags = r.read_i32()?;
                let id = ChannelId(r.read_i64()?);
                let access_hash = if flags & (1 << 0) != 0 {
                    Some(AccessHash(r.read_i64()?))
                } else {
                    None
                };
                let title = String::from_utf8(r.read_bytes()?)?;
                let username = if flags & (1 << 6) != 0 {
                    Some(String::from_utf8(r.read_bytes()?)?)
                } else {
                    None
                };
                let photo = if flags & (1 << 2) != 0 {
                    Some(ProfilePhoto::read_from(r)?)
                } else {
                    None
                };
                let date = r.read_i32()?;
                let version = r.read_i32()?;
                let megagroup = flags & (1 << 8) != 0;
                let broadcast = flags & (1 << 5) != 0;
                let verified = flags & (1 << 7) != 0;
                let scam = flags & (1 << 13) != 0;
                let fake = flags & (1 << 24) != 0;
                let left = flags & (1 << 12) != 0;
                let signature_names_default = flags & (1 << 25) != 0;
                // admin_rights and banned_rights only in recent layers
                Ok(Chat::Channel {
                    id, access_hash, title, username, photo, date, version,
                    megagroup, broadcast, verified, scam, fake, left,
                    signature_names_default,
                    admin_rights: None,
                    banned_rights: None,
                })
            }
            CHANNEL_FORBIDDEN => {
                let flags = r.read_i32()?;
                let id = ChannelId(r.read_i64()?);
                let access_hash = AccessHash(r.read_i64()?);
                let title = String::from_utf8(r.read_bytes()?)?;
                let broadcast = flags & (1 << 5) != 0;
                let megagroup = flags & (1 << 8) != 0;
                Ok(Chat::ChannelForbidden {
                    id, access_hash, title, broadcast, megagroup,
                })
            }
            other => Err(Error::Serialization(format!(
                "unknown Chat constructor {other:#x}"
            ))),
        }
    }
}

/// Channel admin rights.
#[derive(Debug, Clone, Default)]
pub struct ChatAdminRights {
    pub flags: i32,
}

/// Channel banned rights.
#[derive(Debug, Clone, Default)]
pub struct ChatBannedRights {
    pub flags: i32,
    pub until_date: i32,
}

// ===========================================================================
