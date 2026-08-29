//! Peer — the user/chat/channel reference found in messages and dialogs.

use super::*;
use crate::error::{Error, Result};
use crate::serialize::{TLWriter, TLReader};
#[allow(unused_imports)]
use std::fmt;

// §7 Peer types
// ===========================================================================

/// A chat/user/channel peer reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Peer {
    User { user_id: UserId },
    Chat { chat_id: ChatId },
    Channel { channel_id: ChannelId },
    None,
}

impl Peer {
    pub fn user_id(&self) -> Option<UserId> {
        if let Peer::User { user_id } = self { Some(*user_id) } else { None }
    }

    pub fn channel_id(&self) -> Option<ChannelId> {
        if let Peer::Channel { channel_id } = self { Some(*channel_id) } else { None }
    }

    pub fn chat_id(&self) -> Option<ChatId> {
        if let Peer::Chat { chat_id } = self { Some(*chat_id) } else { None }
    }

    pub fn write_to(&self, w: &mut TLWriter) {
        match self {
            Peer::User { user_id } => {
                w.write_u32(PEER_USER);
                w.write_i64(user_id.0);
            }
            Peer::Chat { chat_id } => {
                w.write_u32(PEER_CHAT);
                w.write_i64(chat_id.0);
            }
            Peer::Channel { channel_id } => {
                w.write_u32(PEER_CHANNEL);
                w.write_i64(channel_id.0);
            }
            Peer::None => {}
        }
    }

    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            PEER_USER => {
                let user_id = r.read_i64()?;
                Ok(Peer::User { user_id: UserId(user_id) })
            }
            PEER_CHAT => {
                let chat_id = r.read_i64()?;
                Ok(Peer::Chat { chat_id: ChatId(chat_id) })
            }
            PEER_CHANNEL => {
                let channel_id = r.read_i64()?;
                Ok(Peer::Channel { channel_id: ChannelId(channel_id) })
            }
            other => Err(Error::Serialization(format!(
                "unknown Peer constructor {other:#x}"
            ))),
        }
    }
}

// ===========================================================================
