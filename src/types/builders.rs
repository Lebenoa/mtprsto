//! Convenience builders for common TL shapes.


use super::*;
use crate::serialize::TLWriter;
#[allow(unused_imports)]
use std::fmt;

// ===========================================================================
// Convenience builders for common TL types
// ===========================================================================

/// Build an `InputPeerUser` from user_id and access_hash.
pub fn input_peer_user(user_id: i64, access_hash: i64) -> InputPeer {
    InputPeer::User {
        user_id: UserId(user_id),
        access_hash: AccessHash(access_hash),
    }
}

/// Build an `InputPeerChat` from chat_id.
pub fn input_peer_chat(chat_id: i64) -> InputPeer {
    InputPeer::Chat { chat_id: ChatId(chat_id) }
}

/// Build an `InputPeerChannel` from channel_id and access_hash.
pub fn input_peer_channel(channel_id: i64, access_hash: i64) -> InputPeer {
    InputPeer::Channel {
        channel_id: ChannelId(channel_id),
        access_hash: AccessHash(access_hash),
    }
}

/// Write a TL `vector` of long values (e.g., for deleteMessages msg_ids).
pub fn write_vector_long(w: &mut TLWriter, items: &[i64]) {
    w.write_u32(VECTOR);
    w.write_i32(items.len() as i32);
    for &item in items {
        w.write_i64(item);
    }
}

/// Write a TL `vector` of string values.
pub fn write_vector_string(w: &mut TLWriter, items: &[&[u8]]) {
    w.write_u32(VECTOR);
    w.write_i32(items.len() as i32);
    for item in items {
        w.write_bytes(item);
    }
}

