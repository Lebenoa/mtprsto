//! Dialog, Messages, Dialogs, State.

use super::*;
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
    /// Parse `dialog#fc89f7f3 flags:# pinned:flags.2?true
    /// unread_mark:flags.3?true view_forum_as_messages:flags.6?true
    /// peer:Peer top_message:int read_inbox_max_id:int
    /// read_outbox_max_id:int unread_count:int unread_mentions_count:int
    /// unread_reactions_count:int unread_poll_votes_count:int
    /// notify_settings:PeerNotifySettings pts:flags.0?int
    /// draft:flags.1?DraftMessage folder_id:flags.4?int
    /// ttl_period:flags.5?int`.
    /// Parse `dialog#d58a08c6 flags:# pinned:flags.2?true
    /// unread_mark:flags.3?true view_forum_as_messages:flags.6?true
    /// peer:Peer top_message:int read_inbox_max_id:int
    /// read_outbox_max_id:int unread_count:int unread_mentions_count:int
    /// unread_reactions_count:int notify_settings:PeerNotifySettings
    /// pts:flags.0?int draft:flags.1?DraftMessage folder_id:flags.4?int
    /// ttl_period:flags.5?int` (the published layer-223 shape — no
    /// unread_poll_votes_count; that field exists only from layer 225).
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        // consume the constructor before the flags word (its omission
        // misaligned the whole parse).
        let _ctor = r.read_u32()?;
        let flags = r.read_i32()?;
        let peer = Peer::read_from(r)?;
        let top_message = MsgId(r.read_i32()? as i64);
        let read_inbox_max_id = MsgId(r.read_i32()? as i64);
        let read_outbox_max_id = MsgId(r.read_i32()? as i64);
        let unread_count = r.read_i32()?;
        let _unread_mentions = r.read_i32()?;
        let _unread_reactions = r.read_i32()?;
        // NOTE: layer 223 dialog has NO unread_poll_votes_count (that
        // field appears from layer 225) — reading it here shifted every
        // later field by 4 bytes and broke get_dialogs.
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
        Ok(Dialog {
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
    pub fn empty() -> Self {
        Self { messages: Vec::new() }
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
