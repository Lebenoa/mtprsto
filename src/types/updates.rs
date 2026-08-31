//! Updates container and Update event types.

use super::*;
use crate::error::{Error, Result};
use crate::serialize::TLReader;
use super::constructors::UPDATE_MESSAGE_ID;
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
    /// updateMessageID#4e90bfd6 — the id assigned to our own sent message.
    MessageID { id: MsgId },
    NewMessage { message: Message, pts: i32, pts_count: i32 },
    EditMessage { message: Message, pts: i32, pts_count: i32 },
    DeleteMessages { messages: Vec<MsgId>, pts: i32, pts_count: i32 },
    ReadHistoryInbox { peer: Peer, pts: i32, pts_count: i32 },
    ReadHistoryOutbox { peer: Peer, pts: i32, pts_count: i32 },
    ReadMessages { messages: Vec<MsgId> },
    ChannelTooLong { channel_id: ChannelId, pts: Option<i32> },
    /// `updateChannel#635b4c09 channel_id:long` — channel state changed.
    Channel { channel_id: ChannelId },
    /// `updateReadChannelInbox#922e6e10` — incoming messages read.
    ReadChannelInbox { channel_id: ChannelId, max_id: MsgId, still_unread: i32, pts: i32 },
    /// `updateNewChannelMessage#62ba04d9` — same shape as NewMessage.
    NewChannelMessage { message: Message, pts: i32, pts_count: i32 },
    /// `updateEditChannelMessage#1b3f4df7` — same shape as EditMessage.
    EditChannelMessage { message: Message, pts: i32, pts_count: i32 },
    /// `updateDeleteChannelMessages#c32d5b12`
    DeleteChannelMessages { channel_id: ChannelId, messages: Vec<MsgId>, pts: i32, pts_count: i32 },
    /// `updateReadChannelOutbox#b75f99a9`
    ReadChannelOutbox { channel_id: ChannelId, max_id: MsgId },
    Other { constructor: u32 },
}

impl Updates {
    /// Chats carried by full Updates variants (empty otherwise).
    pub fn chats(&self) -> Vec<Chat> {
        match self {
            Updates::Updates { chats, .. } | Updates::UpdatesCombined { chats, .. } => {
                chats.clone()
            }
            _ => Vec::new(),
        }
    }

    /// Parse any `Updates` container: `updates`, `updatesCombined`,
    /// `updateShort`, `updateShortSentMessage`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = TLReader::new(data);
        let ctor = r.read_u32()?;
        match ctor {
            UPDATES => {
                // updates#74ae4240 (layer 223): updates, users, chats,
                // date, seq — NO flags word.
                let updates = read_update_vector(&mut r)?;
                let users = read_user_vector(&mut r)?;
                let chats = read_chat_vector(&mut r)?;
                let date = r.read_i32()?;
                let seq = r.read_i32()?;
                Ok(Updates::Updates { updates, users, chats, date, seq })
            }
            UPDATES_COMBINED => {
                // updatesCombined#725b04c3 (layer 223): updates, users,
                // chats, date, seq_start, seq — NO flags word.
                let updates = read_update_vector(&mut r)?;
                let users = read_user_vector(&mut r)?;
                let chats = read_chat_vector(&mut r)?;
                let date = r.read_i32()?;
                let seq_start = r.read_i32()?;
                let seq = r.read_i32()?;
                Ok(Updates::UpdatesCombined { updates, users, chats, date, seq, seq_start })
            }
            UPDATE_SHORT => {
                // updateShort#78d4dec1 (layer 223): update:Update date:int
                let update = Update::read_from(&mut r)?;
                let date = r.read_i32()?;
                Ok(Updates::UpdateShort { update, date, seq: 0 })
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
            Updates::Updates { updates, .. } | Updates::UpdatesCombined { updates, .. } => {
                updates.iter().find_map(|u| match u {
                    Update::MessageID { id } => Some(*id),
                    _ => None,
                })
            }
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
            UPDATE_MESSAGE_ID => {
                // updateMessageID#4e90bfd6 id:int random_id:long
                let id = MsgId(r.read_i32()? as i64);
                let _random_id = r.read_i64()?;
                Ok(Update::MessageID { id })
            }
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
                // updateReadHistoryInbox#9e84bc99 (live) flags:#
                //   folder_id:flags.0?int peer:Peer top_msg_id:flags.1?int
                //   max_id:int still_unread_count:int pts:int pts_count:int
                // (the curated arm previously missed folder_id/top_msg_id/
                // still_unread_count, shifting every later parse by 4+)
                let flags = r.read_i32()?;
                if flags & (1 << 0) != 0 {
                    let _folder_id = r.read_i32()?;
                }
                let peer = Peer::read_from(r)?;
                if flags & (1 << 1) != 0 {
                    let _top_msg_id = MsgId(r.read_i32()? as i64);
                }
                let _max_id = MsgId(r.read_i32()? as i64);
                let _still_unread = r.read_i32()?;
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
                // updateReadMessagesContents#f8227181 flags:#
                //   messages:Vector<int> pts:int pts_count:int date:flags.0?int
                let flags = r.read_i32()?;
                let n = r.read_vector_header()?;
                let mut messages = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    messages.push(MsgId(r.read_i32()? as i64));
                }
                let _pts = r.read_i32()?;
                let _pts_count = r.read_i32()?;
                if flags & (1 << 0) != 0 {
                    let _date = r.read_i32()?;
                }
                Ok(Update::ReadMessages { messages })
            }
            UPDATE_WEB_PAGE => {
                // updateWebPage#7f891213 webpage:WebPage pts:int pts_count:int
                let _webpage = WebPage::read_from(r)?;
                let _pts = r.read_i32()?;
                let _pts_count = r.read_i32()?;
                Ok(Update::Other { constructor: UPDATE_WEB_PAGE })
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
            UPDATE_CHANNEL => {
                // updateChannel#635b4c09 channel_id:long
                Ok(Update::Channel { channel_id: ChannelId(r.read_i64()?) })
            }
            UPDATE_READ_CHANNEL_INBOX => {
                // updateReadChannelInbox#922e6e10 flags:# folder_id:flags.0?int
                //   channel_id:long max_id:int still_unread_count:int pts:int
                let flags = r.read_i32()?;
                if flags & (1 << 0) != 0 {
                    let _folder_id = r.read_i32()?;
                }
                let channel_id = ChannelId(r.read_i64()?);
                let max_id = MsgId(r.read_i32()? as i64);
                let still_unread = r.read_i32()?;
                let pts = r.read_i32()?;
                Ok(Update::ReadChannelInbox { channel_id, max_id, still_unread, pts })
            }
            UPDATE_NEW_CHANNEL_MESSAGE => {
                // updateNewChannelMessage#62ba04d9 message:Message pts:int pts_count:int
                let message = Message::read_from(r)?;
                let pts = r.read_i32()?;
                let pts_count = r.read_i32()?;
                Ok(Update::NewChannelMessage { message, pts, pts_count })
            }
            UPDATE_EDIT_CHANNEL_MESSAGE => {
                // updateEditChannelMessage#1b3f4df7 message:Message pts:int pts_count:int
                let message = Message::read_from(r)?;
                let pts = r.read_i32()?;
                let pts_count = r.read_i32()?;
                Ok(Update::EditChannelMessage { message, pts, pts_count })
            }
            UPDATE_DELETE_CHANNEL_MESSAGES => {
                // updateDeleteChannelMessages#c32d5b12 channel_id:long
                //   messages:Vector<int> pts:int pts_count:int
                let channel_id = ChannelId(r.read_i64()?);
                let n = r.read_vector_header()?;
                let mut messages = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    messages.push(MsgId(r.read_i32()? as i64));
                }
                let pts = r.read_i32()?;
                let pts_count = r.read_i32()?;
                Ok(Update::DeleteChannelMessages { channel_id, messages, pts, pts_count })
            }
            UPDATE_READ_CHANNEL_OUTBOX => {
                // updateReadChannelOutbox#b75f99a9 channel_id:long max_id:int
                let channel_id = ChannelId(r.read_i64()?);
                let max_id = MsgId(r.read_i32()? as i64);
                Ok(Update::ReadChannelOutbox { channel_id, max_id })
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
                // differenceEmpty#5d75a138 date:int seq:int
                Ok(Difference::Empty {
                    date: r.read_i32()?,
                    seq: r.read_i32()?,
                    pts: 0,
                    pts_count: 0,
                })
            }
            DIFFERENCE => {
                // difference#f49ca0 new_messages new_encrypted_messages
                //   other_updates chats users state
                let new_messages = read_msg_vector(&mut r)?;
                let enc_count = r.read_vector_header()?; // new_encrypted_messages
                if enc_count > 0 {
                    return Err(Error::Serialization(
                        "EncryptedMessage parsing not supported".into(),
                    ));
                }
                if enc_count > 0 {
                    return Err(Error::Serialization(
                        "EncryptedMessage parsing not supported".into(),
                    ));
                }
                let other_updates = read_update_vector(&mut r)?;
                let chats = read_chat_vector(&mut r)?;
                let users = read_user_vector(&mut r)?;
                // state#a56c2a3e pts qts date seq unread_count
                let _st_ctor = r.read_u32()?;
                let _pts = r.read_i32()?;
                let _qts = r.read_i32()?;
                let _date = r.read_i32()?;
                let seq = r.read_i32()?;
                let _unread = r.read_i32()?;
                Ok(Difference::Difference { seq, new_messages, other_updates, chats, users })
            }
            DIFFERENCE_SLICE => {
                // differenceSlice#a8fb1981 new_messages new_encrypted_messages
                //   other_updates chats users intermediate_state
                let new_messages = read_msg_vector(&mut r)?;
                let enc_count = r.read_vector_header()?; // new_encrypted_messages
                if enc_count > 0 {
                    return Err(Error::Serialization(
                        "EncryptedMessage parsing not supported".into(),
                    ));
                }
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
                // channelDifferenceEmpty#3e11affb flags:# pts:int
                //   timeout:flags.1?int
                let flags = r.read_i32()?;
                let pts = r.read_i32()?;
                let timeout = if flags & (1 << 1) != 0 {
                    Some(r.read_i32()?)
                } else {
                    None
                };
                let _ = timeout; // surface via TooLong/Difference arms
                Ok(ChannelDifference::Empty {
                    pts,
                    final_: flags & (1 << 0) != 0,
                })
            }
            CHANNEL_DIFFERENCE => {
                let (timeout, new_messages, other_updates, chats, users, pts, final_) =
                    Self::read_full(&mut r)?;
                Ok(ChannelDifference::Difference { pts, final_, timeout, new_messages, other_updates, chats, users })
            }
            CHANNEL_DIFFERENCE_TOO_LONG => {
                // channelDifferenceTooLong#a4bcc6fe flags:# final:flags.0?true
                //   timeout:flags.1?int dialog:Dialog messages:Vector<Message>
                //   chats:Vector<Chat> users:Vector<User>
                let flags = r.read_i32()?;
                let final_ = flags & (1 << 0) != 0;
                let timeout = if flags & (1 << 1) != 0 {
                    Some(r.read_i32()?)
                } else {
                    None
                };
                let pts = read_dialog_skip(&mut r)?;
                let new_messages = read_msg_vector(&mut r)?;
                let other_updates = read_update_vector(&mut r)?;
                let chats = read_chat_vector(&mut r)?;
                let users = read_user_vector(&mut r)?;
                Ok(ChannelDifference::TooLong {
                    pts: pts.unwrap_or(0),
                    final_,
                    timeout,
                    new_messages,
                    other_updates,
                    chats,
                    users,
                })
            }
            other => Err(Error::Serialization(format!(
                "unknown ChannelDifference constructor {other:#x}"
            ))),
        }
    }

    /// Tail for channelDifference: flags pts final:flags.0 timeout:flags.1
    /// messages chats users
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
/// Skip a `dialog#fc89f7f3`, returning its pts (flags.0) when present.
/// Drafts (flags.1) cannot be skipped without a full InputMedia parser —
/// those fail loudly rather than desync.
fn read_dialog_skip(r: &mut TLReader) -> Result<Option<i32>> {
    let ctor = r.read_u32()?;
    if ctor != DIALOG {
        return Err(Error::Serialization(format!(
            "expected dialog in channelDifferenceTooLong, got {ctor:#x}"
        )));
    }
    let dflags = r.read_i32()?;
    let _peer = Peer::read_from(r)?;
    // top_message, read_inbox_max_id, read_outbox_max_id, unread_count,
    // unread_mentions_count, unread_reactions_count, unread_poll_votes_count
    for _ in 0..7 {
        let _ = r.read_i32()?;
    }
    skip_peer_notify_settings(r)?;
    let pts = if dflags & (1 << 0) != 0 {
        Some(r.read_i32()?)
    } else {
        None
    };
    if dflags & (1 << 1) != 0 {
        return Err(Error::Serialization(
            "dialog draft (DraftMessage) parsing not supported".into(),
        ));
    }
    if dflags & (1 << 4) != 0 {
        let _folder_id = r.read_i32()?;
    }
    if dflags & (1 << 5) != 0 {
        let _ttl_period = r.read_i32()?;
    }
    Ok(pts)
}

/// Skip `peerNotifySettings#99622c0c`: flags then Bool/int/NotificationSound
/// conditionals for flags 0..10.
fn skip_peer_notify_settings(r: &mut TLReader) -> Result<()> {
    let ctor = r.read_u32()?;
    if ctor != PEER_NOTIFY_SETTINGS {
        return Err(Error::Serialization(format!(
            "expected peerNotifySettings, got {ctor:#x}"
        )));
    }
    let flags = r.read_i32()?;
    // (bit, kind): Bool for 0,1,6,7; int for 2; sound for 3,4,5,8,9,10
    for bit in 0..11 {
        if flags & (1 << bit) == 0 {
            continue;
        }
        match bit {
            0 | 1 | 6 | 7 => {
                // Bool — ctor-serialized
                let _ = r.read_u32()?;
            }
            2 => {
                let _ = r.read_i32()?;
            }
            _ => {
                // NotificationSound union
                let sctor = r.read_u32()?;
                match sctor {
                    NOTIFICATION_SOUND_DEFAULT | NOTIFICATION_SOUND_NONE => {}
                    NOTIFICATION_SOUND_LOCAL | NOTIFICATION_SOUND_RINGTONE => {
                        let _title = r.read_bytes()?;
                        if sctor == NOTIFICATION_SOUND_LOCAL {
                            let _data = r.read_bytes()?;
                        } else {
                            let _id = r.read_i64()?;
                        }
                    }
                    other => {
                        return Err(Error::Serialization(format!(
                            "unknown NotificationSound constructor {other:#x}"
                        )))
                    }
                }
            }
        }
    }
    Ok(())
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

/// Public re-exports for response parsers outside this module.
pub fn read_chat_vector_public(r: &mut TLReader) -> Result<Vec<Chat>> {
    read_chat_vector(r)
}

pub fn read_user_vector_public(r: &mut TLReader) -> Result<Vec<User>> {
    read_user_vector(r)
}

/// Public re-export of the peerNotifySettings skipper (used by Dialog).
pub fn skip_peer_notify_settings_public(r: &mut TLReader) -> Result<()> {
    skip_peer_notify_settings(r)
}
