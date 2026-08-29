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
                let updates = read_update_vector(&mut r)?;
                let chats = read_chat_vector(&mut r)?;
                let users = read_user_vector(&mut r)?;
                Ok(Updates::Updates { updates, users, chats, date, seq })
            }
            UPDATES_COMBINED => {
                // updatesCombined#725b04c3 flags:int date:int seq:int
                //   seq_start:int updates:Vector<Update> chats:... users:...
                let _flags = r.read_i32()?;
                let date = r.read_i32()?;
                let seq = r.read_i32()?;
                let seq_start = r.read_i32()?;
                let updates = read_update_vector(&mut r)?;
                let chats = read_chat_vector(&mut r)?;
                let users = read_user_vector(&mut r)?;
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


/// Result of `updates.getDifference` (SPEC §6).
#[derive(Debug, Clone)]
pub enum Difference {
    /// No updates to apply — session is current as of the returned state.
    Empty { date: i32, seq: i32, pts: i32, pts_count: i32 },
    /// Apply these updates; pts/seq tracked by the contained updates.
    Difference {
        seq: i32,
        new_messages: Vec<Message>,
        other_updates: Vec<Update>,
        chats: Vec<Chat>,
        users: Vec<User>,
    },
    /// Slice: more difference rounds needed (`intermediate_state` is the
    /// new pts cursor).
    Slice {
        new_messages: Vec<Message>,
        other_updates: Vec<Update>,
        chats: Vec<Chat>,
        users: Vec<User>,
        intermediate_state: State,
    },
    /// Local state too far behind — caller must re-sync via getDialogs etc.
    TooLong { pts: i32 },
}

/// Result of `updates.getChannelDifference` (SPEC §6).
#[derive(Debug, Clone)]
pub enum ChannelDifference {
    /// Channel is up to date.
    Empty { pts: i32, final_: bool },
    /// Apply messages/updates; if `final_`, loop ends regardless of pts.
    Difference {
        pts: i32,
        final_: bool,
        timeout: Option<i32>,
        new_messages: Vec<Message>,
        other_updates: Vec<Update>,
        chats: Vec<Chat>,
        users: Vec<User>,
    },
    /// Too far behind — caller should re-fetch the channel's dialogs.
    TooLong {
        pts: i32,
        final_: bool,
        timeout: Option<i32>,
        new_messages: Vec<Message>,
        other_updates: Vec<Update>,
        chats: Vec<Chat>,
        users: Vec<User>,
    },
}

impl Difference {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = TLReader::new(data);
        let ctor = r.read_u32()?;
        match ctor {
            DIFFERENCE_EMPTY => {
                // differenceEmpty#a9eca690 date seq pts pts_count
                Ok(Difference::Empty {
                    date: r.read_i32()?,
                    seq: r.read_i32()?,
                    pts: r.read_i32()?,
                    pts_count: r.read_i32()?,
                })
            }
            DIFFERENCE => {
                // difference#f46ca0 seq new_messages other_updates chats users
                let seq = r.read_i32()?;
                let new_messages = read_msg_vector(&mut r)?;
                let other_updates = read_update_vector(&mut r)?;
                let chats = read_chat_vector(&mut r)?;
                let users = read_user_vector(&mut r)?;
                Ok(Difference::Difference { seq, new_messages, other_updates, chats, users })
            }
            DIFFERENCE_SLICE => {
                // differenceSlice#a004db6 ... intermediate_state:State
                let new_messages = read_msg_vector(&mut r)?;
                let other_updates = read_update_vector(&mut r)?;
                let chats = read_chat_vector(&mut r)?;
                let users = read_user_vector(&mut r)?;
                // intermediate_state#a56c2a3e pts qts date seq unread_count
                let _st_ctor = r.read_u32()?;
                let intermediate_state = State {
                    pts: r.read_i32()?,
                    qts: r.read_i32()?,
                    date: r.read_i32()?,
                    seq: r.read_i32()?,
                    unread_count: r.read_i32()?,
                };
                Ok(Difference::Slice { new_messages, other_updates, chats, users, intermediate_state })
            }
            DIFFERENCE_TOO_LONG => {
                Ok(Difference::TooLong { pts: r.read_i32()? })
            }
            other => Err(Error::Serialization(format!(
                "unknown Difference constructor {other:#x}"
            ))),
        }
    }
}

impl ChannelDifference {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = TLReader::new(data);
        let ctor = r.read_u32()?;
        match ctor {
            CHANNEL_DIFFERENCE_EMPTY => {
                // channelDifferenceEmpty#3e11affb flags:# pts:int final:flags.0?true
                let flags = r.read_i32()?;
                Ok(ChannelDifference::Empty {
                    pts: r.read_i32()?,
                    final_: flags & (1 << 0) != 0,
                })
            }
            CHANNEL_DIFFERENCE => {
                let (timeout, new_messages, other_updates, chats, users, pts, final_) =
                    Self::read_full(&mut r)?;
                Ok(ChannelDifference::Difference { pts, final_, timeout, new_messages, other_updates, chats, users })
            }
            CHANNEL_DIFFERENCE_TOO_LONG => {
                let (timeout, new_messages, other_updates, chats, users, pts, final_) =
                    Self::read_full(&mut r)?;
                Ok(ChannelDifference::TooLong { pts, final_, timeout, new_messages, other_updates, chats, users })
            }
            other => Err(Error::Serialization(format!(
                "unknown ChannelDifference constructor {other:#x}"
            ))),
        }
    }

    /// Shared tail for channelDifference / channelDifferenceTooLong:
    /// flags pts final:flags.0 timeout:flags.1 messages chats users
    #[allow(clippy::type_complexity)]
    fn read_full(
        r: &mut TLReader,
    ) -> Result<(Option<i32>, Vec<Message>, Vec<Update>, Vec<Chat>, Vec<User>, i32, bool)> {
        let flags = r.read_i32()?;
        let pts = r.read_i32()?;
        let final_ = flags & (1 << 0) != 0;
        let timeout = if flags & (1 << 1) != 0 {
            Some(r.read_i32()?)
        } else {
            None
        };
        let new_messages = read_msg_vector(r)?;
        let other_updates = read_update_vector(r)?;
        let chats = read_chat_vector(r)?;
        let users = read_user_vector(r)?;
        Ok((timeout, new_messages, other_updates, chats, users, pts, final_))
    }
}
/// Read a TL `Vector<Update>`, decoding each element by constructor.
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

fn read_msg_vector(r: &mut TLReader) -> Result<Vec<Message>> {
    let count = r.read_vector_header()?;
    let mut messages = Vec::with_capacity(count as usize);
    for _ in 0..count {
        messages.push(Message::read_from(r)?);
    }
    Ok(messages)
}
