//! Typed RPC method wrappers for the Telegram API.
//!
//! Each method builds the TL-serialized payload and provides typed
//! parsing of the response. These correspond to the full TL surface
//! listed in SPEC §7.
//!
//! # Methods implemented
//!
//! All constructor IDs verified against the published TL schema
//! (layer 223, core.telegram.org/schema/json) — see
//! `types/constructors.rs` for the constants.

use crate::error::{Error, Result};
use crate::serialize::{TLWriter, TLReader, RPC_ERROR, RPC_RESULT};
use crate::types::*;
use crate::serialize::VECTOR as TL_VECTOR;

// ===========================================================================
// Messages methods
// ===========================================================================

/// Build `messages.sendMessage` payload.
///
/// Schema (layer 223): `messages.sendMessage#545cd15a flags:# no_webpage:flags.1?true
/// silent:flags.5?true background:flags.6?true clear_draft:flags.7?true
/// noforwards:flags.14?true update_stickersets_order:flags.15?true
/// invert_media:flags.16?true allow_paid_floodskip:flags.19?true peer:InputPeer
/// reply_to:flags.0?InputReplyTo message:string random_id:long
/// reply_markup:flags.2?ReplyMarkup entities:flags.3?Vector<MessageEntity>
/// schedule_date:flags.10?int ...`
pub fn build_send_message(
    peer: &InputPeer,
    message: &str,
    reply_to_msg_id: Option<i64>,
    schedule_date: Option<i32>,
) -> Vec<u8> {
    let mut flags: i32 = 0;
    if reply_to_msg_id.is_some() { flags |= 1 << 0; } // reply_to:flags.0
    if schedule_date.is_some() { flags |= 1 << 10; }

    let mut w = TLWriter::new();
    w.write_u32(MESSAGES_SEND_MESSAGE);
    w.write_i32(flags);
    peer.write_to(&mut w);
    // reply_to:flags.0?InputReplyTo — sits between peer and message
    if let Some(reply_id) = reply_to_msg_id {
        // inputReplyToMessage#869fbe10 flags:# reply_to_msg_id:int
        w.write_u32(INPUT_REPLY_TO_MESSAGE);
        w.write_i32(0); // inner flags (no top_msg_id/quote/...)
        w.write_i32(reply_id as i32);
    }
    w.write_bytes(message.as_bytes());
    // random_id:long
    w.write_i64(rand::random::<i64>());
    // reply_markup:flags.2 (none) — omitted, flag not set
    // entities:flags.3 (none) — omitted, flag not set
    if let Some(date) = schedule_date {
        w.write_i32(date);
    }
    w.into_bytes()
}

/// Build `messages.getDialogs` payload.
/// Schema (layer 223): `messages.getDialogs#a0f4cb4f flags:# exclude_pinned:flags.0?true
/// folder_id:flags.1?int offset_date:int offset_id:int offset_peer:InputPeer
/// limit:int hash:long = messages.Dialogs;`
pub fn build_get_dialogs(
    offset_date: i32,
    offset_id: i32,
    limit: i32,
) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(MESSAGES_GET_DIALOGS);
    w.write_i32(0); // flags
    w.write_i32(offset_date);
    w.write_i32(offset_id);
    // InputPeerEmpty for offset_peer
    w.write_u32(INPUT_PEER_EMPTY);
    w.write_i32(limit);
    w.write_i64(0); // hash:long (no hash check)
    w.into_bytes()
}
/// Build `messages.getHistory` payload.
pub fn build_get_history(
    peer: &InputPeer,
    offset_id: i32,
    offset_date: i32,
    add_offset: i32,
    limit: i32,
    max_id: i32,
    min_id: i32,
) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(MESSAGES_GET_HISTORY);
    peer.write_to(&mut w);
    w.write_i32(offset_id);
    w.write_i32(offset_date);
    w.write_i32(add_offset);
    w.write_i32(limit);
    w.write_i32(max_id);
    w.write_i32(min_id);
    w.write_i64(0); // hash:long
    w.into_bytes()
}

/// Build `messages.getMessages` payload.
pub fn build_get_messages(msg_ids: &[MsgId]) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(MESSAGES_GET_MESSAGES);
    // Vector<int> of message IDs
    w.write_u32(TL_VECTOR);
    w.write_i32(msg_ids.len() as i32);
    for id in msg_ids {
        w.write_i32(id.0 as i32);
    }
    w.into_bytes()
}

/// Build `messages.deleteMessages` payload.
pub fn build_delete_messages(msg_ids: &[MsgId], revoke: bool) -> Vec<u8> {
    let flags: i32 = if revoke { 1 << 0 } else { 0 };
    let mut w = TLWriter::new();
    w.write_u32(MESSAGES_DELETE_MESSAGES);
    w.write_i32(flags);
    // Vector<int> of message IDs
    w.write_u32(TL_VECTOR);
    w.write_i32(msg_ids.len() as i32);
    for id in msg_ids {
        w.write_i32(id.0 as i32);
    }
    w.into_bytes()
}

/// Build `messages.editMessage` payload.
pub fn build_edit_message(
    peer: &InputPeer,
    msg_id: i32,
    message: Option<&str>,
) -> Vec<u8> {
    let flags: i32 = if message.is_some() { 1 << 11 } else { 0 };
    let mut w = TLWriter::new();
    w.write_u32(MESSAGES_EDIT_MESSAGE);
    w.write_i32(flags);
    peer.write_to(&mut w);
    w.write_i32(msg_id);
    if let Some(text) = message {
        w.write_bytes(text.as_bytes());
    }
    w.into_bytes()
}

/// Build `messages.readHistory` payload.
pub fn build_read_history(peer: &InputPeer, max_id: i32) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(MESSAGES_READ_HISTORY);
    peer.write_to(&mut w);
    w.write_i32(max_id);
    w.into_bytes()
}

/// Build `messages.search` payload.
pub fn build_search(
    peer: &InputPeer,
    query: &str,
    limit: i32,
) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(MESSAGES_SEARCH);
    w.write_i32(0); // flags (no from_id / top_msg_id)
    peer.write_to(&mut w);
    w.write_bytes(query.as_bytes());
    // InputMessagesFilterEmpty (filter is required)
    w.write_u32(0x57e2f66c);
    w.write_i32(0); // min_date
    w.write_i32(0); // max_date
    w.write_i32(0); // offset_id
    w.write_i32(0); // add_offset
    w.write_i32(limit);
    w.write_i32(0); // max_id
    w.write_i32(0); // min_id
    w.write_i64(0); // hash:long
    w.into_bytes()
}

/// Build `messages.getBotCallbackAnswer` payload.
///
/// Schema (Layer 223): `messages.getBotCallbackAnswer#9342ca07 flags:#
/// game:flags.1?true peer:InputPeer msg_id:int data:flags.0?bytes
/// password:flags.2?InputCheckPasswordSRP = messages.BotCallbackAnswer;`
pub fn build_get_bot_callback_answer(
    peer: &InputPeer,
    msg_id: i32,
    data: &[u8],
) -> Vec<u8> {
    let flags: i32 = if data.is_empty() { 0 } else { 1 << 0 };
    let mut w = TLWriter::new();
    w.write_u32(MESSAGES_GET_BOT_CALLBACK_ANSWER);
    w.write_i32(flags);
    peer.write_to(&mut w);
    w.write_i32(msg_id);
    if !data.is_empty() {
        w.write_bytes(data);
    }
    w.into_bytes()
}


// ===========================================================================
// Users methods
// ===========================================================================

/// Build `users.getFullUser` payload.
pub fn build_get_full_user(user: &InputUser) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(USERS_GET_FULL_USER);
    user.write_to(&mut w);
    w.into_bytes()
}

/// Build `users.getUsers` payload.
pub fn build_get_users(users: &[InputUser]) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(USERS_GET_USERS);
    // Vector<InputUser>
    w.write_u32(TL_VECTOR);
    w.write_i32(users.len() as i32);
    for user in users {
        user.write_to(&mut w);
    }
    w.into_bytes()
}

// ===========================================================================
// Contacts methods
// ===========================================================================

/// Build `contacts.resolveUsername` payload.
pub fn build_resolve_username(username: &str) -> Vec<u8> {
    // contacts.resolveUsername#725afbbc flags:# username:string
    //   referer:flags.0?string
    let mut w = TLWriter::new();
    w.write_u32(CONTACTS_RESOLVE_USERNAME);
    w.write_i32(0); // flags (no referer)
    w.write_bytes(username.as_bytes());
    w.into_bytes()
}

// ===========================================================================
// Channels methods
// ===========================================================================

/// Build `channels.createChannel` payload.
pub fn build_create_channel(
    title: &str,
    about: &str,
    broadcast: bool,
    megagroup: bool,
) -> Vec<u8> {
    // channels.createChannel#91006707: broadcast:flags.0, megagroup:flags.1
    let mut flags: i32 = 0;
    if broadcast { flags |= 1 << 0; }
    if megagroup { flags |= 1 << 1; }

    let mut w = TLWriter::new();
    w.write_u32(CHANNELS_CREATE_CHANNEL);
    w.write_i32(flags);
    w.write_bytes(title.as_bytes());
    w.write_bytes(about.as_bytes()); // about is required in layer 223
    w.into_bytes()
}

/// Build `channels.inviteToChannel` payload.
pub fn build_invite_to_channel(
    channel: &InputChannel,
    users: &[InputUser],
) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(CHANNELS_INVITE_TO_CHANNEL);
    channel.write_to(&mut w);
    // Vector<InputUser>
    w.write_u32(TL_VECTOR);
    w.write_i32(users.len() as i32);
    for user in users {
        user.write_to(&mut w);
    }
    w.into_bytes()
}

/// Build `channels.editAdmin` payload.
pub fn build_edit_admin(
    channel: &InputChannel,
    user_id: &InputUser,
    admin_rights: i32,
    rank: &str,
) -> Vec<u8> {
    // channels.editAdmin#9a98ad68 flags:# channel:InputChannel
    //   user_id:InputUser admin_rights:ChatAdminRights rank:flags.0?string
    let mut flags: i32 = 0;
    if !rank.is_empty() {
        flags |= 1 << 0;
    }
    let mut w = TLWriter::new();
    w.write_u32(CHANNELS_EDIT_ADMIN);
    w.write_i32(flags);
    channel.write_to(&mut w);
    user_id.write_to(&mut w);
    // ChatAdminRights#5fb224d5 flags:# (rights bit-mask)
    w.write_u32(crate::types::CHAT_ADMIN_RIGHTS);
    w.write_i32(admin_rights);
    if !rank.is_empty() {
        w.write_bytes(rank.as_bytes());
    }
    w.into_bytes()
}

/// Build `channels.getChannels` payload.
pub fn build_get_channels(channels: &[InputChannel]) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(CHANNELS_GET_CHANNELS);
    // Vector<InputChannel>
    w.write_u32(TL_VECTOR);
    w.write_i32(channels.len() as i32);
    for ch in channels {
        ch.write_to(&mut w);
    }
    w.into_bytes()
}

/// Build `channels.leaveChannel` payload.
pub fn build_leave_channel(channel: &InputChannel) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(CHANNELS_LEAVE_CHANNEL);
    channel.write_to(&mut w);
    w.into_bytes()
}

// ===========================================================================
// Upload methods
// ===========================================================================

/// Build `upload.saveFilePart` payload.
pub fn build_save_file_part(
    file_id: i64,
    file_part: i32,
    data: &[u8],
) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(UPLOAD_SAVE_FILE_PART);
    w.write_i64(file_id);
    w.write_i32(file_part);
    w.write_bytes(data);
    w.into_bytes()
}

/// Build `upload.saveBigFilePart` payload.
pub fn build_save_big_file_part(
    file_id: i64,
    file_part: i32,
    file_total_parts: i32,
    data: &[u8],
) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(UPLOAD_SAVE_BIG_FILE_PART);
    w.write_i64(file_id);
    w.write_i32(file_part);
    w.write_i32(file_total_parts);
    w.write_bytes(data);
    w.into_bytes()
}

/// Build `upload.getFile` payload.
///
/// Schema: `upload.getFile#be5335be flags:# precise:flags.0?true
/// cdn_supported:flags.1?true location:InputFileLocation offset:long
/// limit:int = upload.File;`
pub fn build_get_file(
    location: &FileLocation,
    offset: i64,
    limit: i32,
) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(UPLOAD_GET_FILE);
    w.write_i32(0); // flags (no precise / cdn_supported)
    // Write the location as InputFileLocation
    match location {
        FileLocation::VolumeId { volume_id, local_id, secret, reference, dc_id: _ } => {
            w.write_u32(0xdfdaabe1); // inputFileLocation
            w.write_i64(*volume_id);
            w.write_i32(*local_id);
            w.write_i64(*secret);
            w.write_bytes(reference);
        }
        FileLocation::Document { id, access_hash, reference, thumb_size, dc_id: _ } => {
            w.write_u32(0xbad07584); // inputDocumentFileLocation
            w.write_i64(*id);
            w.write_i64(*access_hash);
            w.write_bytes(reference);
            w.write_bytes(thumb_size.as_bytes());
        }
        _ => {
            // Unsupported location type — write empty
            w.write_u32(0);
        }
    }
    w.write_i64(offset);
    w.write_i32(limit);
    w.into_bytes()
}

// ===========================================================================
// Media / photo input types (SPEC §7)
// ===========================================================================

/// `inputPhoto#3bb3b94a id:long access_hash:long file_reference:bytes`
#[derive(Debug, Clone)]
pub struct InputPhoto {
    pub id: i64,
    pub access_hash: i64,
    pub file_reference: Vec<u8>,
}

impl InputPhoto {
    fn write_to(&self, w: &mut TLWriter) {
        w.write_u32(INPUT_PHOTO);
        w.write_i64(self.id);
        w.write_i64(self.access_hash);
        w.write_bytes(&self.file_reference);
    }
}

/// Media payload for `messages.sendMedia` (subset of `InputMedia` the
/// library supports without a full TL generator).
#[derive(Debug, Clone)]
pub enum InputMedia {
    /// `inputMediaEmpty#9664f57f`
    Empty,
    /// `inputMediaContact#f8ab7dfb`
    Contact {
        phone_number: String,
        first_name: String,
        last_name: String,
        vcard: String,
    },
    /// `inputMediaGeoPoint#f9c44144` wrapping `inputGeoPoint#48222faf`.
    GeoPoint { lat: f64, long: f64 },
}

impl InputMedia {
    fn write_to(&self, w: &mut TLWriter) {
        match self {
            InputMedia::Empty => {
                w.write_u32(INPUT_MEDIA_EMPTY);
            }
            InputMedia::Contact { phone_number, first_name, last_name, vcard } => {
                w.write_u32(INPUT_MEDIA_CONTACT);
                w.write_bytes(phone_number.as_bytes());
                w.write_bytes(first_name.as_bytes());
                w.write_bytes(last_name.as_bytes());
                w.write_bytes(vcard.as_bytes());
            }
            InputMedia::GeoPoint { lat, long } => {
                w.write_u32(INPUT_MEDIA_GEO_POINT);
                w.write_i32(0); // flags (no accuracy_radius)
                // TL `double` is 8-byte IEEE 754 big-endian on the wire.
                w.write_double(*lat);
                w.write_double(*long);
            }
        }
    }
}

/// Write an `inputReplyToMessage#869fbe10` (shared by send* builders).
fn write_reply_to(w: &mut TLWriter, reply_to_msg_id: i64) {
    w.write_u32(INPUT_REPLY_TO_MESSAGE);
    w.write_i32(0); // inner flags (no top_msg_id/quote/...)
    w.write_i32(reply_to_msg_id as i32);
}

// ===========================================================================
// Messages: media + history-cleanup methods (SPEC §7)
// ===========================================================================

/// Build `messages.sendMedia` payload.
///
/// Schema (Layer 223): `messages.sendMedia#330e77f flags:# silent:flags.5?true
/// background:flags.6?true clear_draft:flags.7?true noforwards:flags.14?true
/// ... peer:InputPeer reply_to:flags.0?InputReplyTo media:InputMedia
/// message:string random_id:long reply_markup:flags.2?ReplyMarkup
/// entities:flags.3?Vector<MessageEntity> schedule_date:flags.10?int ...`
#[allow(clippy::too_many_arguments)]
pub fn build_send_media(
    peer: &InputPeer,
    media: &InputMedia,
    message: &str,
    reply_to_msg_id: Option<i64>,
    silent: bool,
    clear_draft: bool,
    schedule_date: Option<i32>,
) -> Vec<u8> {
    let mut flags: i32 = 0;
    if reply_to_msg_id.is_some() { flags |= 1 << 0; }
    if silent { flags |= 1 << 5; }
    if clear_draft { flags |= 1 << 7; }
    if schedule_date.is_some() { flags |= 1 << 10; }

    let mut w = TLWriter::new();
    w.write_u32(MESSAGES_SEND_MEDIA);
    w.write_i32(flags);
    peer.write_to(&mut w);
    if let Some(reply_id) = reply_to_msg_id {
        write_reply_to(&mut w, reply_id);
    }
    media.write_to(&mut w);
    w.write_bytes(message.as_bytes());
    w.write_i64(rand::random::<i64>()); // random_id
    if let Some(date) = schedule_date {
        w.write_i32(date);
    }
    w.into_bytes()
}

/// One entry of `messages.sendMultiMedia`'s `multi_media` vector.
pub struct InputSingleMedia {
    pub media: InputMedia,
    pub message: String,
}

/// Build `messages.sendMultiMedia` payload.
///
/// Schema (Layer 223): `messages.sendMultiMedia#1bf89d74 flags:# ...
/// peer:InputPeer reply_to:flags.0?InputReplyTo
/// multi_media:Vector<InputSingleMedia> schedule_date:flags.10?int ...`
pub fn build_send_multi_media(
    peer: &InputPeer,
    items: &[InputSingleMedia],
    reply_to_msg_id: Option<i64>,
    silent: bool,
    clear_draft: bool,
    schedule_date: Option<i32>,
) -> Vec<u8> {
    let mut flags: i32 = 0;
    if reply_to_msg_id.is_some() { flags |= 1 << 0; }
    if silent { flags |= 1 << 5; }
    if clear_draft { flags |= 1 << 7; }
    if schedule_date.is_some() { flags |= 1 << 10; }

    let mut w = TLWriter::new();
    w.write_u32(MESSAGES_SEND_MULTI_MEDIA);
    w.write_i32(flags);
    peer.write_to(&mut w);
    if let Some(reply_id) = reply_to_msg_id {
        write_reply_to(&mut w, reply_id);
    }
    // Vector<InputSingleMedia>
    w.write_u32(TL_VECTOR);
    w.write_u32(items.len() as u32);
    for item in items {
        w.write_u32(INPUT_SINGLE_MEDIA);
        w.write_i32(0); // inner flags (no entities)
        item.media.write_to(&mut w);
        w.write_i64(rand::random::<i64>()); // random_id
        w.write_bytes(item.message.as_bytes());
    }
    if let Some(date) = schedule_date {
        w.write_i32(date);
    }
    w.into_bytes()
}

/// Build `messages.deleteHistory` payload.
///
/// Schema (Layer 223): `messages.deleteHistory#b08f922a flags:#
/// just_clear:flags.0?true revoke:flags.1?true peer:InputPeer max_id:int
/// min_date:flags.2?int max_date:flags.3?int = messages.AffectedHistory;`
pub fn build_delete_history(
    peer: &InputPeer,
    max_id: i32,
    just_clear: bool,
    revoke: bool,
) -> Vec<u8> {
    let mut flags: i32 = 0;
    if just_clear { flags |= 1 << 0; }
    if revoke { flags |= 1 << 1; }

    let mut w = TLWriter::new();
    w.write_u32(MESSAGES_DELETE_HISTORY);
    w.write_i32(flags);
    peer.write_to(&mut w);
    w.write_i32(max_id);
    w.into_bytes()
}

// ===========================================================================
// Contacts methods
// ===========================================================================

/// Build `contacts.resolvePhone` payload.
///
/// Schema (Layer 223): `contacts.resolvePhone#8af94344 phone:string
/// = contacts.ResolvedPeer;`
pub fn build_resolve_phone(phone: &str) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(CONTACTS_RESOLVE_PHONE);
    w.write_bytes(phone.as_bytes());
    w.into_bytes()
}

/// Build `contacts.search` payload.
///
/// Schema (Layer 223): `contacts.search#11f812d8 q:string limit:int
/// = contacts.Found;`
pub fn build_contacts_search(q: &str, limit: i32) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(CONTACTS_SEARCH);
    w.write_bytes(q.as_bytes());
    w.write_i32(limit);
    w.into_bytes()
}

// ===========================================================================
// Channels methods
// ===========================================================================

/// Participant filter for `channels.getParticipants`.
pub enum ChannelParticipantsFilter {
    /// `channelParticipantsRecent#de3f3c79`
    Recent,
    /// `channelParticipantsSearch#656ac4b q:string`
    Search(String),
}

/// Build `channels.getParticipants` payload.
///
/// Schema (Layer 223): `channels.getParticipants#77ced9d0
/// channel:InputChannel filter:ChannelParticipantsFilter offset:int
/// limit:int hash:long = channels.ChannelParticipants;`
pub fn build_get_participants(
    channel: &InputChannel,
    filter: &ChannelParticipantsFilter,
    offset: i32,
    limit: i32,
    hash: i64,
) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(CHANNELS_GET_PARTICIPANTS);
    channel.write_to(&mut w);
    match filter {
        ChannelParticipantsFilter::Recent => {
            w.write_u32(CHANNEL_PARTICIPANTS_RECENT);
        }
        ChannelParticipantsFilter::Search(q) => {
            w.write_u32(CHANNEL_PARTICIPANTS_SEARCH);
            w.write_bytes(q.as_bytes());
        }
    }
    w.write_i32(offset);
    w.write_i32(limit);
    w.write_i64(hash);
    w.into_bytes()
}

/// Build an about-update payload for a channel/supergroup.
///
/// `channels.editAbout#13e27b46` was **removed** from the schema; the
/// modern call is `messages.editChatAbout#def60797 peer:InputPeer
/// about:string = Bool;` (works for channels and supergroups alike).
pub fn build_edit_about(peer: &InputPeer, about: &str) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(MESSAGES_EDIT_CHAT_ABOUT);
    peer.write_to(&mut w);
    w.write_bytes(about.as_bytes());
    w.into_bytes()
}

// ===========================================================================
// Upload methods
// ===========================================================================

/// Build `upload.getWebFile` payload.
///
/// Schema (Layer 223): `upload.getWebFile#24e6818d
/// location:InputWebFileLocation offset:int limit:int = upload.WebFile;`
pub fn build_get_web_file(url: &str, access_hash: i64, offset: i32, limit: i32) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(UPLOAD_GET_WEB_FILE);
    // inputWebFileLocation#c239d686 url:string access_hash:long
    w.write_u32(INPUT_WEB_FILE_LOCATION);
    w.write_bytes(url.as_bytes());
    w.write_i64(access_hash);
    w.write_i32(offset);
    w.write_i32(limit);
    w.into_bytes()
}

/// Build `upload.getCdnFile` payload.
///
/// Schema (Layer 223): `upload.getCdnFile#395f69da file_token:bytes
/// offset:long limit:int = upload.CdnFile;`
pub fn build_get_cdn_file(file_token: &[u8], offset: i64, limit: i32) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(UPLOAD_GET_CDN_FILE);
    w.write_bytes(file_token);
    w.write_i64(offset);
    w.write_i32(limit);
    w.into_bytes()
}

// ===========================================================================
// Photos methods
// ===========================================================================

/// Build `photos.updateProfilePhoto` payload.
///
/// Schema (Layer 223): `photos.updateProfilePhoto#9e82039 flags:#
/// fallback:flags.0?true bot:flags.1?InputUser id:InputPhoto = photos.Photo;`
pub fn build_update_profile_photo(fallback: bool, id: &InputPhoto) -> Vec<u8> {
    let mut flags: i32 = 0;
    if fallback { flags |= 1 << 0; }

    let mut w = TLWriter::new();
    w.write_u32(PHOTOS_UPDATE_PROFILE_PHOTO);
    w.write_i32(flags);
    id.write_to(&mut w);
    w.into_bytes()
}

/// Build `photos.uploadProfilePhoto` payload (static image variant).
///
/// Schema (Layer 223): `photos.uploadProfilePhoto#388a3b5 flags:#
/// fallback:flags.3?true bot:flags.5?InputUser file:flags.0?InputFile
/// video:flags.1?InputFile video_start_ts:flags.2?double
/// video_emoji_markup:flags.4?VideoSize = photos.Photo;`
pub fn build_upload_profile_photo(
    file: &crate::types::InputFile,
    fallback: bool,
) -> Vec<u8> {
    let mut flags: i32 = 0;
    if fallback { flags |= 1 << 3; }
    flags |= 1 << 0; // file is always set in this builder

    let mut w = TLWriter::new();
    w.write_u32(PHOTOS_UPLOAD_PROFILE_PHOTO);
    w.write_i32(flags);
    file.write_to(&mut w);
    w.into_bytes()
}

/// Build `photos.deletePhotos` payload.
///
/// Schema (Layer 223): `photos.deletePhotos#87cf7f2f id:Vector<InputPhoto>
/// = Vector<long>;`
pub fn build_delete_photos(photos: &[InputPhoto]) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(PHOTOS_DELETE_PHOTOS);
    w.write_u32(TL_VECTOR);
    w.write_u32(photos.len() as u32);
    for photo in photos {
        photo.write_to(&mut w);
    }
    w.into_bytes()
}

/// Build `photos.getUserPhotos` payload.
///
/// Schema (Layer 223): `photos.getUserPhotos#91cd32a8 user_id:InputUser
/// offset:int max_id:long limit:int = photos.Photos;`
pub fn build_get_user_photos(
    user: &InputUser,
    offset: i32,
    max_id: i64,
    limit: i32,
) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(PHOTOS_GET_USER_PHOTOS);
    user.write_to(&mut w);
    w.write_i32(offset);
    w.write_i64(max_id);
    w.write_i32(limit);
    w.into_bytes()
}

// ===========================================================================
// Updates difference methods (SPEC §6)
// ===========================================================================

/// Build `updates.getDifference` payload.
///
/// `pts`/`date`/`qts` = last known state; pass `None` pts to force a full
/// difference round from the server's stored state.
pub fn build_get_difference(pts: i32, date: i32, qts: i32) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(UPDATES_GET_DIFFERENCE);
    // flags:int = 0 (no pts_total_limit)
    w.write_i32(0);
    w.write_i32(pts);
    w.write_i32(date);
    w.write_i32(qts);
    w.into_bytes()
}

/// Build `updates.getState` payload.
pub fn build_get_state() -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(UPDATES_GET_STATE);
    w.into_bytes()
}
/// Build `updates.getChannelDifference` payload.
pub fn build_get_channel_difference(
    channel: &InputChannel,
    pts: i32,
    limit: i32,
) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(UPDATES_GET_CHANNEL_DIFFERENCE);
    // flags:int = 0 (no force)
    w.write_i32(0);
    channel.write_to(&mut w);
    w.write_i32(pts);
    w.write_i32(limit);
    w.into_bytes()
}


// ===========================================================================
// Help methods
// ===========================================================================

/// Build `help.getConfig` payload.
pub fn build_get_config() -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(HELP_GET_CONFIG);
    w.into_bytes()
}

/// Build `help.getNearestDc` payload.
pub fn build_get_nearest_dc() -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(HELP_GET_NEAREST_DC);
    w.into_bytes()
}

// ===========================================================================
// Response parsers
// ===========================================================================

/// Parse a `messages.Dialogs` response.
pub fn parse_dialogs(data: &[u8]) -> Result<Dialogs> {
    let mut r = TLReader::new(data);
    let ctor = r.read_u32()?;
    match ctor {
        MESSAGES_DIALOGS => {
            // Parse dialogs vector
            let _v_ctor = r.read_u32()?;
            let count = r.read_i32()?;
            let dialogs: Vec<Dialog> = Vec::new();
            for _ in 0..count {
                let _d_ctor = r.read_u32()?;
                // Simplified: skip dialog bytes
                while r.remaining() > 0 {
                    let _ = r.read_i32()?;
                }
            }
            Ok(Dialogs { dialogs, messages: Vec::new(), users: Vec::new(), chats: Vec::new() })
        }
        MESSAGES_DIALOGS_SLICE => {
            // Skip similarly
            while r.remaining() > 0 { let _ = r.read_i32()?; }
            Ok(Dialogs { dialogs: Vec::new(), messages: Vec::new(), users: Vec::new(), chats: Vec::new() })
        }
        RPC_ERROR => {
            let (code, msg) = crate::mtproto::parse_rpc_error(data)?;
            Err(Error::Rpc { error_code: code, error_message: msg })
        }
        _ => Err(Error::UnexpectedResponse(format!(
            "unexpected constructor {ctor:#x} in getDialogs response"
        ))),
    }
}

/// Parse an RPC result wrapper, extracting the inner result bytes.
pub fn parse_rpc_result_inner(data: &[u8]) -> Result<Vec<u8>> {
    let mut r = TLReader::new(data);
    let ctor = r.read_u32()?;
    match ctor {
        RPC_RESULT => {
            let _req_msg_id = r.read_u64()?;
            Ok(data[r.position()..].to_vec())
        }
        _ => Ok(data.to_vec()), // might already be unwrapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_send_message() {
        let peer = InputPeer::UserFromId { user_id: UserId(123) };
        let payload = build_send_message(&peer, "hello", None, None);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), MESSAGES_SEND_MESSAGE);
        let flags = r.read_i32().unwrap();
        assert_eq!(flags, 0);
        assert_eq!(r.read_u32().unwrap(), INPUT_PEER_USER_FROM_ID);
        assert_eq!(r.read_i64().unwrap(), 123);
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "hello");
        // random_id (i64) must be present
        let _random_id = r.read_i64().unwrap();
    }

    #[test]
    fn test_build_send_message_reply_layout() {
        // reply_to must sit between peer and message, serialized as
        // inputReplyToMessage#869fbe10 flags:# reply_to_msg_id:int
        let peer = InputPeer::UserFromId { user_id: UserId(1) };
        let payload = build_send_message(&peer, "hi", Some(42), None);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), MESSAGES_SEND_MESSAGE);
        assert_eq!(r.read_i32().unwrap(), 1 << 0); // reply_to flag
        assert_eq!(r.read_u32().unwrap(), INPUT_PEER_USER_FROM_ID);
        assert_eq!(r.read_i64().unwrap(), 1);
        assert_eq!(r.read_u32().unwrap(), INPUT_REPLY_TO_MESSAGE);
        assert_eq!(r.read_i32().unwrap(), 0); // inner flags
        assert_eq!(r.read_i32().unwrap(), 42); // reply_to_msg_id
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "hi");
        let _random_id = r.read_i64().unwrap();
    }

    #[test]
    fn test_build_delete_messages() {
        let ids = vec![MsgId(1), MsgId(2), MsgId(3)];
        let payload = build_delete_messages(&ids, false);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), MESSAGES_DELETE_MESSAGES);
        assert_eq!(r.read_i32().unwrap(), 0); // flags
        assert_eq!(r.read_u32().unwrap(), VECTOR);
        assert_eq!(r.read_i32().unwrap(), 3);
        assert_eq!(r.read_i32().unwrap(), 1);
        assert_eq!(r.read_i32().unwrap(), 2);
        assert_eq!(r.read_i32().unwrap(), 3);
    }

    #[test]
    fn test_build_get_dialogs() {
        let payload = build_get_dialogs(0, 0, 10);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), MESSAGES_GET_DIALOGS);
        assert_eq!(r.read_i32().unwrap(), 0); // flags
        assert_eq!(r.read_i32().unwrap(), 0); // offset_date
        assert_eq!(r.read_i32().unwrap(), 0); // offset_id
        assert_eq!(r.read_u32().unwrap(), INPUT_PEER_EMPTY); // offset_peer
        assert_eq!(r.read_i32().unwrap(), 10); // limit
        assert_eq!(r.read_i64().unwrap(), 0); // hash
    }

    #[test]
    fn test_build_create_channel() {
        let payload = build_create_channel("My Channel", "About", true, false);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), CHANNELS_CREATE_CHANNEL);
        let flags = r.read_i32().unwrap();
        assert_eq!(flags, 1 << 0); // broadcast (flags.0 in layer 223)
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "My Channel");
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "About");
    }

    #[test]
    fn test_build_resolve_username() {
        let payload = build_resolve_username("testbot");
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), CONTACTS_RESOLVE_USERNAME);
        assert_eq!(r.read_i32().unwrap(), 0); // flags (no referer)
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "testbot");
    }

    #[test]
    fn test_build_get_config() {
        let payload = build_get_config();
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), HELP_GET_CONFIG);
    }

    #[test]
    fn test_build_get_file() {
        let loc = FileLocation::VolumeId {
            volume_id: 12345,
            local_id: 1,
            secret: 67890,
            reference: vec![1, 2, 3],
            dc_id: 2,
        };
        let payload = build_get_file(&loc, 0, 1024 * 1024);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), UPLOAD_GET_FILE);
    }

    #[test]
    fn test_build_save_file_part() {
        let data = vec![0u8; 512 * 1024]; // 512KB chunk
        let payload = build_save_file_part(12345, 0, &data);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), UPLOAD_SAVE_FILE_PART);
        assert_eq!(r.read_i64().unwrap(), 12345);
        assert_eq!(r.read_i32().unwrap(), 0);
    }

    #[test]
    fn test_build_search() {
        let peer = InputPeer::UserFromId { user_id: UserId(1) };
        let payload = build_search(&peer, "hello", 10);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), MESSAGES_SEARCH);
    }

    #[test]
    fn test_parse_rpc_result_inner() {
        let mut w = TLWriter::new();
        w.write_u32(RPC_RESULT);
        w.write_u64(0x1234);
        w.write_u32(BOOL_TRUE);
        let data = w.into_bytes();
        let inner = parse_rpc_result_inner(&data).unwrap();
        assert_eq!(inner, vec![BOOL_TRUE as u8, 0x75, 0x72, 0x99]); // BOOL_TRUE in LE
    }

    #[test]
    fn test_build_send_media_layout() {
        let peer = InputPeer::UserFromId { user_id: UserId(7) };
        let media = InputMedia::Contact {
            phone_number: "+15551234".into(),
            first_name: "A".into(),
            last_name: "B".into(),
            vcard: String::new(),
        };
        let payload = build_send_media(&peer, &media, "cap", Some(5), true, true, Some(99));
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), MESSAGES_SEND_MEDIA);
        let flags = r.read_i32().unwrap();
        assert_eq!(flags, (1 << 0) | (1 << 5) | (1 << 7) | (1 << 10));
        assert_eq!(r.read_u32().unwrap(), INPUT_PEER_USER_FROM_ID);
        assert_eq!(r.read_i64().unwrap(), 7);
        // reply_to
        assert_eq!(r.read_u32().unwrap(), INPUT_REPLY_TO_MESSAGE);
        assert_eq!(r.read_i32().unwrap(), 0);
        assert_eq!(r.read_i32().unwrap(), 5);
        // inputMediaContact
        assert_eq!(r.read_u32().unwrap(), INPUT_MEDIA_CONTACT);
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "+15551234");
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "A");
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "B");
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "");
        // message + random_id + schedule_date
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "cap");
        let _random_id = r.read_i64().unwrap();
        assert_eq!(r.read_i32().unwrap(), 99);
    }

    #[test]
    fn test_build_send_media_geo_point_uses_double() {
        let peer = InputPeer::UserFromId { user_id: UserId(1) };
        let media = InputMedia::GeoPoint { lat: 1.5, long: -2.5 };
        let payload = build_send_media(&peer, &media, "", None, false, false, None);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), MESSAGES_SEND_MEDIA);
        assert_eq!(r.read_i32().unwrap(), 0);
        assert_eq!(r.read_u32().unwrap(), INPUT_PEER_USER_FROM_ID);
        assert_eq!(r.read_i64().unwrap(), 1);
        assert_eq!(r.read_u32().unwrap(), INPUT_MEDIA_GEO_POINT);
        assert_eq!(r.read_i32().unwrap(), 0); // inner flags (no accuracy_radius)
        let lat = f64::from_bits(r.read_u64().unwrap());
        let long = f64::from_bits(r.read_u64().unwrap());
        assert_eq!(lat, 1.5);
        assert_eq!(long, -2.5);
        // message
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "");
    }

    #[test]
    fn test_build_send_multi_media_layout() {
        let peer = InputPeer::UserFromId { user_id: UserId(3) };
        let items = vec![
            InputSingleMedia { media: InputMedia::Empty, message: "one".into() },
            InputSingleMedia { media: InputMedia::Empty, message: "two".into() },
        ];
        let payload = build_send_multi_media(&peer, &items, None, false, false, None);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), MESSAGES_SEND_MULTI_MEDIA);
        assert_eq!(r.read_i32().unwrap(), 0); // flags
        assert_eq!(r.read_u32().unwrap(), INPUT_PEER_USER_FROM_ID);
        assert_eq!(r.read_i64().unwrap(), 3);
        assert_eq!(r.read_u32().unwrap(), VECTOR);
        assert_eq!(r.read_i32().unwrap(), 2);
        for want in ["one", "two"] {
            assert_eq!(r.read_u32().unwrap(), INPUT_SINGLE_MEDIA);
            assert_eq!(r.read_i32().unwrap(), 0); // inner flags
            assert_eq!(r.read_u32().unwrap(), INPUT_MEDIA_EMPTY);
            let _random_id = r.read_i64().unwrap();
            assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), want);
        }
    }

    #[test]
    fn test_build_delete_history_layout() {
        let peer = InputPeer::UserFromId { user_id: UserId(9) };
        let payload = build_delete_history(&peer, 77, false, true);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), MESSAGES_DELETE_HISTORY);
        assert_eq!(r.read_i32().unwrap(), 1 << 1); // revoke
        assert_eq!(r.read_u32().unwrap(), INPUT_PEER_USER_FROM_ID);
        assert_eq!(r.read_i64().unwrap(), 9);
        assert_eq!(r.read_i32().unwrap(), 77); // max_id
        // min_date/max_date omitted (flags unset)
    }

    #[test]
    fn test_build_resolve_phone_and_search() {
        let payload = build_resolve_phone("+15550001");
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), CONTACTS_RESOLVE_PHONE);
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "+15550001");

        let payload = build_contacts_search("query", 25);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), CONTACTS_SEARCH);
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "query");
        assert_eq!(r.read_i32().unwrap(), 25);
    }

    #[test]
    fn test_build_get_participants_layout() {
        let filter = ChannelParticipantsFilter::Search("abc".into());
        let channel = InputChannel::Channel { channel_id: ChannelId(55), access_hash: AccessHash(666) };
        let payload = build_get_participants(&channel, &filter, 10, 20, 7);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), CHANNELS_GET_PARTICIPANTS);
        assert_eq!(r.read_u32().unwrap(), INPUT_CHANNEL);
        assert_eq!(r.read_i64().unwrap(), 55);
        assert_eq!(r.read_i64().unwrap(), 666);
        assert_eq!(r.read_u32().unwrap(), CHANNEL_PARTICIPANTS_SEARCH);
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "abc");
        assert_eq!(r.read_i32().unwrap(), 10);
        assert_eq!(r.read_i32().unwrap(), 20);
        assert_eq!(r.read_i64().unwrap(), 7);

        // Recent filter serializes as a bare constructor
        let recent = ChannelParticipantsFilter::Recent;
        let payload = build_get_participants(&channel, &recent, 0, 1, 0);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), CHANNELS_GET_PARTICIPANTS);
        assert_eq!(r.read_u32().unwrap(), INPUT_CHANNEL);
        let _ = r.read_i64().unwrap();
        let _ = r.read_i64().unwrap();
        assert_eq!(r.read_u32().unwrap(), CHANNEL_PARTICIPANTS_RECENT);
    }

    #[test]
    fn test_build_edit_about_layout() {
        let peer = InputPeer::Channel { channel_id: ChannelId(5), access_hash: AccessHash(6) };
        let payload = build_edit_about(&peer, "hello world");
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), MESSAGES_EDIT_CHAT_ABOUT);
        assert_eq!(r.read_u32().unwrap(), INPUT_PEER_CHANNEL);
        assert_eq!(r.read_i64().unwrap(), 5);
        assert_eq!(r.read_i64().unwrap(), 6);
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "hello world");
    }

    #[test]
    fn test_build_get_web_file_layout() {
        let payload = build_get_web_file("https://example.com", 42, 100, 200);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), UPLOAD_GET_WEB_FILE);
        assert_eq!(r.read_u32().unwrap(), INPUT_WEB_FILE_LOCATION);
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "https://example.com");
        assert_eq!(r.read_i64().unwrap(), 42);
        assert_eq!(r.read_i32().unwrap(), 100);
        assert_eq!(r.read_i32().unwrap(), 200);
    }

    #[test]
    fn test_build_get_cdn_file_layout() {
        let payload = build_get_cdn_file(&[0xDE, 0xAD], 4096, 1024);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), UPLOAD_GET_CDN_FILE);
        assert_eq!(r.read_bytes().unwrap(), vec![0xDE, 0xAD]);
        assert_eq!(r.read_i64().unwrap(), 4096);
        assert_eq!(r.read_i32().unwrap(), 1024);
    }

    #[test]
    fn test_build_update_profile_photo_layout() {
        let photo = InputPhoto { id: 1, access_hash: 2, file_reference: vec![3, 4] };
        let payload = build_update_profile_photo(false, &photo);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), PHOTOS_UPDATE_PROFILE_PHOTO);
        assert_eq!(r.read_i32().unwrap(), 0);
        assert_eq!(r.read_u32().unwrap(), INPUT_PHOTO);
        assert_eq!(r.read_i64().unwrap(), 1);
        assert_eq!(r.read_i64().unwrap(), 2);
        assert_eq!(r.read_bytes().unwrap(), vec![3, 4]);

        // fallback flag set
        let payload = build_update_profile_photo(true, &photo);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), PHOTOS_UPDATE_PROFILE_PHOTO);
        assert_eq!(r.read_i32().unwrap(), 1 << 0);
    }

    #[test]
    fn test_build_upload_profile_photo_layout() {
        use crate::types::InputFile as TlInputFile;
        let file = TlInputFile::Big { id: 10, parts: 2, name: "photo.jpg".into() };
        let payload = build_upload_profile_photo(&file, false);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), PHOTOS_UPLOAD_PROFILE_PHOTO);
        assert_eq!(r.read_i32().unwrap(), 1 << 0); // file flag always set
        // input file follows — constructor written by InputFile::write_to
    }

    #[test]
    fn test_build_delete_photos_layout() {
        let photos = vec![
            InputPhoto { id: 1, access_hash: 2, file_reference: vec![] },
            InputPhoto { id: 3, access_hash: 4, file_reference: vec![] },
        ];
        let payload = build_delete_photos(&photos);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), PHOTOS_DELETE_PHOTOS);
        assert_eq!(r.read_u32().unwrap(), VECTOR);
        assert_eq!(r.read_i32().unwrap(), 2);
        for (id, hash) in [(1, 2), (3, 4)] {
            assert_eq!(r.read_u32().unwrap(), INPUT_PHOTO);
            assert_eq!(r.read_i64().unwrap(), id);
            assert_eq!(r.read_i64().unwrap(), hash);
            assert_eq!(r.read_bytes().unwrap(), Vec::<u8>::new());
        }
    }

    #[test]
    fn test_build_get_user_photos_layout() {
        let user = InputUser::FromId { user_id: UserId(8) };
        let payload = build_get_user_photos(&user, 4, 999, 10);
        let mut r = TLReader::new(&payload);
        assert_eq!(r.read_u32().unwrap(), PHOTOS_GET_USER_PHOTOS);
        assert_eq!(r.read_u32().unwrap(), INPUT_USER_FROM_ID);
        assert_eq!(r.read_i64().unwrap(), 8);
        assert_eq!(r.read_i32().unwrap(), 4);
        assert_eq!(r.read_i64().unwrap(), 999);
        assert_eq!(r.read_i32().unwrap(), 10);
    }
}
