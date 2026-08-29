//! Updates container and Update event types.

use super::*;
use crate::error::{Error, Result};
use crate::serialize::TLReader;
#[allow(unused_imports)]
use std::fmt;

// §7 Updates types (per SPEC §6)
// ===========================================================================

/// An update from Telegram's update channel.
// Same rationale as `User`: the vectors dominate either way.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Updates {
    /// Full updates with users and chats.
    Updates {
        updates: Vec<Update>,
        users: Vec<User>,
        chats: Vec<Chat>,
        date: i32,
        seq: i32,
    },
    /// Short update (single).
    UpdateShort {
        update: Update,
        date: i32,
        seq: i32,
    },
    /// UpdatesCombined (with seq_start).
    UpdatesCombined {
        updates: Vec<Update>,
        users: Vec<User>,
        chats: Vec<Chat>,
        date: i32,
        seq: i32,
        seq_start: i32,
    },
    /// Short for a sent message (response to sendMessage).
    UpdateShortSentMessage {
        id: MsgId,
        pts: i32,
        pts_count: i32,
        date: i32,
    },
}

/// A single update event.
#[derive(Debug, Clone)]
pub enum Update {
    NewMessage { message: Message, pts: i32, pts_count: i32 },
    EditMessage { message: Message, pts: i32, pts_count: i32 },
    DeleteMessages { messages: Vec<MsgId>, pts: i32, pts_count: i32 },
    ReadHistoryInbox { peer: Peer, pts: i32, pts_count: i32 },
    ReadHistoryOutbox { peer: Peer, pts: i32, pts_count: i32 },
    ReadMessages { messages: Vec<MsgId> },
    ChannelTooLong { channel_id: ChannelId, pts: Option<i32> },
    Other { constructor: u32 },
}

impl Updates {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = TLReader::new(data);
        let ctor = r.read_u32()?;
        match ctor {
            UPDATES => {
                let _flags = r.read_i32()?;
                let _date = r.read_i32()?;
                let seq = r.read_i32()?;
                // updates vector
                let _v_ctor = r.read_u32()?;
                let count = r.read_i32()?;
                let mut updates = Vec::new();
                for _ in 0..count {
                    let u_ctor = r.read_u32()?;
                    // Simplified: skip inner update data
                    while r.remaining() > 0 {
                        let _ = r.read_i32()?;
                    }
                    updates.push(Update::Other { constructor: u_ctor });
                }
                Ok(Updates::Updates {
                    updates, users: Vec::new(), chats: Vec::new(),
                    date: 0, seq,
                })
            }
            UPDATE_SHORT => {
                let u_ctor = r.read_i32()?;
                let date = r.read_i32()?;
                let seq = r.read_i32()?;
                Ok(Updates::UpdateShort {
                    update: Update::Other { constructor: u_ctor as u32 },
                    date, seq,
                })
            }
            UPDATES_COMBINED => {
                Ok(Updates::Updates { updates: Vec::new(), users: Vec::new(), chats: Vec::new(), date: 0, seq: 0 })
            }
            UPDATE_SHORT_SENT_MESSAGE => {
                // updateShortSentMessage#9015e101 id:int pts:int pts_count:int date:int
                let id = MsgId(r.read_i32()? as i64);
                let pts = r.read_i32()?;
                let pts_count = r.read_i32()?;
                let date = r.read_i32()?;
                Ok(Updates::UpdateShortSentMessage { id, pts, pts_count, date })
            }
            other => Err(Error::Serialization(format!(
                "unknown Updates constructor {other:#x}"
            ))),
        }
    }

    /// Get the message ID if this is a short sent message update.
    pub fn message_id(&self) -> Option<MsgId> {
        match self {
            Updates::UpdateShortSentMessage { id, .. } => Some(*id),
            _ => None,
        }
    }
}

// ===========================================================================
