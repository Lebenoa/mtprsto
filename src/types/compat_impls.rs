//! Helper impls on the GENERATED input types (schema parsing lives in
//! `input_gen.rs`; this module only adds the serialization side that the
//! generator does not emit for unions).

use super::input_gen::{InputChannel, InputDocument, InputFile, InputPeer, InputUser};
use super::peer_gen::Peer;
use super::photo_gen::{Document, PhotoSize};
use super::photo::FileLocation;
use crate::types::constructors::{PEER_CHANNEL, PEER_CHAT, PEER_USER};
use super::ids::{ChannelId, ChatId, UserId};
use crate::serialize::TLWriter;
use crate::types::constructors::{
    INPUT_CHANNEL, INPUT_DOCUMENT, INPUT_DOCUMENT_EMPTY, INPUT_FILE,
    INPUT_FILE_BIG, INPUT_PEER_CHANNEL, INPUT_PEER_CHAT, INPUT_PEER_EMPTY,
    INPUT_PEER_SELF, INPUT_PEER_USER, INPUT_USER, INPUT_USER_SELF,
};

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
            InputPeer::InputPeerEmpty => {
                w.write_u32(INPUT_PEER_EMPTY);
            }
            _ => {}
        }
    }
}

impl InputUser {
    /// TL-serialize this InputUser.
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
            _ => {}
        }
    }
}

impl InputChannel {
    /// TL-serialize this InputChannel.
    pub fn write_to(&self, w: &mut TLWriter) {
        if let InputChannel::Channel { channel_id, access_hash } = self {
            w.write_u32(INPUT_CHANNEL);
            w.write_i64(channel_id.0);
            w.write_i64(access_hash.0);
        }
    }
}

impl InputFile {
    /// TL-serialize this InputFile.
    pub fn write_to(&self, w: &mut TLWriter) {
        match self {
            InputFile::Id { id, parts, name, md5_checksum } => {
                // md5_checksum is UNCONDITIONAL (no flags field).
                w.write_u32(INPUT_FILE);
                w.write_i64(*id);
                w.write_i32(*parts);
                w.write_bytes(name.as_bytes());
                w.write_bytes(md5_checksum.as_bytes());
            }
            InputFile::Big { id, parts, name } => {
                w.write_u32(INPUT_FILE_BIG);
                w.write_i64(*id);
                w.write_i32(*parts);
                w.write_bytes(name.as_bytes());
            }
            _ => {}
        }
    }
}

impl InputDocument {
    /// TL-serialize this InputDocument.
    pub fn write_to(&self, w: &mut TLWriter) {
        match self {
            InputDocument::Document { id, access_hash, file_reference } => {
                w.write_u32(INPUT_DOCUMENT);
                w.write_i64(*id);
                w.write_i64(access_hash.0);
                w.write_bytes(file_reference);
            }
            InputDocument::Empty => {
                w.write_u32(INPUT_DOCUMENT_EMPTY);
                w.write_i64(0);
            }
        }
    }
}

impl Peer {
    /// TL-serialize this Peer. `None` is a client-side sentinel with no
    /// wire representation.
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

    pub fn user_id(&self) -> Option<UserId> {
        if let Peer::User { user_id } = self { Some(*user_id) } else { None }
    }

    pub fn chat_id(&self) -> Option<ChatId> {
        if let Peer::Chat { chat_id } = self { Some(*chat_id) } else { None }
    }

    pub fn channel_id(&self) -> Option<ChannelId> {
        if let Peer::Channel { channel_id } = self { Some(*channel_id) } else { None }
    }
}

impl Document {
    /// Download location for this document. `None` for empty/placeholder
    /// documents.
    pub fn location(&self) -> Option<FileLocation> {
        match self {
            Document::Document { id, access_hash, file_reference, dc_id, .. } => {
                Some(FileLocation::Document {
                    id: id.0,
                    access_hash: access_hash.0,
                    reference: file_reference.clone(),
                    thumb_size: String::new(),
                    dc_id: *dc_id,
                })
            }
            _ => None,
        }
    }
}

impl PhotoSize {
    /// Largest-pixel-dimension shorthand used by download callers.
    pub fn dimensions(&self) -> (i32, i32) {
        match self {
            PhotoSize::PhotoSize { w, h, .. } => (*w, *h),
            PhotoSize::PhotoSizeProgressive { w, h, .. } => (*w, *h),
            PhotoSize::PhotoCachedSize { w, h, .. } => (*w, *h),
            _ => (0, 0),
        }
    }
}
