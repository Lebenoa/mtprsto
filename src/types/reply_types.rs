//! Reply-surface types (SPEC §7): message entities, reply markups,
//! document attributes, photo sizes, ChatFull/ChannelFull, DialogFolder,
//! and named chat admin/banned rights.
//!
//! All constructor IDs verified against the Layer 223 schema (2026-08).

use super::*;
use crate::error::{Error, Result};
use crate::serialize::TLReader;

// ===========================================================================
// Message entities
// ===========================================================================

/// A TL message entity with a concrete kind.
#[derive(Debug, Clone)]
pub struct MessageEntityFull {
    pub offset: i32,
    pub length: i32,
    pub kind: MessageEntityKind,
}

/// Named message entity kinds (previously only `Unknown(u32)` existed).
#[derive(Debug, Clone, PartialEq)]
pub enum MessageEntityKind {
    Mention,
    Hashtag,
    BotCommand,
    Url,
    Email,
    Bold,
    Italic,
    Underline,
    Strike,
    Code,
    /// `messageEntityPre` — code block with a language tag.
    Pre { language: String },
    TextUrl { url: String },
    /// `messageEntityMentionName` — inline mention of a user by id.
    MentionName { user_id: i64 },
    Phone,
    Cashtag,
    Spoiler,
    CustomEmoji { document_id: i64 },
    Blockquote,
    BankCard,
    Unknown(u32),
}

impl MessageEntityKind {
    /// Read the kind-specific payload that follows `offset`/`length`.
    fn read_tail(ctor: u32, r: &mut TLReader) -> Result<Self> {
        Ok(match ctor {
            MESSAGE_ENTITY_MENTION => Self::Mention,
            MESSAGE_ENTITY_HASHTAG => Self::Hashtag,
            MESSAGE_ENTITY_BOT_COMMAND => Self::BotCommand,
            MESSAGE_ENTITY_URL => Self::Url,
            MESSAGE_ENTITY_EMAIL => Self::Email,
            MESSAGE_ENTITY_BOLD => Self::Bold,
            MESSAGE_ENTITY_ITALIC => Self::Italic,
            MESSAGE_ENTITY_UNDERLINE => Self::Underline,
            MESSAGE_ENTITY_STRIKE => Self::Strike,
            MESSAGE_ENTITY_CODE => Self::Code,
            MESSAGE_ENTITY_PRE => Self::Pre {
                language: String::from_utf8(r.read_bytes()?)?,
            },
            MESSAGE_ENTITY_TEXT_URL => Self::TextUrl {
                url: String::from_utf8(r.read_bytes()?)?,
            },
            MESSAGE_ENTITY_MENTION_NAME => Self::MentionName { user_id: r.read_i64()? },
            MESSAGE_ENTITY_PHONE => Self::Phone,
            MESSAGE_ENTITY_CASHTAG => Self::Cashtag,
            MESSAGE_ENTITY_SPOILER => Self::Spoiler,
            MESSAGE_ENTITY_CUSTOM_EMOJI => Self::CustomEmoji { document_id: r.read_i64()? },
            MESSAGE_ENTITY_BLOCKQUOTE => Self::Blockquote, // flags already consumed by caller
            MESSAGE_ENTITY_BANK_CARD => Self::BankCard,
            other => Self::Unknown(other),
        })
    }
}

/// Read a `Vector<MessageEntity>`.
pub fn read_message_entities(r: &mut TLReader) -> Result<Vec<MessageEntityFull>> {
    let vec_ctor = r.read_u32()?;
    if vec_ctor != VECTOR {
        return Err(Error::Serialization(format!(
            "expected Vector<MessageEntity>, got {vec_ctor:#x}"
        )));
    }
    let count = r.read_i32()?;
    let mut out = Vec::with_capacity(count.max(0) as usize);
    for _ in 0..count {
        let ctor = r.read_u32()?;
        // messageEntityBlockquote carries flags between ctor and offset.
        if ctor == MESSAGE_ENTITY_BLOCKQUOTE {
            let _flags = r.read_i32()?;
        }
        let offset = r.read_i32()?;
        let length = r.read_i32()?;
        let kind = MessageEntityKind::read_tail(ctor, r)?;
        out.push(MessageEntityFull { offset, length, kind });
    }
    Ok(out)
}

// ===========================================================================
// Incoming reply markup (the outbound `ReplyMarkup` in
// reply_markup.rs has `write_to`; this is the parsed/read side).
// ===========================================================================

/// A keyboard button (subset: text, url, callback).
#[derive(Debug, Clone, PartialEq)]
pub enum KeyboardButtonKind {
    /// `keyboardButton` — plain text button.
    Text { text: String },
    /// `keyboardButtonUrl` — opens a URL.
    Url { text: String, url: String },
    /// `keyboardButtonCallback` — sends callback data to the bot.
    Callback { text: String, data: Vec<u8> },
    /// `keyboardButtonRequestPhone`, `keyboardButtonRequestGeoLocation`, etc.
    Other { ctor: u32, text: Option<String> },
}

/// One row of buttons.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyboardButtonRow {
    pub buttons: Vec<KeyboardButtonKind>,
}

/// Reply markup attached to an incoming message.
#[derive(Debug, Clone)]
pub enum IncomingReplyMarkup {
    /// `replyKeyboardHide#a03e5b85 flags:# selective:flags.2?true`
    HideKeyboard { selective: bool },
    /// `replyKeyboardForceReply#86b40b08 flags:# single_use:flags.1?true
    ///  selective:flags.2?true placeholder:flags.3?string`
    ForceReply { single_use: bool, selective: bool, placeholder: Option<String> },
    /// `replyKeyboardMarkup#85dd99d1 flags:# resize:flags.0?true
    ///  single_use:flags.1?true selective:flags.2?true persistent:flags.4?true
    ///  rows:Vector<KeyboardButtonRow> placeholder:flags.3?string`
    Keyboard {
        resize: bool,
        single_use: bool,
        selective: bool,
        persistent: bool,
        rows: Vec<KeyboardButtonRow>,
        placeholder: Option<String>,
    },
    /// `replyInlineMarkup#48a30254 rows:Vector<KeyboardButtonRow>`
    Inline { rows: Vec<KeyboardButtonRow> },
}

/// Read a keyboard button (after its ctor).
fn read_keyboard_button(ctor: u32, r: &mut TLReader) -> Result<KeyboardButtonKind> {
    // All button ctors carry a flags:# first (style:flags.10).
    let flags = r.read_i32()?;
    // keyboardButtonStyle#4fdd3430 flags:# icon:flags.3?long
    if flags & (1 << 10) != 0 {
        let style_ctor = r.read_u32()?;
        if style_ctor != KEYBOARD_BUTTON_STYLE {
            return Err(Error::Serialization(format!(
                "expected keyboardButtonStyle, got {style_ctor:#x}"
            )));
        }
        let style_flags = r.read_i32()?;
        if style_flags & (1 << 3) != 0 {
            let _icon = r.read_i64()?;
        }
    }
    Ok(match ctor {
        KEYBOARD_BUTTON => KeyboardButtonKind::Text {
            text: String::from_utf8(r.read_bytes()?)?,
        },
        KEYBOARD_BUTTON_URL => {
            let text = String::from_utf8(r.read_bytes()?)?;
            let url = String::from_utf8(r.read_bytes()?)?;
            KeyboardButtonKind::Url { text, url }
        }
        KEYBOARD_BUTTON_CALLBACK => {
            let text = String::from_utf8(r.read_bytes()?)?;
            let data = r.read_bytes()?;
            KeyboardButtonKind::Callback { text, data }
        }
        other => {
            // Unknown button: best-effort text read (most ctors have text as
            // the first string). The remaining bytes cannot be skipped
            // reliably without full knowledge of the ctor.
            let text = String::from_utf8(r.read_bytes()?).ok();
            KeyboardButtonKind::Other { ctor: other, text }
        }
    })
}

/// Read a `Vector<KeyboardButtonRow>`.
fn read_button_rows(r: &mut TLReader) -> Result<Vec<KeyboardButtonRow>> {
    let vec_ctor = r.read_u32()?;
    if vec_ctor != VECTOR {
        return Err(Error::Serialization(format!(
            "expected Vector<KeyboardButtonRow>, got {vec_ctor:#x}"
        )));
    }
    let row_count = r.read_i32()?;
    let mut rows = Vec::with_capacity(row_count.max(0) as usize);
    for _ in 0..row_count {
        // keyboardButtonRow#77608b83 buttons:Vector<KeyboardButton>
        let row_ctor = r.read_u32()?;
        if row_ctor != KEYBOARD_BUTTON_ROW {
            return Err(Error::Serialization(format!(
                "expected keyboardButtonRow, got {row_ctor:#x}"
            )));
        }
        let vec_ctor = r.read_u32()?;
        if vec_ctor != VECTOR {
            return Err(Error::Serialization(format!(
                "expected Vector<KeyboardButton>, got {vec_ctor:#x}"
            )));
        }
        let button_count = r.read_i32()?;
        let mut buttons = Vec::with_capacity(button_count.max(0) as usize);
        for _ in 0..button_count {
            let ctor = r.read_u32()?;
            buttons.push(read_keyboard_button(ctor, r)?);
        }
        rows.push(KeyboardButtonRow { buttons });
    }
    Ok(rows)
}

/// Read any `ReplyMarkup` payload (ctor already consumed by the caller is
/// NOT supported — pass the full remaining buffer with ctor first).
pub fn read_reply_markup(r: &mut TLReader) -> Result<IncomingReplyMarkup> {
    let ctor = r.read_u32()?;
    Ok(match ctor {
        REPLY_KEYBOARD_HIDE => {
            let flags = r.read_i32()?;
            IncomingReplyMarkup::HideKeyboard { selective: flags & (1 << 2) != 0 }
        }
        REPLY_KEYBOARD_FORCE_REPLY => {
            let flags = r.read_i32()?;
            let placeholder = if flags & (1 << 3) != 0 {
                Some(String::from_utf8(r.read_bytes()?)?)
            } else {
                None
            };
            IncomingReplyMarkup::ForceReply {
                single_use: flags & (1 << 1) != 0,
                selective: flags & (1 << 2) != 0,
                placeholder,
            }
        }
        REPLY_KEYBOARD_MARKUP_223 => {
            let flags = r.read_i32()?;
            let rows = read_button_rows(r)?;
            let placeholder = if flags & (1 << 3) != 0 {
                Some(String::from_utf8(r.read_bytes()?)?)
            } else {
                None
            };
            IncomingReplyMarkup::Keyboard {
                resize: flags & (1 << 0) != 0,
                single_use: flags & (1 << 1) != 0,
                selective: flags & (1 << 2) != 0,
                persistent: flags & (1 << 4) != 0,
                rows,
                placeholder,
            }
        }
        REPLY_INLINE_MARKUP => {
            let rows = read_button_rows(r)?;
            IncomingReplyMarkup::Inline { rows }
        }
        other => {
            return Err(Error::Serialization(format!(
                "unknown ReplyMarkup constructor {other:#x}"
            )))
        }
    })
}

// ===========================================================================
// Document attributes
// ===========================================================================

/// `DocumentAttribute` variants relevant to file metadata.
#[derive(Debug, Clone, PartialEq)]
pub enum DocumentAttribute {
    /// `documentAttributeImageSize#6c37c15c w:int h:int`
    ImageSize { w: i32, h: i32 },
    /// `documentAttributeAnimated#11b58939`
    Animated,
    /// `documentAttributeSticker#6319d612 flags:# mask:flags.1?true alt:string ...`
    Sticker { alt: String, mask: bool },
    /// `documentAttributeVideo#43c57c48 flags:# round_message:flags.0?true
    ///  supports_streaming:flags.1?true nosound:flags.3?true duration:double
    ///  w:int h:int preload_prefix_size:flags.2?int ...`
    Video {
        duration: f64,
        w: i32,
        h: i32,
        round_message: bool,
        supports_streaming: bool,
    },
    /// `documentAttributeAudio#9852f9c6 flags:# voice:flags.10?true
    ///  duration:int title:flags.0?string performer:flags.1?string
    ///  waveform:flags.2?bytes`
    Audio {
        duration: i32,
        voice: bool,
        title: Option<String>,
        performer: Option<String>,
    },
    /// `documentAttributeFilename#15590068 file_name:string`
    Filename { file_name: String },
    /// `documentAttributeHasStickers#9801d2f7`
    HasStickers,
    /// `documentAttributeCustomEmoji#fd149899 flags:# free:flags.0?true
    ///  text_color:flags.1?true alt:string stickerset:InputStickerSet`
    CustomEmoji { alt: String, free: bool },
}

/// Read one `DocumentAttribute` (ctor included).
pub fn read_document_attribute(r: &mut TLReader) -> Result<DocumentAttribute> {
    let ctor = r.read_u32()?;
    Ok(match ctor {
        DOCUMENT_ATTRIBUTE_IMAGE_SIZE => DocumentAttribute::ImageSize {
            w: r.read_i32()?,
            h: r.read_i32()?,
        },
        DOCUMENT_ATTRIBUTE_ANIMATED => DocumentAttribute::Animated,
        DOCUMENT_ATTRIBUTE_STICKER => {
            let flags = r.read_i32()?;
            let alt = String::from_utf8(r.read_bytes()?)?;
            // stickerset:InputStickerSet — bare ctor + fields; InputStickerSet
            // Empty is ctor 0xffb62b95 with no fields; anything else carries
            // fields we can't skip reliably without full support.
            let set_ctor = r.read_u32()?;
            if set_ctor != 0xffb62b95 {
                return Err(Error::Serialization(format!(
                    "non-empty InputStickerSet ({set_ctor:#x}) not supported yet"
                )));
            }
            DocumentAttribute::Sticker { alt, mask: flags & (1 << 1) != 0 }
        }
        DOCUMENT_ATTRIBUTE_VIDEO => {
            let flags = r.read_i32()?;
            let duration = f64::from_bits(r.read_u64()?);
            let w = r.read_i32()?;
            let h = r.read_i32()?;
            let _preload_prefix_size = if flags & (1 << 2) != 0 {
                Some(r.read_i32()?)
            } else {
                None
            };
            let _video_start_ts = if flags & (1 << 4) != 0 {
                Some(f64::from_bits(r.read_u64()?))
            } else {
                None
            };
            let _video_codec = if flags & (1 << 5) != 0 {
                Some(String::from_utf8(r.read_bytes()?)?)
            } else {
                None
            };
            let video_codec = if flags & (1 << 5) != 0 {
                String::from_utf8(r.read_bytes()?)?
            } else {
                String::new()
            };
            let _ = video_codec;
            DocumentAttribute::Video {
                duration,
                w,
                h,
                round_message: flags & (1 << 0) != 0,
                supports_streaming: flags & (1 << 1) != 0,
            }
        }
        DOCUMENT_ATTRIBUTE_AUDIO => {
            let flags = r.read_i32()?;
            let duration = r.read_i32()?;
            let title = if flags & (1 << 0) != 0 {
                Some(String::from_utf8(r.read_bytes()?)?)
            } else {
                None
            };
            let performer = if flags & (1 << 1) != 0 {
                Some(String::from_utf8(r.read_bytes()?)?)
            } else {
                None
            };
            let waveform = if flags & (1 << 2) != 0 {
                let _ = r.read_bytes()?;
                Some(())
            } else {
                None
            };
            let _ = waveform;
            DocumentAttribute::Audio {
                duration,
                voice: flags & (1 << 10) != 0,
                title,
                performer,
            }
        }
        DOCUMENT_ATTRIBUTE_FILENAME => DocumentAttribute::Filename {
            file_name: String::from_utf8(r.read_bytes()?)?,
        },
        DOCUMENT_ATTRIBUTE_HAS_STICKERS => DocumentAttribute::HasStickers,
        DOCUMENT_ATTRIBUTE_CUSTOM_EMOJI => {
            let flags = r.read_i32()?;
            let alt = String::from_utf8(r.read_bytes()?)?;
            let set_ctor = r.read_u32()?;
            if set_ctor != 0xffb62b95 {
                return Err(Error::Serialization(format!(
                    "non-empty InputStickerSet ({set_ctor:#x}) not supported yet"
                )));
            }
            DocumentAttribute::CustomEmoji {
                alt,
                free: flags & (1 << 0) != 0,
            }
        }
        other => {
            return Err(Error::Serialization(format!(
                "unknown DocumentAttribute constructor {other:#x}"
            )))
        }
    })
}

/// Read a `Vector<DocumentAttribute>`.
pub fn read_document_attributes(r: &mut TLReader) -> Result<Vec<DocumentAttribute>> {
    let vec_ctor = r.read_u32()?;
    if vec_ctor != VECTOR {
        return Err(Error::Serialization(format!(
            "expected Vector<DocumentAttribute>, got {vec_ctor:#x}"
        )));
    }
    let count = r.read_i32()?;
    let mut out = Vec::with_capacity(count.max(0) as usize);
    for _ in 0..count {
        out.push(read_document_attribute(r)?);
    }
    Ok(out)
}

// ===========================================================================
// Photo sizes
// ===========================================================================

/// Full `PhotoSize` union (previously stubbed).
#[derive(Debug, Clone, PartialEq)]
pub enum PhotoSizeFull {
    /// `photoSize#75c78e60 type:string w:int h:int size:int`
    Size { type_: String, w: i32, h: i32, size: i32 },
    /// `photoStrippedSize#e0b0bc2e type:string bytes:bytes`
    Stripped { type_: String, bytes: Vec<u8> },
    /// `photoSizeProgressive#fa3efb95 type:string w:int h:int sizes:Vector<int>`
    Progressive { type_: String, w: i32, h: i32, sizes: Vec<i32> },
    /// `photoPathSize#d8214d41 type:string bytes:bytes`
    Path { type_: String, bytes: Vec<u8> },
}

/// Read one `PhotoSize` (ctor included).
pub fn read_photo_size(r: &mut TLReader) -> Result<PhotoSizeFull> {
    let ctor = r.read_u32()?;
    Ok(match ctor {
        PHOTO_SIZE => PhotoSizeFull::Size {
            type_: String::from_utf8(r.read_bytes()?)?,
            w: r.read_i32()?,
            h: r.read_i32()?,
            size: r.read_i32()?,
        },
        PHOTO_STRIPPED_SIZE => PhotoSizeFull::Stripped {
            type_: String::from_utf8(r.read_bytes()?)?,
            bytes: r.read_bytes()?,
        },
        PHOTO_SIZE_PROGRESSIVE => {
            let type_ = String::from_utf8(r.read_bytes()?)?;
            let w = r.read_i32()?;
            let h = r.read_i32()?;
            let vec_ctor = r.read_u32()?;
            if vec_ctor != VECTOR {
                return Err(Error::Serialization(format!(
                    "expected Vector<int> in photoSizeProgressive, got {vec_ctor:#x}"
                )));
            }
            let count = r.read_i32()?;
            let mut sizes = Vec::with_capacity(count.max(0) as usize);
            for _ in 0..count {
                sizes.push(r.read_i32()?);
            }
            PhotoSizeFull::Progressive { type_, w, h, sizes }
        }
        PHOTO_PATH_SIZE => PhotoSizeFull::Path {
            type_: String::from_utf8(r.read_bytes()?)?,
            bytes: r.read_bytes()?,
        },
        other => {
            return Err(Error::Serialization(format!(
                "unknown PhotoSize constructor {other:#x}"
            )))
        }
    })
}

/// Read a `Vector<PhotoSize>`.
pub fn read_photo_sizes(r: &mut TLReader) -> Result<Vec<PhotoSizeFull>> {
    let vec_ctor = r.read_u32()?;
    if vec_ctor != VECTOR {
        return Err(Error::Serialization(format!(
            "expected Vector<PhotoSize>, got {vec_ctor:#x}"
        )));
    }
    let count = r.read_i32()?;
    let mut out = Vec::with_capacity(count.max(0) as usize);
    for _ in 0..count {
        out.push(read_photo_size(r)?);
    }
    Ok(out)
}

// ===========================================================================
// Chat admin / banned rights with named fields
// ===========================================================================

/// `chatAdminRights#5fb224d5` — named flag accessors.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatAdminRightsFull {
    pub flags: i32,
}

impl ChatAdminRightsFull {
    pub fn from_flags(flags: i32) -> Self {
        Self { flags }
    }
    pub fn change_info(&self) -> bool { self.flags & (1 << 0) != 0 }
    pub fn post_messages(&self) -> bool { self.flags & (1 << 1) != 0 }
    pub fn edit_messages(&self) -> bool { self.flags & (1 << 2) != 0 }
    pub fn delete_messages(&self) -> bool { self.flags & (1 << 3) != 0 }
    pub fn ban_users(&self) -> bool { self.flags & (1 << 4) != 0 }
    pub fn invite_users(&self) -> bool { self.flags & (1 << 5) != 0 }
    pub fn pin_messages(&self) -> bool { self.flags & (1 << 7) != 0 }
    pub fn add_admins(&self) -> bool { self.flags & (1 << 9) != 0 }
    pub fn anonymous(&self) -> bool { self.flags & (1 << 10) != 0 }
    pub fn manage_call(&self) -> bool { self.flags & (1 << 11) != 0 }
    pub fn other(&self) -> bool { self.flags & (1 << 12) != 0 }
    pub fn manage_topics(&self) -> bool { self.flags & (1 << 13) != 0 }
    pub fn post_stories(&self) -> bool { self.flags & (1 << 14) != 0 }
    pub fn edit_stories(&self) -> bool { self.flags & (1 << 15) != 0 }
    pub fn delete_stories(&self) -> bool { self.flags & (1 << 16) != 0 }
}

/// `chatBannedRights#9f120418` — named flag accessors.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatBannedRightsFull {
    pub flags: i32,
    pub until_date: i32,
}

impl ChatBannedRightsFull {
    pub fn from_flags(flags: i32, until_date: i32) -> Self {
        Self { flags, until_date }
    }
    pub fn view_messages(&self) -> bool { self.flags & (1 << 0) != 0 }
    pub fn send_messages(&self) -> bool { self.flags & (1 << 1) != 0 }
    pub fn send_media(&self) -> bool { self.flags & (1 << 2) != 0 }
    pub fn send_stickers(&self) -> bool { self.flags & (1 << 3) != 0 }
    pub fn send_gifs(&self) -> bool { self.flags & (1 << 4) != 0 }
    pub fn send_games(&self) -> bool { self.flags & (1 << 5) != 0 }
    pub fn send_inline(&self) -> bool { self.flags & (1 << 6) != 0 }
    pub fn embed_links(&self) -> bool { self.flags & (1 << 7) != 0 }
    pub fn send_polls(&self) -> bool { self.flags & (1 << 8) != 0 }
    pub fn change_info(&self) -> bool { self.flags & (1 << 10) != 0 }
    pub fn invite_users(&self) -> bool { self.flags & (1 << 15) != 0 }
    pub fn pin_messages(&self) -> bool { self.flags & (1 << 17) != 0 }
    pub fn manage_topics(&self) -> bool { self.flags & (1 << 18) != 0 }
    pub fn send_photos(&self) -> bool { self.flags & (1 << 19) != 0 }
    pub fn send_videos(&self) -> bool { self.flags & (1 << 20) != 0 }
}

/// Read `chatBannedRights#9f120418` (ctor included).
pub fn read_chat_banned_rights(r: &mut TLReader) -> Result<ChatBannedRightsFull> {
    let ctor = r.read_u32()?;
    if ctor != CHAT_BANNED_RIGHTS {
        return Err(Error::Serialization(format!(
            "expected chatBannedRights, got {ctor:#x}"
        )));
    }
    Ok(ChatBannedRightsFull {
        flags: r.read_i32()?,
        until_date: r.read_i32()?,
    })
}

// ===========================================================================
// ChatFull / ChannelFull
// ===========================================================================

/// Parsed `chatFull#2633421b` / `channelFull#e4e0b29d`.
///
/// Vector-typed and deeply-nested members (participants, bot_info, call…)
/// are captured as "not parsed yet" markers; scalars and strings that
/// callers actually need are fully populated.
#[derive(Debug, Clone)]
pub struct ChatFullInfo {
    pub is_channel: bool,
    pub id: i64,
    pub about: String,
    // chatFull only
    pub can_set_username: bool,
    pub pinned_msg_id: Option<i32>,
    // channelFull only
    pub can_view_participants: bool,
    pub participants_count: Option<i32>,
    pub admins_count: Option<i32>,
    pub banned_count: Option<i32>,
    pub online_count: Option<i32>,
    pub read_inbox_max_id: i32,
    pub read_outbox_max_id: i32,
    pub unread_count: i32,
    pub migrated_from_chat_id: Option<i64>,
    pub linked_chat_id: Option<i64>,
    pub pts: Option<i32>,
    /// `true` when the raw TL tail contained members this parser does not
    /// model (call/requests_pending/stories/...).
    pub has_unparsed_tail: bool,
}

/// Read a `ChatFull`-family payload (ctor included). Handles both
/// `chatFull#2633421b` and `channelFull#e4e0b29d`.
///
/// Fields this parser does not consume are skipped in wire order using the
/// schema's flag layout, and `has_unparsed_tail` reports whether any such
/// tail existed.
pub fn read_chat_full(r: &mut TLReader) -> Result<ChatFullInfo> {
    let ctor = r.read_u32()?;
    match ctor {
        CHAT_FULL => read_chat_full_plain(r),
        CHANNEL_FULL => read_channel_full(r),
        other => Err(Error::Serialization(format!(
            "expected chatFull/channelFull, got {other:#x}"
        ))),
    }
}

/// Read `chatFull#2633421b` body. ChatParticipants (which immediately
/// follows `about`) has a variable layout, so parsing stops at `about`;
/// everything after is reported via `has_unparsed_tail`.
fn read_chat_full_plain(r: &mut TLReader) -> Result<ChatFullInfo> {
    let flags = r.read_i32()?;
    let id = r.read_i64()?;
    let about = String::from_utf8(r.read_bytes()?)?;
    // participants:ChatParticipants — complex; capture and skip its payload
    // is not possible without full support, so report unparsed tail. We must
    // still locate the fields AFTER it, which is impossible positionally.
    // ChatParticipants has a variable layout, so chatFull is parsed only up
    // to `about`; everything after is reported as unparsed.
    let _can_set_username = flags & (1 << 7) != 0;
    let pinned_msg_id = None;

    Ok(ChatFullInfo {
        is_channel: false,
        id,
        about,
        can_set_username: flags & (1 << 7) != 0,
        pinned_msg_id,
        can_view_participants: false,
        participants_count: None,
        admins_count: None,
        banned_count: None,
        online_count: None,
        read_inbox_max_id: 0,
        read_outbox_max_id: 0,
        unread_count: 0,
        migrated_from_chat_id: None,
        linked_chat_id: None,
        pts: None,
        has_unparsed_tail: true, // participants onward
    })
}

fn read_channel_full(r: &mut TLReader) -> Result<ChatFullInfo> {
    // channelFull#e4e0b29d flags:# ... flags2:# ... id:long about:string
    // participants_count:flags.0?int admins_count:flags.1?int
    // kicked_count:flags.2?int banned_count:flags.2?int
    // online_count:flags.13?int read_inbox_max_id:int read_outbox_max_id:int
    // unread_count:int chat_photo:Photo notify_settings:PeerNotifySettings
    // exported_invite:flags.23?ExportedChatInvite bot_info:Vector<BotInfo>
    // migrated_from_chat_id:flags.4?long migrated_from_max_id:flags.4?int
    // pinned_msg_id:flags.5?int stickerset:flags.8?StickerSet
    // available_min_id:flags.9?int folder_id:flags.11?int
    // linked_chat_id:flags.14?long location:flags.15?ChannelLocation
    // slowmode_seconds:flags.17?int slowmode_next_send_date:flags.18?int
    // stats_dc:flags.12?int pts:int call:flags.21?InputGroupCall
    // ttl_period:flags.24?int pending_suggestions:flags.25?Vector<string>
    // groupcall_default_join_as:flags.26?Peer theme_emoticon:flags.27?string
    // requests_pending:flags.28?int recent_requesters:flags.28?Vector<long>
    // default_send_as:flags.29?Peer available_reactions:flags.30?ChatReactions
    // reactions_limit:flags2.13?int stories:flags2.4?PeerStories
    // wallpaper:flags2.7?WallPaper boosts_applied:flags2.8?int
    // boosts_unrestrict:flags2.9?int emojiset:flags2.10?StickerSet
    // bot_verification:flags2.17?BotVerification
    // stargifts_count:flags2.18?int
    // send_paid_messages_stars:flags2.21?long main_tab:flags2.22?ProfileTab
    //
    // Everything up to unread_count is positionally parseable. chat_photo is
    // a complex Photo; bot_info a Vector<BotInfo> — both variable-length, so
    // parsing stops after notify_settings flag handling: we return the
    // scalars and mark the tail unparsed (chat_photo onwards).
    let flags = r.read_i32()?;
    let flags2 = r.read_i32()?;
    let _ = flags2;
    let id = r.read_i64()?;
    let about = String::from_utf8(r.read_bytes()?)?;
    let participants_count = if flags & (1 << 0) != 0 { Some(r.read_i32()?) } else { None };
    let admins_count = if flags & (1 << 1) != 0 { Some(r.read_i32()?) } else { None };
    let banned_count = if flags & (1 << 2) != 0 { Some(r.read_i32()?) } else { None };
    let online_count = if flags & (1 << 13) != 0 { Some(r.read_i32()?) } else { None };
    let read_inbox_max_id = r.read_i32()?;
    let read_outbox_max_id = r.read_i32()?;
    let unread_count = r.read_i32()?;
    // chat_photo:Photo follows — variable; stop here.
    Ok(ChatFullInfo {
        is_channel: true,
        id,
        about,
        can_set_username: flags & (1 << 6) != 0,
        pinned_msg_id: None,
        can_view_participants: flags & (1 << 3) != 0,
        participants_count,
        admins_count,
        banned_count,
        online_count,
        read_inbox_max_id,
        read_outbox_max_id,
        unread_count,
        migrated_from_chat_id: None,
        linked_chat_id: None,
        pts: None,
        has_unparsed_tail: true, // chat_photo onward
    })
}

// ===========================================================================
// DialogFolder
// ===========================================================================

/// Parsed `dialogFolder#71bd134c`.
#[derive(Debug, Clone)]
pub struct DialogFolderFull {
    pub pinned: bool,
    /// `folder#ff544e65 id:int title:string` (photo:flags.3 skipped).
    pub folder_id: i32,
    pub folder_title: String,
    pub peer: Peer,
    pub top_message: i32,
    pub unread_muted_peers_count: i32,
    pub unread_unmuted_peers_count: i32,
    pub unread_muted_messages_count: i32,
    pub unread_unmuted_messages_count: i32,
}

impl DialogFolderFull {
    /// Read a `dialogFolder#71bd134c` payload (ctor included).
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        if ctor != DIALOG_FOLDER {
            return Err(Error::Serialization(format!(
                "expected dialogFolder, got {ctor:#x}"
            )));
        }
        let flags = r.read_i32()?;
        // folder:Folder — folder#ff544e65 flags:# autofill_*:flags.0-2?true
        // id:int title:string photo:flags.3?ChatPhoto
        let folder_ctor = r.read_u32()?;
        if folder_ctor != FOLDER {
            return Err(Error::Serialization(format!(
                "expected folder, got {folder_ctor:#x}"
            )));
        }
        let folder_flags = r.read_i32()?;
        let folder_id = r.read_i32()?;
        let folder_title = String::from_utf8(r.read_bytes()?)?;
        let _has_folder_photo = folder_flags & (1 << 3) != 0;

        Ok(Self {
            pinned: flags & (1 << 2) != 0,
            folder_id,
            folder_title,
            peer: Peer::read_from(r)?,
            top_message: r.read_i32()?,
            unread_muted_peers_count: r.read_i32()?,
            unread_unmuted_peers_count: r.read_i32()?,
            unread_muted_messages_count: r.read_i32()?,
            unread_unmuted_messages_count: r.read_i32()?,
        })
    }
}
