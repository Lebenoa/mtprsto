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
    /// Parse any `Updates` container: `updates`, `updatesCombined`,
    /// `updateShort`, `updateShortSentMessage`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = TLReader::new(data);
        let ctor = r.read_u32()?;
        match ctor {
            UPDATES => {
                // updates#74ae4240 flags:int date:int seq:int
                //   updates:Vector<Update> chats:Vector<Chat> users:Vector<User>
                let _flags = r.read_i32()?;
                let date = r.read_i32()?;
                let seq = r.read_i32()?;
                let updates = Self::read_update_vector(&mut r)?;
                let chats = Self::read_chat_vector(&mut r)?;
                let users = Self::read_user_vector(&mut r)?;
                Ok(Updates::Updates { updates, users, chats, date, seq })
            }
            UPDATES_COMBINED => {
                // updatesCombined#725b04c3 flags:int date:int seq:int
                //   seq_start:int updates:Vector<Update> chats:... users:...
                let _flags = r.read_i32()?;
                let date = r.read_i32()?;
                let seq = r.read_i32()?;
                let seq_start = r.read_i32()?;
                let updates = Self::read_update_vector(&mut r)?;
                let chats = Self::read_chat_vector(&mut r)?;
                let users = Self::read_user_vector(&mut r)?;
                Ok(Updates::UpdatesCombined { updates, users, chats, date, seq, seq_start })
            }
            UPDATE_SHORT => {
                // updateShort#78d4dec1 update:Update date:int seq:int
                let update = Update::read_from(&mut r)?;
                let date = r.read_i32()?;
                let seq = r.read_i32()?;
                Ok(Updates::UpdateShort { update, date, seq })
            }
            UPDATE_SHORT_SENT_MESSAGE => {
                // updateShortSentMessage#9015e101 flags:int id:int pts:int
                //   pts_count:int date:int
                let _flags = r.read_i32()?;
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

    /// Read a Vector<Update>, decoding each inner update by constructor.
    fn read_update_vector(r: &mut TLReader) -> Result<Vec<Update>> {
        let count = r.read_vector_header()?;
        let mut updates = Vec::with_capacity(count as usize);
        for _ in 0..count {
            updates.push(Update::read_from(r)?);
        }
        Ok(updates)
    }

    fn read_chat_vector(r: &mut TLReader) -> Result<Vec<Chat>> {
        let count = r.read_vector_header()?;
        let mut chats = Vec::with_capacity(count as usize);
        for _ in 0..count {
            chats.push(Chat::read_from(r)?);
        }
        Ok(chats)
    }

    fn read_user_vector(r: &mut TLReader) -> Result<Vec<User>> {
        let count = r.read_vector_header()?;
        let mut users = Vec::with_capacity(count as usize);
        for _ in 0..count {
            users.push(User::read_from(r)?);
        }
        Ok(users)
    }

    /// Get the message ID if this is a short sent message update.
    pub fn message_id(&self) -> Option<MsgId> {
        match self {
            Updates::UpdateShortSentMessage { id, .. } => Some(*id),
            _ => None,
        }
    }
}

impl Update {
    /// Decode a single update object by its constructor. Unrecognized
    /// constructors are preserved as `Update::Other` (with their inner
    /// bytes left unread — the caller owns the stream position, so a
    /// container parse treats them as opaque).
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            UPDATE_NEW_MESSAGE | UPDATE_EDIT_MESSAGE => {
                // updateNewMessage#1f2b0afd message:Message pts:int pts_count:int
                // updateEditMessage#e40370a3 message:Message pts:int pts_count:int
                let message = Message::read_from(r)?;
                let pts = r.read_i32()?;
                let pts_count = r.read_i32()?;
                if ctor == UPDATE_NEW_MESSAGE {
                    Ok(Update::NewMessage { message, pts, pts_count })
                } else {
                    Ok(Update::EditMessage { message, pts, pts_count })
                }
            }
            UPDATE_DELETE_MESSAGES => {
                // updateDeleteMessages#a20db0e5 messages:Vector<int> pts:int pts_count:int
                let n = r.read_vector_header()?;
                let mut messages = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    messages.push(MsgId(r.read_i32()? as i64));
                }
                let pts = r.read_i32()?;
                let pts_count = r.read_i32()?;
                Ok(Update::DeleteMessages { messages, pts, pts_count })
            }
            UPDATE_READ_HISTORY_INBOX => {
                // updateReadHistoryInbox#9e84bc99 flags:int peer:Peer max_id:int
                //   pts:int pts_count:int
                let _flags = r.read_i32()?;
                let peer = Peer::read_from(r)?;
                let _max_id = MsgId(r.read_i32()? as i64);
                let pts = r.read_i32()?;
                let pts_count = r.read_i32()?;
                Ok(Update::ReadHistoryInbox { peer, pts, pts_count })
            }
            UPDATE_READ_HISTORY_OUTBOX => {
                // updateReadHistoryOutbox#2f2f21bf peer:Peer max_id:int pts:int pts_count:int
                let peer = Peer::read_from(r)?;
                let _max_id = MsgId(r.read_i32()? as i64);
                let pts = r.read_i32()?;
                let pts_count = r.read_i32()?;
                Ok(Update::ReadHistoryOutbox { peer, pts, pts_count })
            }
            UPDATE_READ_MESSAGES => {
                // updateReadMessages#c66f9217 messages:Vector<int>
                let n = r.read_vector_header()?;
                let mut messages = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    messages.push(MsgId(r.read_i32()? as i64));
                }
                Ok(Update::ReadMessages { messages })
            }
            UPDATE_CHANNEL_TOO_LONG => {
                // updateChannelTooLong#108d941f flags:# channel_id:long pts:flags.0?int
                let flags = r.read_i32()?;
                let channel_id = ChannelId(r.read_i64()?);
                let pts = if flags & (1 << 0) != 0 {
                    Some(r.read_i32()?)
                } else {
                    None
                };
                Ok(Update::ChannelTooLong { channel_id, pts })
            }
            other => Ok(Update::Other { constructor: other }),
        }
    }
}
