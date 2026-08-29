//! Dialog, Messages, Dialogs, State.

use super::*;
use crate::error::Result;
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
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let flags = r.read_i32()?;
        let peer = Peer::read_from(r)?;
        let top_message = MsgId(r.read_i64()?);
        let top_message_date = r.read_i32()?;
        let unread_count = r.read_i32()?;
        let read_inbox_max_id = MsgId(r.read_i64()?);
        let read_outbox_max_id = MsgId(r.read_i64()?);
        let unread_count_i32 = r.read_i32()?;
        let pts = if flags & (1 << 0) != 0 {
            Some(r.read_i32()?)
        } else {
            None
        };
        let draft = if flags & (1 << 1) != 0 {
            Some(r.read_i32()?) // simplified
        } else {
            None
        };
        let pinned = flags & (1 << 2) != 0;
        let unread_mark = flags & (1 << 3) != 0;
        Ok(Dialog {
            peer, top_message, top_message_date, unread_count,
            read_inbox_max_id, read_outbox_max_id, unread_count_i32,
            pts, draft, pinned, unread_mark,
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

// ===========================================================================
