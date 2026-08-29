//! TL type library for the Telegram API.
//!
//! Defines typed wrappers for IDs, all input/output types, and
//! serialization helpers. Every constructor matches the official
//! TL schema used by Telegram.
//!
//! This module gates the entire higher-level API surface: the Client,
//! SenderPool, and every RPC call all depend on these types.

use crate::error::{Error, Result};
use crate::serialize::{TLWriter, TLReader};
use std::fmt;

// ===========================================================================
// §5. Newtype ID wrappers (DX-5 from spec §12.1)
// ===========================================================================

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(pub i64);

        impl From<i64> for $name {
            fn from(v: i64) -> Self { Self(v) }
        }

        impl From<$name> for i64 {
            fn from(v: $name) -> Self { v.0 }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

define_id!(
    /// Telegram user ID. Distinct from ChannelId even though both wrap i64.
    UserId
);
define_id!(
    /// Telegram chat (group) ID.
    ChatId
);
define_id!(
    /// Telegram channel/supergroup ID.
    ChannelId
);
define_id!(
    /// Access hash — required to interact with users/channels the client doesn't share a chat with.
    AccessHash
);
define_id!(
    /// Message ID — uniquely identifies a message within a chat.
    MsgId
);
define_id!(
    /// Photo ID.
    PhotoId
);
define_id!(
    /// Document/file ID.
    DocumentId
);
define_id!(
    /// File reference — used to authorize file downloads; expires over time.
    FileRef
);

// ===========================================================================
// §7 Input types (peer/user/channel references for API calls)
// ===========================================================================

/// A reference to a chat participant that can be used in API calls.
#[derive(Debug, Clone)]
pub enum InputPeer {
    /// Reference to a user, requires access_hash.
    User { user_id: UserId, access_hash: AccessHash },
    /// Reference to a chat (basic group).
    Chat { chat_id: ChatId },
    /// Reference to a channel/supergroup, requires access_hash.
    Channel { channel_id: ChannelId, access_hash: AccessHash },
    /// Reference to yourself.
    Self_,

    /// Legacy: user by ID, resolved server-side.
    UserFromId { user_id: UserId },
}

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
            InputPeer::UserFromId { user_id } => {
                w.write_u32(INPUT_PEER_USER_FROM_ID);
                w.write_i64(user_id.0);
            }
        }
    }

    /// Parse an InputPeer from a TL reader.
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            INPUT_PEER_USER => {
                let user_id = r.read_i64()?;
                let access_hash = r.read_i64()?;
                Ok(InputPeer::User {
                    user_id: UserId(user_id),
                    access_hash: AccessHash(access_hash),
                })
            }
            INPUT_PEER_CHAT => {
                let chat_id = r.read_i64()?;
                Ok(InputPeer::Chat { chat_id: ChatId(chat_id) })
            }
            INPUT_PEER_CHANNEL => {
                let channel_id = r.read_i64()?;
                let access_hash = r.read_i64()?;
                Ok(InputPeer::Channel {
                    channel_id: ChannelId(channel_id),
                    access_hash: AccessHash(access_hash),
                })
            }
            INPUT_PEER_SELF => Ok(InputPeer::Self_),
            INPUT_PEER_USER_FROM_ID => {
                let user_id = r.read_i64()?;
                Ok(InputPeer::UserFromId { user_id: UserId(user_id) })
            }
            other => Err(Error::Serialization(format!(
                "unknown InputPeer constructor {other:#x}"
            ))),
        }
    }
}

/// A reference to a specific user for API calls.
#[derive(Debug, Clone)]
pub enum InputUser {
    /// User with access hash.
    User { user_id: UserId, access_hash: AccessHash },
    /// Yourself.
    Self_,
    /// Legacy: user by ID only.
    FromId { user_id: UserId },
}

impl InputUser {
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
            InputUser::FromId { user_id } => {
                w.write_u32(INPUT_USER_FROM_ID);
                w.write_i64(user_id.0);
            }
        }
    }

    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            INPUT_USER => {
                let user_id = r.read_i64()?;
                let access_hash = r.read_i64()?;
                Ok(InputUser::User {
                    user_id: UserId(user_id),
                    access_hash: AccessHash(access_hash),
                })
            }
            INPUT_USER_SELF => Ok(InputUser::Self_),
            INPUT_USER_FROM_ID => {
                let user_id = r.read_i64()?;
                Ok(InputUser::FromId { user_id: UserId(user_id) })
            }
            other => Err(Error::Serialization(format!(
                "unknown InputUser constructor {other:#x}"
            ))),
        }
    }
}

/// A reference to a channel/supergroup for API calls.
#[derive(Debug, Clone)]
pub enum InputChannel {
    /// inputChannel#f35aec28 channel_id:long access_hash:long
    Channel { channel_id: ChannelId, access_hash: AccessHash },
}
impl InputChannel {
    pub fn write_to(&self, w: &mut TLWriter) {
        match self {
            InputChannel::Channel { channel_id, access_hash } => {
                w.write_u32(INPUT_CHANNEL);
                w.write_i64(channel_id.0);
                w.write_i64(access_hash.0);
            }
        }
    }

    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            INPUT_CHANNEL => {
                let channel_id = r.read_i64()?;
                let access_hash = r.read_i64()?;
                Ok(InputChannel::Channel {
                    channel_id: ChannelId(channel_id),
                    access_hash: AccessHash(access_hash),
                })
            }
            other => Err(Error::Serialization(format!(
                "unknown InputChannel constructor {other:#x}"
            ))),
        }
    }
}

// ===========================================================================
// §7 File input types
// ===========================================================================

/// Reference to a file for upload.
#[derive(Debug, Clone)]
pub enum InputFile {
    /// File from local disk.
    Id { id: i64, parts: i32, name: String, md5_checksum: Option<String> },
    /// Large file from local disk (>10MB, sent in parts).
    Big { id: i64, parts: i32, name: String },
    /// Partial file from CDN.
    Cd { id: i64, file_chain_id: i32, file_chain_part: i32 },
    /// File from a URL.
    Url { url: String },
}

impl InputFile {
    pub fn write_to(&self, w: &mut TLWriter) {
        match self {
            InputFile::Id { id, parts, name, md5_checksum } => {
                // inputFile#f52ff27f id:long parts:int name:string md5_checksum:string
                // md5_checksum is UNCONDITIONAL (no flags field).
                w.write_u32(INPUT_FILE);
                w.write_i64(*id);
                w.write_i32(*parts);
                w.write_bytes(name.as_bytes());
                w.write_bytes(md5_checksum.as_deref().unwrap_or("").as_bytes());
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

/// Reference to a file for download (the document attachment on a message).
#[derive(Debug, Clone)]
pub enum InputDocument {
    /// Standard document reference.
    Document { id: DocumentId, access_hash: AccessHash, file_reference: Vec<u8> },
    /// Empty/missing document.
    Empty,
}

impl InputDocument {
    pub fn write_to(&self, w: &mut TLWriter) {
        match self {
            InputDocument::Document { id, access_hash, file_reference } => {
                w.write_u32(INPUT_DOCUMENT);
                w.write_i64(id.0);
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

// ===========================================================================
// §7 Reply markup types
// ===========================================================================

/// Reply markup for messages (inline keyboards, reply keyboards, etc.).
#[derive(Debug, Clone)]
pub enum ReplyMarkup {
    /// No special markup.
    None,
    /// Force reply (forces the client to show a reply UI).
    ForceReply { selective: bool },
    /// Inline keyboard buttons.
    InlineKeyboard { rows: Vec<Vec<KeyboardButton>> },
    /// Reply keyboard (shown above the input field).
    ReplyKeyboard {
        rows: Vec<Vec<KeyboardButton>>,
        resize: bool,
        single_use: bool,
        selective: bool,
        persistent: bool,
    },
}

impl ReplyMarkup {
    pub fn write_to(&self, w: &mut TLWriter) {
        match self {
            ReplyMarkup::None => {}
            ReplyMarkup::ForceReply { selective } => {
                let flags: i32 = if *selective { 1 << 2 } else { 0 };
                w.write_u32(FORCE_REPLY);
                w.write_i32(flags);
            }
            ReplyMarkup::InlineKeyboard { rows } => {
                w.write_u32(inline_keyboard_markup::CONSTRUCTOR_ID);
                w.write_u32(VECTOR);
                w.write_i32(rows.len() as i32);
                for row in rows {
                    w.write_u32(VECTOR);
                    w.write_i32(row.len() as i32);
                    for btn in row {
                        btn.write_to(w);
                    }
                }
            }
            ReplyMarkup::ReplyKeyboard { rows, resize, single_use, selective, persistent } => {
                let mut flags: i32 = 0;
                if *resize { flags |= 1 << 0; }
                if *single_use { flags |= 1 << 1; }
                if *selective { flags |= 1 << 2; }
                if *persistent { flags |= 1 << 4; }
                w.write_u32(REPLY_KEYBOARD_MARKUP);
                w.write_i32(flags);
                w.write_u32(VECTOR);
                w.write_i32(rows.len() as i32);
                for row in rows {
                    w.write_u32(VECTOR);
                    w.write_i32(row.len() as i32);
                    for btn in row {
                        btn.write_to(w);
                    }
                }
            }
        }
    }
}

/// A keyboard button.
#[derive(Debug, Clone)]
pub enum KeyboardButton {
    Text { text: String },
    Url { text: String, url: String },
    Callback { text: String, data: Vec<u8> },
    // Simplified — full surface has ~15 variants
}

impl KeyboardButton {
    pub fn write_to(&self, w: &mut TLWriter) {
        match self {
            KeyboardButton::Text { text } => {
                w.write_u32(KEYBOARD_BUTTON);
                w.write_bytes(text.as_bytes());
            }
            KeyboardButton::Url { text, url } => {
                w.write_u32(KEYBOARD_BUTTON_URL);
                w.write_bytes(text.as_bytes());
                w.write_bytes(url.as_bytes());
            }
            KeyboardButton::Callback { text, data } => {
                w.write_u32(KEYBOARD_BUTTON_CALLBACK);
                w.write_bytes(text.as_bytes());
                w.write_bytes(data);
            }
        }
    }
}

// ===========================================================================
// §7 User types
// ===========================================================================

/// A Telegram user.
// The full-user variant is inherently wide; boxing would complicate every
// match for no real-world win — the enum is built once per response.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum User {
    /// Full user with all fields.
    User {
        id: UserId,
        access_hash: Option<AccessHash>,
        first_name: Option<String>,
        last_name: Option<String>,
        username: Option<String>,
        phone: Option<String>,
        photo: Option<ProfilePhoto>,
        status: Option<UserStatus>,
        bot: bool,
        min: bool,
        scam: bool,
        fake: bool,
    },
    /// Empty user (deleted or unknown).
    Empty {
        id: UserId,
    },
}

impl User {
    pub fn id(&self) -> UserId {
        match self {
            User::User { id, .. } | User::Empty { id } => *id,
        }
    }

    pub fn access_hash(&self) -> Option<AccessHash> {
        match self {
            User::User { access_hash, .. } => *access_hash,
            User::Empty { .. } => None,
        }
    }

    pub fn username(&self) -> Option<&str> {
        match self {
            User::User { username, .. } => username.as_deref(),
            User::Empty { .. } => None,
        }
    }

    pub fn first_name(&self) -> Option<&str> {
        match self {
            User::User { first_name, .. } => first_name.as_deref(),
            User::Empty { .. } => None,
        }
    }

    pub fn phone(&self) -> Option<&str> {
        match self {
            User::User { phone, .. } => phone.as_deref(),
            User::Empty { .. } => None,
        }
    }

    pub fn is_bot(&self) -> bool {
        match self {
            User::User { bot, .. } => *bot,
            User::Empty { .. } => false,
        }
    }

    /// Parse a User from a TL reader.
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            USER => {
                // user#31774388 flags:# ... flags2:# ... id:long ...
                // The flags2 word is ALWAYS serialized (before id) — it
                // keys its own conditional fields.
                let flags = r.read_i32()?;
                let _flags2 = r.read_i32()?;
                let id = UserId(r.read_i64()?);
                let access_hash = if flags & (1 << 0) != 0 {
                    Some(AccessHash(r.read_i64()?))
                } else {
                    None
                };
                let first_name = if flags & (1 << 1) != 0 {
                    Some(String::from_utf8(r.read_bytes()?)?)
                } else {
                    None
                };
                let last_name = if flags & (1 << 2) != 0 {
                    Some(String::from_utf8(r.read_bytes()?)?)
                } else {
                    None
                };
                let username = if flags & (1 << 3) != 0 {
                    Some(String::from_utf8(r.read_bytes()?)?)
                } else {
                    None
                };
                let phone = if flags & (1 << 4) != 0 {
                    Some(String::from_utf8(r.read_bytes()?)?)
                } else {
                    None
                };
                let photo = if flags & (1 << 5) != 0 {
                    Some(ProfilePhoto::read_from(r)?)
                } else {
                    None
                };
                let status = if flags & (1 << 6) != 0 {
                    Some(UserStatus::read_from(r)?)
                } else {
                    None
                };
                let bot = flags & (1 << 14) != 0;
                let min = flags & (1 << 20) != 0;
                let scam = flags & (1 << 24) != 0;
                let fake = flags & (1 << 25) != 0;

                Ok(User::User {
                    id, access_hash, first_name, last_name,
                    username, phone, photo, status,
                    bot, min, scam, fake,
                })
            }
            USER_EMPTY => {
                let id = UserId(r.read_i64()?);
                Ok(User::Empty { id })
            }
            other => Err(Error::Serialization(format!(
                "unknown User constructor {other:#x}"
            ))),
        }
    }
}

/// User online status.
#[derive(Debug, Clone)]
pub enum UserStatus {
    Online { expires: i32 },
    Offline { was_online: i32 },
    Recently,
    LastWeek,
    LastMonth,
    Empty,
}

impl UserStatus {
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            USER_STATUS_OFFLINE => {
                let was_online = r.read_i32()?;
                Ok(UserStatus::Offline { was_online })
            }
            USER_STATUS_ONLINE => {
                let expires = r.read_i32()?;
                Ok(UserStatus::Online { expires })
            }
            USER_STATUS_RECENTLY => Ok(UserStatus::Recently),
            USER_STATUS_LAST_WEEK => Ok(UserStatus::LastWeek),
            USER_STATUS_LAST_MONTH => Ok(UserStatus::LastMonth),
            USER_STATUS_EMPTY => Ok(UserStatus::Empty),
            other => Err(Error::Serialization(format!(
                "unknown UserStatus constructor {other:#x}"
            ))),
        }
    }
}

/// Profile photo.
#[derive(Debug, Clone)]
pub enum ProfilePhoto {
    Photo {
        photo_id: PhotoId,
        dc_id: i32,
    },
    Empty,
}

impl ProfilePhoto {
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            USER_PROFILE_PHOTO | CHAT_PHOTO => {
                let photo_id = PhotoId(r.read_i64()?);
                let _volume_id = r.read_i64()?;
                let _dc_id = r.read_i32()?;
                Ok(ProfilePhoto::Photo { photo_id, dc_id: 0 })
            }
            USER_PROFILE_PHOTO_EMPTY | CHAT_PHOTO_EMPTY => Ok(ProfilePhoto::Empty),
            other => Err(Error::Serialization(format!(
                "unknown ProfilePhoto constructor {other:#x}"
            ))),
        }
    }
}

// ===========================================================================
// §7 Chat / Channel types
// ===========================================================================

/// A Telegram chat (basic group, supergroup, or channel).
#[derive(Debug, Clone)]
pub enum Chat {
    /// Basic group chat.
    Chat {
        id: ChatId,
        title: String,
        photo: Option<ProfilePhoto>,
        participants_count: i32,
        date: i32,
        version: i32,
        creator: bool,
        kicked: bool,
        left: bool,
        deactivated: bool,
    },
    /// Empty/deleted chat.
    Empty { id: ChatId },
    /// Channel or supergroup.
    Channel {
        id: ChannelId,
        access_hash: Option<AccessHash>,
        title: String,
        username: Option<String>,
        photo: Option<ProfilePhoto>,
        date: i32,
        version: i32,
        megagroup: bool,
        broadcast: bool,
        verified: bool,
        scam: bool,
        fake: bool,
        left: bool,
        signature_names_default: bool,
        admin_rights: Option<ChatAdminRights>,
        banned_rights: Option<ChatBannedRights>,
    },
    /// Forbidden channel.
    ChannelForbidden {
        id: ChannelId,
        access_hash: AccessHash,
        title: String,
        broadcast: bool,
        megagroup: bool,
    },
}

impl Chat {
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            CHAT => {
                let flags = r.read_i32()?;
                let id = ChatId(r.read_i64()?);
                let title = String::from_utf8(r.read_bytes()?)?;
                let photo = if flags & (1 << 3) != 0 {
                    Some(ProfilePhoto::read_from(r)?)
                } else {
                    None
                };
                let participants_count = r.read_i32()?;
                let date = r.read_i32()?;
                let version = r.read_i32()?;
                let creator = flags & (1 << 0) != 0;
                let kicked = flags & (1 << 1) != 0;
                let left = flags & (1 << 2) != 0;
                let deactivated = flags & (1 << 5) != 0;
                Ok(Chat::Chat {
                    id, title, photo, participants_count, date, version,
                    creator, kicked, left, deactivated,
                })
            }
            CHAT_EMPTY => {
                let id = ChatId(r.read_i64()?);
                Ok(Chat::Empty { id })
            }
            CHANNEL => {
                let flags = r.read_i32()?;
                let id = ChannelId(r.read_i64()?);
                let access_hash = if flags & (1 << 0) != 0 {
                    Some(AccessHash(r.read_i64()?))
                } else {
                    None
                };
                let title = String::from_utf8(r.read_bytes()?)?;
                let username = if flags & (1 << 6) != 0 {
                    Some(String::from_utf8(r.read_bytes()?)?)
                } else {
                    None
                };
                let photo = if flags & (1 << 2) != 0 {
                    Some(ProfilePhoto::read_from(r)?)
                } else {
                    None
                };
                let date = r.read_i32()?;
                let version = r.read_i32()?;
                let megagroup = flags & (1 << 8) != 0;
                let broadcast = flags & (1 << 5) != 0;
                let verified = flags & (1 << 7) != 0;
                let scam = flags & (1 << 13) != 0;
                let fake = flags & (1 << 24) != 0;
                let left = flags & (1 << 12) != 0;
                let signature_names_default = flags & (1 << 25) != 0;
                // admin_rights and banned_rights only in recent layers
                Ok(Chat::Channel {
                    id, access_hash, title, username, photo, date, version,
                    megagroup, broadcast, verified, scam, fake, left,
                    signature_names_default,
                    admin_rights: None,
                    banned_rights: None,
                })
            }
            CHANNEL_FORBIDDEN => {
                let flags = r.read_i32()?;
                let id = ChannelId(r.read_i64()?);
                let access_hash = AccessHash(r.read_i64()?);
                let title = String::from_utf8(r.read_bytes()?)?;
                let broadcast = flags & (1 << 5) != 0;
                let megagroup = flags & (1 << 8) != 0;
                Ok(Chat::ChannelForbidden {
                    id, access_hash, title, broadcast, megagroup,
                })
            }
            other => Err(Error::Serialization(format!(
                "unknown Chat constructor {other:#x}"
            ))),
        }
    }
}

/// Channel admin rights.
#[derive(Debug, Clone, Default)]
pub struct ChatAdminRights {
    pub flags: i32,
}

/// Channel banned rights.
#[derive(Debug, Clone, Default)]
pub struct ChatBannedRights {
    pub flags: i32,
    pub until_date: i32,
}

// ===========================================================================
// §7 Message types
// ===========================================================================

/// A Telegram message.
#[derive(Debug, Clone)]
pub enum Message {
    /// Full message.
    Message(Box<MessageFull>),
    /// Empty message (deleted or not found).
    Empty { id: MsgId },
    /// Service message (channel actions, etc.).
    Service {
        id: MsgId,
        from_id: Option<Peer>,
        peer_id: Peer,
        date: i32,
        action: MessageAction,
        reply_to: Option<ReplyHeader>,
    },
}

/// Payload of [`Message::Message`] — boxed to keep the enum small.
#[derive(Debug, Clone)]
pub struct MessageFull {
    pub id: MsgId,
    pub from_id: Option<Peer>,
    pub peer_id: Peer,
    pub date: i32,
    pub message: String,
    pub media: Option<MessageMedia>,
    pub reply_markup: Option<ReplyMarkup>,
    pub entities: Vec<MessageEntity>,
    pub views: Option<i32>,
    pub edit_date: Option<i32>,
    pub post: bool,
    pub grouped_id: Option<i64>,
    pub via_bot_id: Option<UserId>,
    pub reply_to: Option<ReplyHeader>,
    pub edit_hide: bool,
}

impl Message {
    pub fn id(&self) -> MsgId {
        match self {
            Message::Message(full) => full.id,
            Message::Empty { id } | Message::Service { id, .. } => *id,
        }
    }

    pub fn text(&self) -> &str {
        match self {
            Message::Message(full) => &full.message,
            _ => "",
        }
    }

    pub fn peer_id(&self) -> &Peer {
        match self {
            Message::Message(full) => &full.peer_id,
            Message::Service { peer_id, .. } => peer_id,
            Message::Empty { .. } => &Peer::None,
        }
    }

    pub fn from_id(&self) -> Option<&Peer> {
        match self {
            Message::Message(full) => full.from_id.as_ref(),
            Message::Service { from_id, .. } => from_id.as_ref(),
            _ => None,
        }
    }

    pub fn media(&self) -> Option<&MessageMedia> {
        match self {
            Message::Message(full) => full.media.as_ref(),
            _ => None,
        }
    }

    pub fn parse_from_bytes(data: &[u8]) -> Result<Self> {
        let mut r = TLReader::new(data);
        Self::read_from(&mut r)
    }

    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            MESSAGE => {
                let flags = r.read_i32()?;
                let id = MsgId(r.read_i64()?);
                let from_id = if flags & (1 << 8) != 0 {
                    Some(Peer::read_from(r)?)
                } else {
                    None
                };
                let peer_id = Peer::read_from(r)?;
                let date = r.read_i32()?;
                let message_text = String::from_utf8(r.read_bytes()?)?;
                let media = if flags & (1 << 9) != 0 {
                    Some(MessageMedia::read_from(r)?)
                } else {
                    None
                };
                let reply_markup = if flags & (1 << 6) != 0 {
                    // TODO: parse reply_markup
                    None
                } else {
                    None
                };
                let entities = if flags & (1 << 7) != 0 {
                    // TODO: parse entities vector
                    let _ = r.read_u32()?; // vector ctor
                    let _ = r.read_i32()?; // count
                    Vec::new()
                } else {
                    Vec::new()
                };
                let views = if flags & (1 << 10) != 0 {
                    Some(r.read_i32()?)
                } else {
                    None
                };
                let edit_date = if flags & (1 << 11) != 0 {
                    Some(r.read_i32()?)
                } else {
                    None
                };
                let post = flags & (1 << 14) != 0;
                let grouped_id = if flags & (1 << 13) != 0 {
                    Some(r.read_i64()?)
                } else {
                    None
                };
                let via_bot_id = if flags & (1 << 11) != 0 {
                    Some(UserId(r.read_i64()?))
                } else {
                    None
                };
                let reply_to = if flags & (1 << 0) != 0 {
                    Some(ReplyHeader::read_from(r)?)
                } else {
                    None
                };
                let edit_hide = flags & (1 << 21) != 0;
                Ok(Message::Message(Box::new(MessageFull {
                    id, from_id, peer_id, date, message: message_text,
                    media, reply_markup, entities, views, edit_date,
                    post, grouped_id, via_bot_id, reply_to, edit_hide,
                })))
            }
            MESSAGE_EMPTY => {
                let id = MsgId(r.read_i64()?);
                Ok(Message::Empty { id })
            }
            MESSAGE_SERVICE => {
                let id = MsgId(r.read_i64()?);
                let from_id = if true { // simplified flag check
                    Some(Peer::read_from(r)?)
                } else {
                    None
                };
                let peer_id = Peer::read_from(r)?;
                let date = r.read_i32()?;
                let action = MessageAction::read_from(r)?;
                Ok(Message::Service {
                    id, from_id, peer_id, date, action,
                    reply_to: None,
                })
            }
            other => Err(Error::Serialization(format!(
                "unknown Message constructor {other:#x}"
            ))),
        }
    }
}

/// Reply header (reply_to_msg_id, reply_to_peer_id, etc.).
#[derive(Debug, Clone)]
pub struct ReplyHeader {
    pub reply_to_msg_id: MsgId,
    pub reply_to_peer_id: Option<Peer>,
    pub reply_to_top_id: Option<MsgId>,
}

impl ReplyHeader {
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let flags = r.read_i32()?;
        let reply_to_msg_id = MsgId(r.read_i64()?);
        let reply_to_peer_id = if flags & (1 << 0) != 0 {
            Some(Peer::read_from(r)?)
        } else {
            None
        };
        let reply_to_top_id = if flags & (1 << 1) != 0 {
            Some(MsgId(r.read_i64()?))
        } else {
            None
        };
        Ok(ReplyHeader {
            reply_to_msg_id,
            reply_to_peer_id,
            reply_to_top_id,
        })
    }
}

/// Message action (service messages).
#[derive(Debug, Clone)]
pub enum MessageAction {
    Empty,
    MessageActionChatCreate { title: String, users: Vec<UserId> },
    MessageActionChatEditTitle { title: String },
    MessageActionChatAddUser { users: Vec<UserId> },
    MessageActionChatDeleteUser { user_id: UserId },
    MessageActionChatJoinedByLink { inviter_id: UserId, via_link: bool },
    MessageActionChannelCreate { title: String },
    MessageActionPinMessage,
    MessageActionHistoryClear,
    MessageActionGameScore { game_id: i64, score: i32 },
    MessageActionPaymentSentMe {
        currency: String,
        total_amount: i64,
        invoice_slug: String,
    },
    MessageActionPaymentSent {
        currency: String,
        total_amount: i64,
    },
    Other,
}

impl MessageAction {
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            MESSAGE_ACTION_EMPTY => Ok(MessageAction::Empty),
            MESSAGE_ACTION_HISTORY_CLEAR => Ok(MessageAction::MessageActionHistoryClear),
            MESSAGE_ACTION_PIN_MESSAGE => Ok(MessageAction::MessageActionPinMessage),
            _ => {
                // Skip unknown action: consume remaining bytes
                while r.remaining() > 0 {
                    let _ = r.read_i32()?;
                }
                Ok(MessageAction::Other)
            }
        }
    }
}

/// Message media (attachments on a message).
#[derive(Debug, Clone)]
pub enum MessageMedia {
    None,
    Photo { photo: Photo },
    Geo { geo: GeoPoint },
    Contact { user_id: UserId, first_name: String, last_name: String, phone_number: String, vcard: String },
    Document { document: Document, caption: String },
    WebPage { webpage: WebPage },
    VoiceCall {},
    Game { game: String },
    Poll {},
    Dice { value: i32, emoticon: String },
    Unsupported,
}

impl MessageMedia {
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            MESSAGE_MEDIA_EMPTY => Ok(MessageMedia::None),
            MESSAGE_MEDIA_PHOTO => {
                let flags = r.read_i32()?;
                if flags & (1 << 2) != 0 {
                    let _ttl_seconds = r.read_i32()?;
                }
                let photo = Photo::read_from(r)?;
                let _caption = if flags & (1 << 7) != 0 {
                    String::from_utf8(r.read_bytes()?)?
                } else {
                    String::new()
                };
                Ok(MessageMedia::Photo { photo })
            }
            MESSAGE_MEDIA_DOCUMENT => {
                let flags = r.read_i32()?;
                let document = if flags & (1 << 2) != 0 {
                    Some(Document::read_from(r)?)
                } else {
                    None
                };
                let caption = if flags & (1 << 7) != 0 {
                    String::from_utf8(r.read_bytes()?)?
                } else {
                    String::new()
                };
                Ok(MessageMedia::Document {
                    document: document.unwrap_or(Document::Empty { id: DocumentId(0), access_hash: AccessHash(0), file_reference: Vec::new() }),
                    caption,
                })
            }
            MESSAGE_MEDIA_WEB_PAGE => {
                let webpage = WebPage::read_from(r)?;
                Ok(MessageMedia::WebPage { webpage })
            }
            MESSAGE_MEDIA_GEO => {
                let geo = GeoPoint::read_from(r)?;
                Ok(MessageMedia::Geo { geo })
            }
            MESSAGE_MEDIA_DICE => {
                let value = r.read_i32()?;
                let emoticon = String::from_utf8(r.read_bytes()?)?;
                Ok(MessageMedia::Dice { value, emoticon })
            }
            MESSAGE_MEDIA_UNSUPPORTED => Ok(MessageMedia::Unsupported),
            _ => {
                // Skip unknown media
                while r.remaining() > 0 {
                    let _ = r.read_i32()?;
                }
                Ok(MessageMedia::None)
            }
        }
    }
}

/// A TL message entity (bold, italic, links, etc.).
#[derive(Debug, Clone)]
pub struct MessageEntity {
    pub offset: i32,
    pub length: i32,
    pub kind: MessageEntityType,
}

/// Message entity type.
#[derive(Debug, Clone)]
pub enum MessageEntityType {
    Unknown(u32),
}

// ===========================================================================
// §7 Photo, Document, and related types
// ===========================================================================

/// Telegram photo.
#[derive(Debug, Clone)]
pub enum Photo {
    Photo {
        id: PhotoId,
        access_hash: AccessHash,
        file_reference: Vec<u8>,
        dates: PhotoDateInfo,
        sizes: Vec<PhotoSize>,
    },
    Empty { id: PhotoId },
}

impl Photo {
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            PHOTO => {
                let _flags = r.read_i32()?;
                let id = PhotoId(r.read_i64()?);
                let access_hash = AccessHash(r.read_i64()?);
                let file_reference = r.read_bytes()?;
                let _dc_id = r.read_i32()?;
                let _w = r.read_i32()?;
                let _h = r.read_i32()?;
                // Skip remaining fields for now
                while r.remaining() > 0 {
                    let _ = r.read_i32()?;
                }
                Ok(Photo::Photo {
                    id, access_hash, file_reference,
                    dates: PhotoDateInfo::default(),
                    sizes: Vec::new(),
                })
            }
            PHOTO_EMPTY => {
                let id = PhotoId(r.read_i64()?);
                Ok(Photo::Empty { id })
            }
            other => Err(Error::Serialization(format!(
                "unknown Photo constructor {other:#x}"
            ))),
        }
    }
}

/// Photo date info (simplified).
#[derive(Debug, Clone, Default)]
pub struct PhotoDateInfo {
    pub date: i32,
}

/// Photo size variants.
#[derive(Debug, Clone)]
pub enum PhotoSize {
    Size { type_: String, location: FileLocation, w: i32, h: i32, size: i32 },
    Cached { type_: String, location: FileLocation, size: i32 },
    Stripped { type_: String, bytes: Vec<u8> },
    Empty { type_: String },
}

/// File location reference.
#[derive(Debug, Clone)]
pub enum FileLocation {
    VolumeId { volume_id: i64, local_id: i32, secret: i64, reference: Vec<u8>, dc_id: i32 },
    Web { dc_id: i32, url: String, size: i32 },
    EmojiStickerSet { version: i32, set_id: i64 },
    Unknown,
}

/// Telegram document (files, stickers, etc.).
#[derive(Debug, Clone)]
pub enum Document {
    Document {
        id: DocumentId,
        access_hash: AccessHash,
        file_reference: Vec<u8>,
        date: i32,
        mime_type: String,
        size: i64,
        thumb: Option<PhotoSize>,
        dc_id: i32,
        version: i32,
    },
    Empty { id: DocumentId, access_hash: AccessHash, file_reference: Vec<u8> },
}

impl Document {
    pub fn id(&self) -> DocumentId {
        match self {
            Document::Document { id, .. } | Document::Empty { id, .. } => *id,
        }
    }

    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            DOCUMENT => {
                let _flags = r.read_i32()?;
                let id = DocumentId(r.read_i64()?);
                let access_hash = AccessHash(r.read_i64()?);
                let file_reference = r.read_bytes()?;
                let date = r.read_i32()?;
                let mime_type = String::from_utf8(r.read_bytes()?)?;
                let size = r.read_i64()?;
                let _thumb = None; // simplified
                let _dc_id = r.read_i32()?;
                let _version = r.read_i32()?;
                // Skip remaining fields
                while r.remaining() > 0 {
                    let _ = r.read_i32()?;
                }
                Ok(Document::Document {
                    id, access_hash, file_reference, date, mime_type, size,
                    thumb: _thumb, dc_id: _dc_id, version: _version,
                })
            }
            DOCUMENT_EMPTY => {
                let id = DocumentId(r.read_i64()?);
                Ok(Document::Empty { id, access_hash: AccessHash(0), file_reference: Vec::new() })
            }
            other => Err(Error::Serialization(format!(
                "unknown Document constructor {other:#x}"
            ))),
        }
    }
}

/// Web page preview.
#[derive(Debug, Clone)]
pub enum WebPage {
    Empty { id: i64 },
    WebPage { id: i64, url: String, display_type: String, description: Option<String> },
    Instant { id: i64, short_name: String, description: String },
}

impl WebPage {
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let _ctor = r.read_u32()?;
        let id = r.read_i64()?;
        while r.remaining() > 0 {
            let _ = r.read_i32()?;
        }
        Ok(WebPage::Empty { id })
    }
}

/// Geo point.
#[derive(Debug, Clone)]
pub struct GeoPoint {
    pub long: f64,
    pub lat: f64,
}

impl GeoPoint {
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let _flags = r.read_i32()?;
        let long_bits = r.read_u64()?;
        let lat_bits = r.read_u64()?;
        Ok(GeoPoint {
            long: f64::from_bits(long_bits),
            lat: f64::from_bits(lat_bits),
        })
    }
}

// ===========================================================================
// §7 Peer types
// ===========================================================================

/// A chat/user/channel peer reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Peer {
    User { user_id: UserId },
    Chat { chat_id: ChatId },
    Channel { channel_id: ChannelId },
    None,
}

impl Peer {
    pub fn user_id(&self) -> Option<UserId> {
        if let Peer::User { user_id } = self { Some(*user_id) } else { None }
    }

    pub fn channel_id(&self) -> Option<ChannelId> {
        if let Peer::Channel { channel_id } = self { Some(*channel_id) } else { None }
    }

    pub fn chat_id(&self) -> Option<ChatId> {
        if let Peer::Chat { chat_id } = self { Some(*chat_id) } else { None }
    }

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

    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            PEER_USER => {
                let user_id = r.read_i64()?;
                Ok(Peer::User { user_id: UserId(user_id) })
            }
            PEER_CHAT => {
                let chat_id = r.read_i64()?;
                Ok(Peer::Chat { chat_id: ChatId(chat_id) })
            }
            PEER_CHANNEL => {
                let channel_id = r.read_i64()?;
                Ok(Peer::Channel { channel_id: ChannelId(channel_id) })
            }
            other => Err(Error::Serialization(format!(
                "unknown Peer constructor {other:#x}"
            ))),
        }
    }
}

// ===========================================================================
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
// §7 API reply types
// ===========================================================================

/// Response to messages.sendMessage.
#[derive(Debug, Clone)]
pub enum SendMessageResult {
    /// Updates containing the sent message.
    Updates(Box<Updates>),
    /// Short sent message response (newer layers).
    ShortSentMessage {
        id: MsgId,
        pts: i32,
        pts_count: i32,
    },
}

/// Response to auth.sentCode.
#[derive(Debug, Clone)]
pub struct SentCode {
    pub phone_code_hash: String,
    pub code_type: SentCodeType,
    pub next_code_type: Option<SentCodeType>,
    pub timeout: Option<i32>,
}

/// Type of verification code sent.
#[derive(Debug, Clone)]
pub enum SentCodeType {
    App,
    Sms,
    Call,
    FlashCall,
    SmsCall,
    FragmentSms,
}

/// auth.authorization response.
#[derive(Debug, Clone)]
pub struct Authorization {
    pub user: User,
    pub dc_list: Option<Vec<i32>>,
    pub user_config: Option<i32>,
}

/// Bot callback answer.
#[derive(Debug, Clone)]
pub struct BotCallbackAnswer {
    pub message: Option<String>,
    pub alert: bool,
    pub url: Option<String>,
    pub cache_time: i32,
}

/// Peer settings (e.g., for report/spam).
#[derive(Debug, Clone, Default)]
pub struct PeerSettings {
    pub flags: i32,
}

// ===========================================================================
// §7 Constructor IDs
// ===========================================================================

// --- Input peer (Layer 223) ---
pub const INPUT_PEER_EMPTY: u32 = 0x7f3b18ea;
pub const INPUT_PEER_SELF: u32 = 0x7da07ec9;
pub const INPUT_PEER_USER: u32 = 0xdde8a54c;
pub const INPUT_PEER_USER_FROM_ID: u32 = 0xa87b0a1c;
pub const INPUT_PEER_CHAT: u32 = 0x35a95cb9;
pub const INPUT_PEER_CHANNEL: u32 = 0x27bcbbfc;
pub const INPUT_PEER_CHANNEL_FROM_ID: u32 = 0xbd2a0840;

// --- Input user (Layer 223) ---
pub const INPUT_USER_EMPTY: u32 = 0xb98886cf;
pub const INPUT_USER_SELF: u32 = 0xf7c1b13f;
pub const INPUT_USER: u32 = 0xf21158c6;
pub const INPUT_USER_FROM_ID: u32 = 0x1da448e2;

pub const INPUT_REPLY_TO_MESSAGE: u32 = 0x869fbe10;
pub const INPUT_REPLY_TO_MONOFORUM: u32 = 0x76ab27de;
// --- Input channel (Layer 223) ---
pub const INPUT_CHANNEL: u32 = 0xf35aec28;
pub const INPUT_CHANNEL_FROM_MESSAGE: u32 = 0x5b934f9d; // inputChannelFromMessage

// --- Input file (Layer 223) ---
pub const INPUT_FILE: u32 = 0xf52ff27f;
pub const INPUT_FILE_BIG: u32 = 0xfa4f0bb5;
pub const INPUT_FILE_STORY_DOCUMENT: u32 = 0x62dc8b48;

// --- Input document (Layer 223) ---
pub const INPUT_DOCUMENT: u32 = 0x1abfb575;
pub const INPUT_DOCUMENT_EMPTY: u32 = 0x72f0eaae; // inputDocumentEmpty
/// document#8fd32c0b (Layer 223)
pub const DOCUMENT: u32 = 0x8fd32c0b;
/// documentEmpty#3631cf4c id:long
pub const DOCUMENT_EMPTY: u32 = 0x3631cf4c;

// --- User (Layer 223) ---
pub const USER: u32 = 0x31774388;
pub const USER_EMPTY: u32 = 0xd3bc4b7a;

// --- User status (Layer 223) ---
pub const USER_STATUS_EMPTY: u32 = 0x9d05049;
pub const USER_STATUS_ONLINE: u32 = 0xedb93949;
pub const USER_STATUS_OFFLINE: u32 = 0x8c703f;
pub const USER_STATUS_RECENTLY: u32 = 0x7b197dc8;
pub const USER_STATUS_LAST_WEEK: u32 = 0x541a1d1a;
pub const USER_STATUS_LAST_MONTH: u32 = 0x65899777;

// --- Chat (Layer 223) ---
pub const CHAT: u32 = 0x41cbf256;
pub const CHAT_EMPTY: u32 = 0x29562865;
pub const CHAT_FORBIDDEN: u32 = 0x6592a1a7;
pub const CHAT_FULL: u32 = 0x2633421b;

// --- Channel (Layer 223) ---
pub const CHANNEL: u32 = 0x1c32b11c;
pub const CHANNEL_FORBIDDEN: u32 = 0x17d493d5;

// --- Photo/UserProfilePhoto/ChatPhoto (Layer 223) ---
pub const PHOTO_EMPTY: u32 = 0x2331b22d;
pub const PHOTO: u32 = 0xfb197a65;
pub const CHAT_PHOTO: u32 = 0x1c6e1c11;
pub const CHAT_PHOTO_EMPTY: u32 = 0x37c1011c;
pub const USER_PROFILE_PHOTO: u32 = 0x82d1f706;
pub const USER_PROFILE_PHOTO_EMPTY: u32 = 0x4f11bae1;

// --- Message (Layer 223) ---
pub const MESSAGE: u32 = 0x3ae56482;
pub const MESSAGE_EMPTY: u32 = 0x90a6ca84;
pub const MESSAGE_SERVICE: u32 = 0x7a800e0a;

// --- Message media (Layer 223) ---
pub const MESSAGE_MEDIA_EMPTY: u32 = 0x3ded6320;
pub const MESSAGE_MEDIA_PHOTO: u32 = 0x695150d7;
pub const MESSAGE_MEDIA_DOCUMENT: u32 = 0x52d8ccd9;
pub const MESSAGE_MEDIA_WEB_PAGE: u32 = 0xddf10c3b;
pub const MESSAGE_MEDIA_GEO: u32 = 0x56e0d474;
pub const MESSAGE_MEDIA_CONTACT: u32 = 0x70322949;
pub const MESSAGE_MEDIA_DICE: u32 = 0x08cbec07;
pub const MESSAGE_MEDIA_UNSUPPORTED: u32 = 0x9f84f49e;
pub const MESSAGE_MEDIA_GAME: u32 = 0xfdb19008;
pub const MESSAGE_MEDIA_POLL: u32 = 0x4bd6e798;
pub const MESSAGE_MEDIA_INVOICE: u32 = 0xf6a548d3;
pub const MESSAGE_MEDIA_STORY: u32 = 0x68cb6283;
pub const MESSAGE_MEDIA_GIVEAWAY: u32 = 0xaa073beb;
pub const MESSAGE_MEDIA_GIVEAWAY_RESULTS: u32 = 0xceaa3ea1;
pub const MESSAGE_MEDIA_PAID_MEDIA: u32 = 0xa8852491;

// --- Message action (Layer 223) ---
pub const MESSAGE_ACTION_EMPTY: u32 = 0xb6aef7b0;
pub const MESSAGE_ACTION_HISTORY_CLEAR: u32 = 0x9fbab604;
pub const MESSAGE_ACTION_CHAT_CREATE: u32 = 0xbd47cbad;
pub const MESSAGE_ACTION_CHAT_EDIT_TITLE: u32 = 0xb5a1ce5a;
pub const MESSAGE_ACTION_CHAT_ADD_USER: u32 = 0x15cefd00;
pub const MESSAGE_ACTION_CHAT_DELETE_USER: u32 = 0xa43f30cc;
pub const MESSAGE_ACTION_CHAT_JOINED_BY_LINK: u32 = 0x031224c3;
pub const MESSAGE_ACTION_CHANNEL_CREATE: u32 = 0x95d2ac92;
pub const MESSAGE_ACTION_PIN_MESSAGE: u32 = 0x94bd38ed;
pub const MESSAGE_ACTION_GAME_SCORE: u32 = 0x92a72876;

// --- Peer (Layer 223) ---
pub const PEER_USER: u32 = 0x59511722;
pub const PEER_CHAT: u32 = 0x36c6019a;
pub const PEER_CHANNEL: u32 = 0xa2a5371e;

// --- Updates (Layer 223) ---
pub const UPDATES: u32 = 0x74ae4240;
pub const UPDATE_SHORT: u32 = 0x78d4dec1; // TODO: verify from schema
pub const UPDATES_COMBINED: u32 = 0x725b04c3; // TODO: verify from schema
pub const UPDATE_SHORT_SENT_MESSAGE: u32 = 0x9015e101;

// --- Update events (Layer 223) ---
pub const UPDATE_NEW_MESSAGE: u32 = 0x1f2b0afd;
pub const UPDATE_DELETE_MESSAGES: u32 = 0xa20db0e5;
pub const UPDATE_READ_HISTORY_INBOX: u32 = 0x9e84bc99;
pub const UPDATE_READ_HISTORY_OUTBOX: u32 = 0x2f2f21bf;
pub const UPDATE_CHANNEL_TOO_LONG: u32 = 0x108d941f;
pub const UPDATE_EDIT_MESSAGE: u32 = 0xe40370a3;
pub const UPDATE_WEB_PAGE: u32 = 0x7f891213;
/// replyKeyboardMarkup#350284c2
pub const REPLY_KEYBOARD_MARKUP: u32 = 0x350284c2;
pub const FORCE_REPLY: u32 = 0x86872538;
pub mod inline_keyboard_markup { pub const CONSTRUCTOR_ID: u32 = 0x158b2380; }

// --- Keyboard buttons ---
pub const KEYBOARD_BUTTON: u32 = 0x683a5c46;
pub const KEYBOARD_BUTTON_URL: u32 = 0x258aff06;
pub const KEYBOARD_BUTTON_CALLBACK: u32 = 0x3250872a;
pub const KEYBOARD_BUTTON_SWITCH_INLINE: u32 = 0x063760c8;
pub const KEYBOARD_BUTTON_GAME: u32 = 0x568be74c;
pub const KEYBOARD_BUTTON_URL_AUTH: u32 = 0x10b78d29;
pub const KEYBOARD_BUTTON_REQUEST_PEER: u32 = 0xb1764226;

// --- Messages (Layer 223) ---
pub const MESSAGES_DIALOGS: u32 = 0x15ba6c40;
pub const MESSAGES_DIALOGS_SLICE: u32 = 0x71e094f3;
pub const MESSAGES_DIALOGS_NOT_MODIFIED: u32 = 0xf0e3e596;
pub const MESSAGES_MESSAGES: u32 = 0x1d73e7ea;
pub const MESSAGES_MESSAGES_SLICE: u32 = 0x5f206716;
pub const MESSAGES_CHANNEL_MESSAGES: u32 = 0xc776ba4e;
pub const MESSAGES_MESSAGES_NOT_MODIFIED: u32 = 0x74535f21;

// --- Dialog (Layer 223) ---
pub const DIALOG: u32 = 0xd58a08c6;
pub const DIALOG_FOLDER: u32 = 0x71bd134c;

// --- Sent code (Layer 223) ---
pub const AUTH_SENT_CODE: u32 = 0x5e002502;
pub const AUTH_SENT_CODE_SUCCESS: u32 = 0x2390fe44;
pub const AUTH_SENT_CODE_PAYMENT_REQUIRED: u32 = 0xe0955a3c;
pub const AUTH_SENT_CODE_TYPE_APP: u32 = 0x3dbb5986;
pub const AUTH_SENT_CODE_TYPE_SMS: u32 = 0xc004bac7;

// --- Auth (Layer 223) ---
pub const AUTH_AUTHORIZATION: u32 = 0x2ea2c0d4;
pub const AUTH_AUTHORIZATION_SIGN_UP_REQUIRED: u32 = 0x44747e9a;
pub const AUTH_LOG_OUT: u32 = 0x87971c3d; // TODO: verify

// --- Auth functions (Layer 223) ---
pub const AUTH_SEND_CODE: u32 = 0xa677244f;
pub const AUTH_SIGN_IN: u32 = 0x8d52a951; // TODO: verify
pub const AUTH_SIGN_UP: u32 = 0x80eead27; // TODO: verify
pub const AUTH_CHECK_PASSWORD: u32 = 0xd18b4d16; // TODO: verify
pub const IMPORT_BOT_AUTH: u32 = 0x67a3ff2c;

// --- Messages methods ---
pub const MESSAGES_SEND_MESSAGE: u32 = 0x545cd15a;
pub const MESSAGES_SEND_MEDIA: u32 = 0xb8d0afdf;
pub const MESSAGES_SEND_MULTI_MEDIA: u32 = 0xb6f3e0c0;
pub const MESSAGES_GET_DIALOGS: u32 = 0xa0f4cb4f;
pub const MESSAGES_GET_HISTORY: u32 = 0xdc3f8240;
pub const MESSAGES_GET_MESSAGES: u32 = 0x63c66506;
pub const MESSAGES_GET_BOT_CALLBACK_ANSWER: u32 = 0x934a4ee1;
pub const MESSAGES_DELETE_MESSAGES: u32 = 0xe58e95c6;
pub const MESSAGES_DELETE_HISTORY: u32 = 0xb7e36194;
pub const MESSAGES_EDIT_MESSAGE: u32 = 0x48f71768;
pub const MESSAGES_READ_HISTORY: u32 = 0x0e306d3a;
pub const MESSAGES_SEARCH: u32 = 0xd07bbf76;
pub const MESSAGES_SEND_CALLBACK_DATA: u32 = 0x934a4ee1;

// --- Users ---
pub const USERS_GET_FULL_USER: u32 = 0xe0b917f2;
pub const USERS_GET_USERS: u32 = 0x0d91a548;
/// users.userFull#d69e83e0 full_user:UserFull chats:Vector<Chat> users:Vector<User>
pub const USERS_USER_FULL: u32 = 0xd69e83e0;
/// contacts.found#b3134d19 my_results:Vector<Peer> results:Vector<Peer> chats:Vector<Chat> users:Vector<User>
pub const CONTACTS_FOUND: u32 = 0xb3134d19;
/// updates.state#a56c2a3e pts:int qts:int date:int seq:int unread_count:int
pub const UPDATES_STATE: u32 = 0xa56c2a3e;

// --- Contacts ---
pub const CONTACTS_RESOLVE_USERNAME: u32 = 0xf93ccba3;
pub const CONTACTS_RESOLVE_PHONE: u32 = 0x8af2a521;
pub const CONTACTS_SEARCH: u32 = 0x11f812d8;

// --- Channels ---
pub const CHANNELS_CREATE_CHANNEL: u32 = 0x3d5d10fd;
pub const CHANNELS_INVITE_TO_CHANNEL: u32 = 0x199f3a6c;
pub const CHANNELS_EDIT_ADMIN: u32 = 0x70d896ff;
pub const CHANNELS_GET_CHANNELS: u32 = 0xa7f6d76b;
pub const CHANNELS_GET_PARTICIPANTS: u32 = 0x123ffe12;
pub const CHANNELS_EDIT_ABOUT: u32 = 0x13e27b46;
pub const CHANNELS_LEAVE_CHANNEL: u32 = 0xf836aa28;

// --- Updates ---
pub const UPDATES_GET_STATE: u32 = 0xedd4882a;
pub const UPDATES_GET_DIFFERENCE: u32 = 0x25939104;
pub const UPDATES_GET_CHANNEL_DIFFERENCE: u32 = 0x3173d78;

// --- Upload ---
pub const UPLOAD_SAVE_FILE_PART: u32 = 0xb304a621;
pub const UPLOAD_SAVE_BIG_FILE_PART: u32 = 0xde7b673d;
pub const UPLOAD_GET_FILE: u32 = 0xb3e7e951;
pub const UPLOAD_GET_WEB_FILE: u32 = 0x24e5e54e;
pub const UPLOAD_SAVE_FILE: u32 = 0x96f18c5e;
pub const UPLOAD_GET_CDN_FILE: u32 = 0x572f9519;

// --- Help ---
pub const HELP_GET_CONFIG: u32 = 0xc4f3926c;
pub const HELP_GET_NEAREST_DC: u32 = 0x1fb33026;

// --- Photos ---
pub const PHOTOS_UPDATE_PROFILE_PHOTO: u32 = 0x1c3c2a85;
pub const PHOTOS_UPLOAD_PROFILE_PHOTO: u32 = 0x4f32c098;
pub const PHOTOS_DELETE_PHOTOS: u32 = 0x87cf7f2f;
pub const PHOTOS_GET_USER_PHOTOS: u32 = 0x91cd32a8;

// --- Invoke wrappers ---
pub const INVOKE_WITH_LAYER: u32 = 0xda9b0d0d;
pub const INVOKE_AFTER_MSG: u32 = 0xcb9f372d;
pub const INVOKE_WITHOUT_UPDATES: u32 = 0xbf94591b;

// --- Bool ---
pub const BOOL_TRUE: u32 = 0x997275b5;
pub const BOOL_FALSE: u32 = 0xbc799737;
pub const VECTOR: u32 = 0x1cb5c415;

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

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_id_newtype() {
        let uid = UserId(12345);
        assert_eq!(uid.0, 12345);
        let raw: i64 = uid.into();
        assert_eq!(raw, 12345);
    }

    #[test]
    fn test_input_peer_roundtrip() {
        let peer = input_peer_user(12345, 67890);
        let mut w = TLWriter::new();
        peer.write_to(&mut w);
        let mut r = TLReader::new(w.as_bytes());
        let parsed = InputPeer::read_from(&mut r).unwrap();
        match parsed {
            InputPeer::User { user_id, access_hash } => {
                assert_eq!(user_id.0, 12345);
                assert_eq!(access_hash.0, 67890);
            }
            _ => panic!("expected User"),
        }
    }

    #[test]
    fn test_input_peer_chat_roundtrip() {
        let peer = input_peer_chat(999);
        let mut w = TLWriter::new();
        peer.write_to(&mut w);
        let mut r = TLReader::new(w.as_bytes());
        let parsed = InputPeer::read_from(&mut r).unwrap();
        match parsed {
            InputPeer::Chat { chat_id } => assert_eq!(chat_id.0, 999),
            _ => panic!("expected Chat"),
        }
    }

    #[test]
    fn test_input_peer_channel_roundtrip() {
        let peer = input_peer_channel(42, 100);
        let mut w = TLWriter::new();
        peer.write_to(&mut w);
        let mut r = TLReader::new(w.as_bytes());
        let parsed = InputPeer::read_from(&mut r).unwrap();
        match parsed {
            InputPeer::Channel { channel_id, access_hash } => {
                assert_eq!(channel_id.0, 42);
                assert_eq!(access_hash.0, 100);
            }
            _ => panic!("expected Channel"),
        }
    }

    #[test]
    fn test_peer_roundtrip() {
        for peer in &[
            Peer::User { user_id: UserId(1) },
            Peer::Chat { chat_id: ChatId(2) },
            Peer::Channel { channel_id: ChannelId(3) },
        ] {
            let mut w = TLWriter::new();
            peer.write_to(&mut w);
            let mut r = TLReader::new(w.as_bytes());
            let parsed = Peer::read_from(&mut r).unwrap();
            assert_eq!(&parsed, peer);
        }
    }

    #[test]
    fn test_keyboard_button_text_roundtrip() {
        let btn = KeyboardButton::Text { text: "Click me".into() };
        let mut w = TLWriter::new();
        btn.write_to(&mut w);
        let mut r = TLReader::new(w.as_bytes());
        assert_eq!(r.read_u32().unwrap(), KEYBOARD_BUTTON);
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "Click me");
    }

    #[test]
    fn test_write_vector_long() {
        let mut w = TLWriter::new();
        write_vector_long(&mut w, &[1, 2, 3]);
        let mut r = TLReader::new(w.as_bytes());
        assert_eq!(r.read_u32().unwrap(), VECTOR);
        assert_eq!(r.read_i32().unwrap(), 3);
        assert_eq!(r.read_i64().unwrap(), 1);
        assert_eq!(r.read_i64().unwrap(), 2);
        assert_eq!(r.read_i64().unwrap(), 3);
    }

    #[test]
    fn test_constructor_ids_unique() {
        // Ensure key constructors don't collide
        let ids = vec![
            INPUT_PEER_SELF, INPUT_PEER_USER, INPUT_PEER_USER_FROM_ID,
            INPUT_PEER_CHAT, INPUT_PEER_CHANNEL, INPUT_PEER_CHANNEL_FROM_ID,
        ];
        let mut deduped = ids.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(ids.len(), deduped.len(), "duplicate constructor IDs detected");
    }

    #[test]
    fn test_user_read_parses_flags2_before_id() {
        // user#31774388 always serializes TWO flag words before id:long.
        // A bot-auth response typically has flags=0x4000 (bot flag),
        // flags2=0, id=777.
        let mut w = TLWriter::new();
        w.write_u32(USER); // 0x31774388
        w.write_i32(1 << 14); // flags: bot=true
        w.write_i32(0); // flags2 (must be consumed here)
        w.write_i64(777); // id
        // access_hash:flags.0 not set — nothing follows
        let mut r = TLReader::new(w.as_bytes());
        let user = User::read_from(&mut r).unwrap();
        assert_eq!(user.id().0, 777, "flags2 word must be consumed before id");
        assert!(user.is_bot());
    }
}
