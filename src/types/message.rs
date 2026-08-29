//! Message, MessageFull, ReplyHeader, MessageAction, MessageMedia, MessageEntity.

use super::*;
use crate::error::{Error, Result};
use crate::serialize::TLReader;
#[allow(unused_imports)]
use std::fmt;

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
    pub reply_markup: Option<IncomingReplyMarkup>,
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
                    super::reply_types::read_reply_markup(r).ok()
                } else {
                    None
                };
                let entities = if flags & (1 << 7) != 0 {
                    super::reply_types::read_message_entities(r)?
                        .into_iter()
                        .map(|e| MessageEntity { offset: e.offset, length: e.length, kind: MessageEntityType::Known(e.kind) })
                        .collect()
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
    /// `messageMediaVenue` — geo plus a human-readable place name.
    Venue { geo: GeoPoint, title: String, address: String },
    /// `messageMediaGeoLive` — live location.
    GeoLive { geo: GeoPoint, heading: Option<i32>, period: i32 },
    /// Recognized but variable-length media (poll, invoice, story,
    /// giveaway, paid media, game). Presence is known; the payload is not
    /// modelled.
    Unsupported,
    /// Constructor not recognized by this library version.
    Unknown(u32),
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
            MESSAGE_MEDIA_VENUE => {
                let geo = GeoPoint::read_from(r)?;
                let title = String::from_utf8(r.read_bytes()?)?;
                let address = String::from_utf8(r.read_bytes()?)?;
                let _provider = String::from_utf8(r.read_bytes()?)?;
                let _venue_id = String::from_utf8(r.read_bytes()?)?;
                let _venue_type = String::from_utf8(r.read_bytes()?)?;
                Ok(MessageMedia::Venue {
                    geo,
                    title,
                    address,
                })
            }
            MESSAGE_MEDIA_GEO_LIVE => {
                let flags = r.read_i32()?;
                let geo = GeoPoint::read_from(r)?;
                let heading = if flags & (1 << 0) != 0 {
                    Some(r.read_i32()?)
                } else {
                    None
                };
                let period = r.read_i32()?;
                if flags & (1 << 1) != 0 {
                    let _ = r.read_i32()?; // proximity_notification_radius
                }
                Ok(MessageMedia::GeoLive { geo, heading, period })
            }
            MESSAGE_MEDIA_CONTACT => {
                let phone_number = String::from_utf8(r.read_bytes()?)?;
                let first_name = String::from_utf8(r.read_bytes()?)?;
                let last_name = String::from_utf8(r.read_bytes()?)?;
                let vcard = String::from_utf8(r.read_bytes()?)?;
                let user_id = UserId(r.read_i64()?);
                Ok(MessageMedia::Contact {
                    user_id,
                    first_name,
                    last_name,
                    phone_number,
                    vcard,
                })
            }
            MESSAGE_MEDIA_POLL | MESSAGE_MEDIA_INVOICE | MESSAGE_MEDIA_STORY
            | MESSAGE_MEDIA_GIVEAWAY | MESSAGE_MEDIA_GIVEAWAY_RESULTS
            | MESSAGE_MEDIA_PAID_MEDIA | MESSAGE_MEDIA_GAME => {
                // Variable-length nested payloads without per-field skip
                // support. Mark as present-but-unsupported; the enclosing
                // message parse completes because we stop consuming here.
                Ok(MessageMedia::Unsupported)
            }
            other => {
                // Do NOT drain the reader (old behaviour corrupted the
                // stream); surface the ctor to the caller instead.
                Ok(MessageMedia::Unknown(other))
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
    /// Fully-parsed kind (see [`MessageEntityKind`]).
    Known(MessageEntityKind),
}
