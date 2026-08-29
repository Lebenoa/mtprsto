//! Input peers/users/channels — references used in API calls.

use super::*;
use crate::error::{Error, Result};
use crate::serialize::{TLWriter, TLReader};
#[allow(unused_imports)]
use std::fmt;

// §7 Input types (peer/user/channel references for API calls)
// ===========================================================================

/// A reference to a chat participant that can be used in API calls.
#[derive(Debug, Clone)]
pub enum InputPeer {
    /// Reference to a user, requires access_hash.
    User { user_id: UserId, access_hash: AccessHash },
    /// Reference to a chat (basic group).
    Chat { chat_id: ChatId },
    /// Reference to a channel/supergroup, requires access_hash.
    Channel { channel_id: ChannelId, access_hash: AccessHash },
    /// Reference to yourself.
    Self_,

    /// Legacy: user by ID, resolved server-side.
    UserFromId { user_id: UserId },
}

impl InputPeer {
    /// TL-serialize this InputPeer.
    pub fn write_to(&self, w: &mut TLWriter) {
        match self {
            InputPeer::User { user_id, access_hash } => {
                w.write_u32(INPUT_PEER_USER);
                w.write_i64(user_id.0);
                w.write_i64(access_hash.0);
            }
            InputPeer::Chat { chat_id } => {
                w.write_u32(INPUT_PEER_CHAT);
                w.write_i64(chat_id.0);
            }
            InputPeer::Channel { channel_id, access_hash } => {
                w.write_u32(INPUT_PEER_CHANNEL);
                w.write_i64(channel_id.0);
                w.write_i64(access_hash.0);
            }
            InputPeer::Self_ => {
                w.write_u32(INPUT_PEER_SELF);
            }
            InputPeer::UserFromId { user_id } => {
                w.write_u32(INPUT_PEER_USER_FROM_ID);
                w.write_i64(user_id.0);
            }
        }
    }

    /// Parse an InputPeer from a TL reader.
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            INPUT_PEER_USER => {
                let user_id = r.read_i64()?;
                let access_hash = r.read_i64()?;
                Ok(InputPeer::User {
                    user_id: UserId(user_id),
                    access_hash: AccessHash(access_hash),
                })
            }
            INPUT_PEER_CHAT => {
                let chat_id = r.read_i64()?;
                Ok(InputPeer::Chat { chat_id: ChatId(chat_id) })
            }
            INPUT_PEER_CHANNEL => {
                let channel_id = r.read_i64()?;
                let access_hash = r.read_i64()?;
                Ok(InputPeer::Channel {
                    channel_id: ChannelId(channel_id),
                    access_hash: AccessHash(access_hash),
                })
            }
            INPUT_PEER_SELF => Ok(InputPeer::Self_),
            INPUT_PEER_USER_FROM_ID => {
                let user_id = r.read_i64()?;
                Ok(InputPeer::UserFromId { user_id: UserId(user_id) })
            }
            other => Err(Error::Serialization(format!(
                "unknown InputPeer constructor {other:#x}"
            ))),
        }
    }
}

/// A reference to a specific user for API calls.
#[derive(Debug, Clone)]
pub enum InputUser {
    /// User with access hash.
    User { user_id: UserId, access_hash: AccessHash },
    /// Yourself.
    Self_,
    /// Legacy: user by ID only.
    FromId { user_id: UserId },
}

impl InputUser {
    pub fn write_to(&self, w: &mut TLWriter) {
        match self {
            InputUser::User { user_id, access_hash } => {
                w.write_u32(INPUT_USER);
                w.write_i64(user_id.0);
                w.write_i64(access_hash.0);
            }
            InputUser::Self_ => {
                w.write_u32(INPUT_USER_SELF);
            }
            InputUser::FromId { user_id } => {
                w.write_u32(INPUT_USER_FROM_ID);
                w.write_i64(user_id.0);
            }
        }
    }

    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            INPUT_USER => {
                let user_id = r.read_i64()?;
                let access_hash = r.read_i64()?;
                Ok(InputUser::User {
                    user_id: UserId(user_id),
                    access_hash: AccessHash(access_hash),
                })
            }
            INPUT_USER_SELF => Ok(InputUser::Self_),
            INPUT_USER_FROM_ID => {
                let user_id = r.read_i64()?;
                Ok(InputUser::FromId { user_id: UserId(user_id) })
            }
            other => Err(Error::Serialization(format!(
                "unknown InputUser constructor {other:#x}"
            ))),
        }
    }
}

/// A reference to a channel/supergroup for API calls.
#[derive(Debug, Clone)]
pub enum InputChannel {
    /// inputChannel#f35aec28 channel_id:long access_hash:long
    Channel { channel_id: ChannelId, access_hash: AccessHash },
}
impl InputChannel {
    pub fn write_to(&self, w: &mut TLWriter) {
        match self {
            InputChannel::Channel { channel_id, access_hash } => {
                w.write_u32(INPUT_CHANNEL);
                w.write_i64(channel_id.0);
                w.write_i64(access_hash.0);
            }
        }
    }

    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            INPUT_CHANNEL => {
                let channel_id = r.read_i64()?;
                let access_hash = r.read_i64()?;
                Ok(InputChannel::Channel {
                    channel_id: ChannelId(channel_id),
                    access_hash: AccessHash(access_hash),
                })
            }
            other => Err(Error::Serialization(format!(
                "unknown InputChannel constructor {other:#x}"
            ))),
        }
    }
}

// ===========================================================================
