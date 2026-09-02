//! Dialog, Messages, Dialogs, State.

use super::{
    Chat, MESSAGES_DIALOGS, MESSAGES_DIALOGS_NOT_MODIFIED, MESSAGES_DIALOGS_SLICE, Message, MsgId,
    Peer, User,
};
use crate::error::{Error, Result};
use crate::serialize::TLReader;
#[allow(unused_imports)]
use std::fmt;

// §7 Dialog and Messages types
// ===========================================================================

/// A dialog (conversation).
#[derive(Debug, Clone)]
pub struct Dialog {
    pub peer: Peer,
    pub top_message: MsgId,
    pub top_message_date: i32,
    pub unread_count: i32,
    pub read_inbox_max_id: MsgId,
    pub read_outbox_max_id: MsgId,
    pub unread_count_i32: i32,
    pub pts: Option<i32>,
    pub draft: Option<i32>,
    pub pinned: bool,
    pub unread_mark: bool,
}

impl Dialog {
    /// Parse `dialog#fc89f7f3` (layer 225 — carries
    /// `unread_poll_votes_count`) or the 223-era `dialog#d58a08c6`
    /// (no such field), discriminating by constructor id.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Serialization`] when any field of the dialog
    /// payload fails to decode.
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        // consume the constructor before the flags word (its omission
        // misaligned the whole parse).
        let ctor = r.read_u32()?;
        let flags = r.read_i32()?;
        let peer = Peer::read_from(r)?;
        let top_message = MsgId(i64::from(r.read_i32()?));
        let read_inbox_max_id = MsgId(i64::from(r.read_i32()?));
        let read_outbox_max_id = MsgId(i64::from(r.read_i32()?));
        let unread_count = r.read_i32()?;
        let _unread_mentions = r.read_i32()?;
        let _unread_reactions = r.read_i32()?;
        // dialog#fc89f7f3 (layer 225) inserts unread_poll_votes_count
        // here; the 223-era dialog#d58a08c6 has no such field. Reading
        // it for the wrong shape shifted every later field by 4 bytes
        // and broke get_dialogs.
        if ctor == crate::types::DIALOG {
            let _unread_poll_votes = r.read_i32()?;
        }
        // notify_settings:PeerNotifySettings — always present.
        crate::types::skip_peer_notify_settings_public(r)?;
        let pts = if flags & (1 << 0) != 0 {
            Some(r.read_i32()?)
        } else {
            None
        };
        if flags & (1 << 1) != 0 {
            // draft:flags.1?DraftMessage — decode via the generated
            // parser and discard; the curated model keeps no draft body.
            let _draft = crate::types::DraftMessage::read_from(r)?;
        }
        if flags & (1 << 4) != 0 {
            let _folder_id = r.read_i32()?;
        }
        if flags & (1 << 5) != 0 {
            let _ttl_period = r.read_i32()?;
        }
        let pinned = flags & (1 << 2) != 0;
        let unread_mark = flags & (1 << 3) != 0;
        Ok(Self {
            peer,
            top_message,
            top_message_date: 0,
            unread_count,
            read_inbox_max_id,
            read_outbox_max_id,
            unread_count_i32: unread_count,
            pts,
            draft: None,
            pinned,
            unread_mark,
        })
    }
}

/// A list of messages.
#[derive(Debug, Clone)]
pub struct Messages {
    pub messages: Vec<Message>,
}

impl Messages {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            messages: Vec::new(),
        }
    }
}

/// A list of dialogs.
#[derive(Debug, Clone)]
pub struct Dialogs {
    pub dialogs: Vec<Dialog>,
    pub messages: Vec<Message>,
    pub users: Vec<User>,
    pub chats: Vec<Chat>,
}

impl Dialogs {
    /// Decode a `messages.dialogs#15ba6c40` / `messages.dialogsSlice#71e094f3`
    /// answer (constructor included): dialogs first, then messages, chats,
    /// users; the slice variant carries a leading `count:int`.
    /// `messages.dialogsNotModified#f0e3e596` decodes to an empty list.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Serialization`] for an unexpected constructor or a
    /// nested dialog/message/chat/user payload that fails to decode.
    #[allow(clippy::cast_sign_loss, clippy::as_conversions)] // TL vector header: length-prefixed i32 count, non-negative on well-formed frames
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = TLReader::new(data);
        let ctor = r.read_u32()?;
        match ctor {
            MESSAGES_DIALOGS | MESSAGES_DIALOGS_SLICE => {
                if ctor == MESSAGES_DIALOGS_SLICE {
                    let _count = r.read_i32()?;
                }
                let n = r.read_vector_header()?;
                let mut dialogs = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    dialogs.push(Dialog::read_from(&mut r)?);
                }
                let n = r.read_vector_header()?;
                let mut messages = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    messages.push(Message::read_from(&mut r)?);
                }
                let chats = crate::types::read_chat_vector_public(&mut r)?;
                let users = crate::types::read_user_vector_public(&mut r)?;
                Ok(Self {
                    dialogs,
                    messages,
                    users,
                    chats,
                })
            }
            MESSAGES_DIALOGS_NOT_MODIFIED => Ok(Self {
                dialogs: Vec::new(),
                messages: Vec::new(),
                users: Vec::new(),
                chats: Vec::new(),
            }),
            other => Err(Error::Serialization(format!(
                "unexpected getDialogs response {other:#x}"
            ))),
        }
    }
}

/// Response to updates.getState.
#[derive(Debug, Clone)]
pub struct State {
    pub pts: i32,
    pub qts: i32,
    pub date: i32,
    pub seq: i32,
    pub unread_count: i32,
}

impl State {
    /// Decode `updates.state#a56c2a3e pts:int qts:int date:int seq:int
    /// unread_count:int`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Serialization`] when the constructor is not
    /// `updates.state` or a field fails to decode.
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        if ctor != crate::types::UPDATES_STATE {
            return Err(Error::Serialization(format!(
                "expected updates.state, got {ctor:#x}"
            )));
        }
        Ok(Self {
            pts: r.read_i32()?,
            qts: r.read_i32()?,
            date: r.read_i32()?,
            seq: r.read_i32()?,
            unread_count: r.read_i32()?,
        })
    }
}

// ===========================================================================
