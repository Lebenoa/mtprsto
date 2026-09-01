//! `Message`, `MessageFull`, `ReplyHeader`, `MessageAction`, `MessageMedia`,
//! `MessageEntity`.

use super::constructors::{
    MESSAGE, MESSAGE_ACTION_CHANNEL_CREATE, MESSAGE_ACTION_CHAT_ADD_USER,
    MESSAGE_ACTION_CHAT_CREATE, MESSAGE_ACTION_CHAT_DELETE_USER, MESSAGE_ACTION_CHAT_EDIT_TITLE,
    MESSAGE_ACTION_CHAT_JOINED_BY_LINK, MESSAGE_ACTION_CHAT_JOINED_BY_REQUEST,
    MESSAGE_ACTION_CONTACT_SIGN_UP, MESSAGE_ACTION_EMPTY, MESSAGE_ACTION_GAME_SCORE,
    MESSAGE_ACTION_HISTORY_CLEAR, MESSAGE_ACTION_PIN_MESSAGE, MESSAGE_EMPTY, MESSAGE_MEDIA_CONTACT,
    MESSAGE_MEDIA_DICE, MESSAGE_MEDIA_DOCUMENT, MESSAGE_MEDIA_EMPTY, MESSAGE_MEDIA_GAME,
    MESSAGE_MEDIA_GEO, MESSAGE_MEDIA_GEO_LIVE, MESSAGE_MEDIA_GIVEAWAY,
    MESSAGE_MEDIA_GIVEAWAY_RESULTS, MESSAGE_MEDIA_INVOICE, MESSAGE_MEDIA_PAID_MEDIA,
    MESSAGE_MEDIA_PHOTO, MESSAGE_MEDIA_POLL, MESSAGE_MEDIA_STORY, MESSAGE_MEDIA_VENUE,
    MESSAGE_MEDIA_WEB_PAGE, MESSAGE_REPLY_HEADER, MESSAGE_REPLY_HEADER_V225,
    MESSAGE_REPLY_STORY_HEADER, MESSAGE_SERVICE,
};
use super::{
    Document, DocumentId, GeoPoint, IncomingReplyMarkup, MessageEntityKind, MsgId, Peer, Photo,
    PhotoId, UserId, WebPage,
};
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
    #[must_use]
    pub fn id(&self) -> MsgId {
        match self {
            Self::Message(full) => full.id,
            Self::Empty { id } | Self::Service { id, .. } => *id,
        }
    }

    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            Self::Message(full) => &full.message,
            _ => "",
        }
    }

    #[must_use]
    pub fn peer_id(&self) -> &Peer {
        match self {
            Self::Message(full) => &full.peer_id,
            Self::Service { peer_id, .. } => peer_id,
            Self::Empty { .. } => &Peer::None,
        }
    }

    #[must_use]
    pub fn from_id(&self) -> Option<&Peer> {
        match self {
            Self::Message(full) => full.from_id.as_ref(),
            Self::Service { from_id, .. } => from_id.as_ref(),
            Self::Empty { .. } => None,
        }
    }

    #[must_use]
    pub fn media(&self) -> Option<&MessageMedia> {
        match self {
            Self::Message(full) => full.media.as_ref(),
            _ => None,
        }
    }

    /// The document behind the message, when it carries one.
    #[must_use]
    pub fn document(&self) -> Option<Document> {
        match self.media() {
            Some(MessageMedia::Document { document, .. }) => Some(document.clone()),
            _ => None,
        }
    }

    /// The photo behind the message, when it carries one (non-empty).
    #[must_use]
    pub fn photo(&self) -> Option<Photo> {
        match self.media() {
            Some(MessageMedia::Photo {
                photo: p @ Photo::Photo { .. },
            }) => Some(p.clone()),
            _ => None,
        }
    }

    /// # Errors
    ///
    /// Forwards [`Error::Serialization`] from [`Self::read_from`].
    pub fn parse_from_bytes(data: &[u8]) -> Result<Self> {
        let mut r = TLReader::new(data);
        Self::read_from(&mut r)
    }

    /// # Errors
    ///
    /// Returns [`Error::Serialization`] for unknown constructors, for
    /// conditionals this library does not model, and for nested payloads
    /// that fail to decode.
    #[allow(clippy::too_many_lines)] // one arm per schema ctor with every conditional in wire order
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            MESSAGE => {
                // message#3ae56482: flags + flags2,
                // id:int,
                // message:string is REQUIRED, conditionals interleaved.
                let flags = r.read_i32()?;
                let flags2 = r.read_i32()?;
                let id = MsgId(i64::from(r.read_i32()?));
                let from_id = if flags & (1 << 8) != 0 {
                    Some(Peer::read_from(r)?)
                } else {
                    None
                };
                if flags & (1 << 29) != 0 {
                    let _from_boosts = r.read_i32()?;
                }
                if flags2 & (1 << 12) != 0 {
                    let _from_rank = r.read_bytes()?;
                }
                let peer_id = Peer::read_from(r)?;
                if flags & (1 << 28) != 0 {
                    let _saved_peer_id = Peer::read_from(r)?;
                }
                if flags & (1 << 2) != 0 {
                    // fwd_from:MessageFwdHeader — decode via the generated
                    // parser and discard (the curated MessageFull keeps no
                    // fwd body; erroring here broke get_dialogs on any
                    // dialog whose top message is a forward).
                    let _fwd = crate::types::MessageFwdHeader::read_from(r)?;
                }
                let via_bot_id = if flags & (1 << 11) != 0 {
                    Some(UserId(r.read_i64()?))
                } else {
                    None
                };
                if flags2 & (1 << 0) != 0 {
                    let _via_business_bot = r.read_i64()?;
                }
                // NOTE: layer 223 message has NO guestchat_via_from field
                // (flags2.19 is `legacy` at 223 — a no-byte true-flag);
                // reading it here misparsed legacy messages.
                let reply_to = if flags & (1 << 3) != 0 {
                    Some(ReplyHeader::read_from(r)?)
                } else {
                    None
                };
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
                        .map(|e| MessageEntity {
                            offset: e.offset,
                            length: e.length,
                            kind: MessageEntityType::Known(e.kind),
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                let views = if flags & (1 << 10) != 0 {
                    Some(r.read_i32()?)
                } else {
                    None
                };
                if flags & (1 << 10) != 0 {
                    let _forwards = r.read_i32()?; // forwards shares flags.10
                }
                if flags & (1 << 23) != 0 {
                    // replies:MessageReplies — generated decode-discard.
                    let _ = crate::types::MessageReplies::read_from(r)?;
                }
                let edit_date = if flags & (1 << 15) != 0 {
                    Some(r.read_i32()?)
                } else {
                    None
                };
                if flags & (1 << 16) != 0 {
                    let _post_author = r.read_bytes()?;
                }
                let grouped_id = if flags & (1 << 17) != 0 {
                    Some(r.read_i64()?)
                } else {
                    None
                };
                if flags & (1 << 20) != 0 {
                    // reactions:MessageReactions — generated decode-discard.
                    let _ = crate::types::MessageReactions::read_from(r)?;
                }
                if flags & (1 << 22) != 0 {
                    // restrictionReason#d072acb4 platform reason text
                    let n = r.read_vector_header()?;
                    for _ in 0..n {
                        let _ctor = r.read_u32()?;
                        let _platform = r.read_bytes()?;
                        let _reason = r.read_bytes()?;
                        let _text = r.read_bytes()?;
                    }
                }
                if flags & (1 << 25) != 0 {
                    let _ttl_period = r.read_i32()?;
                }
                if flags & (1 << 30) != 0 {
                    let _quick_reply_shortcut_id = r.read_i32()?;
                }
                if flags2 & (1 << 2) != 0 {
                    let _effect = r.read_i64()?;
                }
                if flags2 & (1 << 3) != 0 {
                    // factcheck:FactCheck — generated decode-discard.
                    let _ = crate::types::FactCheck::read_from(r)?;
                }
                if flags2 & (1 << 5) != 0 {
                    let _report_delivery_until_date = r.read_i32()?;
                }
                if flags2 & (1 << 6) != 0 {
                    let _paid_message_stars = r.read_i64()?;
                }
                if flags2 & (1 << 7) != 0 {
                    // suggested_post:SuggestedPost — generated decode-discard.
                    let _ = crate::types::SuggestedPost::read_from(r)?;
                }
                if flags2 & (1 << 10) != 0 {
                    let _schedule_repeat_period = r.read_i32()?;
                }
                if flags2 & (1 << 11) != 0 {
                    let _summary_from_language = r.read_bytes()?;
                }
                if flags2 & (1 << 13) != 0 {
                    return Err(Error::Serialization(
                        "message rich_message (RichMessage) parsing not supported".into(),
                    ));
                }
                let post = flags & (1 << 14) != 0;
                let edit_hide = flags & (1 << 21) != 0;
                Ok(Self::Message(Box::new(MessageFull {
                    id,
                    from_id,
                    peer_id,
                    date,
                    message: message_text,
                    media,
                    reply_markup,
                    entities,
                    views,
                    edit_date,
                    post,
                    grouped_id,
                    via_bot_id,
                    reply_to,
                    edit_hide,
                })))
            }
            MESSAGE_EMPTY => {
                // messageEmpty#90a6ca84 flags:# id:int peer_id:flags.0?Peer
                let _flags = r.read_i32()?;
                let id = MsgId(i64::from(r.read_i32()?));
                Ok(Self::Empty { id })
            }
            MESSAGE_SERVICE => {
                // messageService#7a800e0a flags:# (no flags2) id:int
                //   from_id:flags.8?Peer peer_id:Peer saved_peer_id:flags.28?Peer
                //   reply_to:flags.3?MessageReplyHeader date:int
                //   action:MessageAction reactions:flags.20? ttl_period:flags.25?int
                let flags = r.read_i32()?;
                let id = MsgId(i64::from(r.read_i32()?));
                let from_id = if flags & (1 << 8) != 0 {
                    Some(Peer::read_from(r)?)
                } else {
                    None
                };
                let peer_id = Peer::read_from(r)?;
                if flags & (1 << 28) != 0 {
                    let _saved_peer_id = Peer::read_from(r)?;
                }
                let reply_to = if flags & (1 << 3) != 0 {
                    Some(ReplyHeader::read_from(r)?)
                } else {
                    None
                };
                let date = r.read_i32()?;
                let action = MessageAction::read_from(r)?;
                if flags & (1 << 20) != 0 {
                    return Err(Error::Serialization(
                        "messageService reactions (MessageReactions) not supported".into(),
                    ));
                }
                if flags & (1 << 25) != 0 {
                    let _ttl_period = r.read_i32()?;
                }
                Ok(Self::Service {
                    id,
                    from_id,
                    peer_id,
                    date,
                    action,
                    reply_to,
                })
            }
            other => Err(Error::Serialization(format!(
                "unknown Message constructor {other:#x}"
            ))),
        }
    }
}

/// Reply header (`reply_to_msg_id`, `reply_to_peer_id`, etc.).
#[derive(Debug, Clone)]
pub struct ReplyHeader {
    pub reply_to_msg_id: MsgId,
    pub reply_to_peer_id: Option<Peer>,
    pub reply_to_top_id: Option<MsgId>,
}

impl ReplyHeader {
    /// messageReplyHeader#6917560b (layer 223).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Serialization`] for unknown constructors, story
    /// replies, a missing `reply_to_msg_id`, and nested payloads this
    /// parser does not support.
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        if ctor == MESSAGE_REPLY_STORY_HEADER {
            // messageReplyStoryHeader#0e5af939 peer:Peer story_id:int —
            // consume so the stream stays aligned, then fail loudly.
            let _peer = Peer::read_from(r)?;
            let _story_id = r.read_i32()?;
            return Err(Error::Serialization(
                "story reply (messageReplyStoryHeader) not supported".into(),
            ));
        }
        if ctor != MESSAGE_REPLY_HEADER && ctor != MESSAGE_REPLY_HEADER_V225 {
            return Err(Error::Serialization(format!(
                "unknown MessageReplyHeader constructor {ctor:#x}"
            )));
        }
        let flags = r.read_i32()?;
        let reply_to_msg_id = if flags & (1 << 4) != 0 {
            Some(MsgId(i64::from(r.read_i32()?)))
        } else {
            None
        };
        let reply_to_peer_id = if flags & (1 << 0) != 0 {
            Some(Peer::read_from(r)?)
        } else {
            None
        };
        if flags & (1 << 5) != 0 {
            return Err(Error::Serialization(
                "reply_from (MessageFwdHeader) parsing not supported".into(),
            ));
        }
        if flags & (1 << 8) != 0 {
            let _reply_media = crate::types::MessageMedia::read_from(r)?;
        }
        let reply_to_top_id = if flags & (1 << 1) != 0 {
            Some(MsgId(i64::from(r.read_i32()?)))
        } else {
            None
        };
        if flags & (1 << 6) != 0 {
            let _quote_text = r.read_bytes()?;
        }
        if flags & (1 << 7) != 0 {
            let _quote_entities = super::reply_types::read_message_entities(r)?;
        }
        if flags & (1 << 10) != 0 {
            let _quote_offset = r.read_i32()?;
        }
        if flags & (1 << 11) != 0 {
            let _todo_item_id = r.read_i32()?;
        }
        if flags & (1 << 12) != 0 {
            let _poll_option = r.read_bytes()?;
        }
        Ok(Self {
            reply_to_msg_id: reply_to_msg_id
                .ok_or_else(|| Error::Serialization("reply header without msg id".into()))?,
            reply_to_peer_id,
            reply_to_top_id,
        })
    }
}

/// Message action (service messages).
#[derive(Debug, Clone)]
pub enum MessageAction {
    Empty,
    MessageActionChatCreate {
        title: String,
        users: Vec<UserId>,
    },
    MessageActionChatEditTitle {
        title: String,
    },
    MessageActionChatAddUser {
        users: Vec<UserId>,
    },
    MessageActionChatDeleteUser {
        user_id: UserId,
    },
    MessageActionChatJoinedByLink {
        inviter_id: UserId,
        via_link: bool,
    },
    MessageActionChannelCreate {
        title: String,
    },
    MessageActionPinMessage,
    MessageActionHistoryClear,
    MessageActionGameScore {
        game_id: i64,
        score: i32,
    },
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
    /// # Errors
    ///
    /// Returns [`Error::Serialization`] when a recognized action's payload
    /// fails to decode; unknown actions drain the frame and become
    /// [`MessageAction::Other`].
    #[allow(clippy::cast_sign_loss, clippy::as_conversions)] // TL vector header: length-prefixed i32 count, non-negative on well-formed frames
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            MESSAGE_ACTION_EMPTY => Ok(Self::Empty),
            MESSAGE_ACTION_HISTORY_CLEAR => Ok(Self::MessageActionHistoryClear),
            MESSAGE_ACTION_PIN_MESSAGE => Ok(Self::MessageActionPinMessage),
            MESSAGE_ACTION_CONTACT_SIGN_UP | MESSAGE_ACTION_CHAT_JOINED_BY_REQUEST => {
                Ok(Self::Other)
            }
            MESSAGE_ACTION_CHAT_CREATE => {
                // messageActionChatCreate#bd47cbad title:string users:Vector<long>
                let title = String::from_utf8(r.read_bytes()?)?;
                let n = r.read_vector_header()?;
                let mut users = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    users.push(UserId(r.read_i64()?));
                }
                Ok(Self::MessageActionChatCreate { title, users })
            }
            MESSAGE_ACTION_CHAT_EDIT_TITLE => {
                let title = String::from_utf8(r.read_bytes()?)?;
                Ok(Self::MessageActionChatEditTitle { title })
            }
            MESSAGE_ACTION_CHAT_ADD_USER => {
                let n = r.read_vector_header()?;
                let mut users = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    users.push(UserId(r.read_i64()?));
                }
                Ok(Self::MessageActionChatAddUser { users })
            }
            MESSAGE_ACTION_CHAT_DELETE_USER => Ok(Self::MessageActionChatDeleteUser {
                user_id: UserId(r.read_i64()?),
            }),
            MESSAGE_ACTION_CHAT_JOINED_BY_LINK => {
                // inviter_id:long
                Ok(Self::MessageActionChatJoinedByLink {
                    inviter_id: UserId(r.read_i64()?),
                    via_link: true,
                })
            }
            MESSAGE_ACTION_CHANNEL_CREATE => {
                let title = String::from_utf8(r.read_bytes()?)?;
                Ok(Self::MessageActionChannelCreate { title })
            }
            MESSAGE_ACTION_GAME_SCORE => {
                let game_id = r.read_i64()?;
                let score = r.read_i32()?;
                Ok(Self::MessageActionGameScore { game_id, score })
            }
            _ => {
                // Unknown action: no safe way to know its length — drain the
                // rest of the frame (only safe because callers treat the
                // messageService tail as terminal after this).
                while r.remaining() > 0 {
                    let _ = r.read_i32()?;
                }
                Ok(Self::Other)
            }
        }
    }
}

/// Message media (attachments on a message).
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum MessageMedia {
    None,
    Photo {
        photo: Photo,
    },
    Geo {
        geo: GeoPoint,
    },
    Contact {
        user_id: UserId,
        first_name: String,
        last_name: String,
        phone_number: String,
        vcard: String,
    },
    Document {
        document: Document,
        caption: String,
    },
    WebPage {
        webpage: WebPage,
    },
    VoiceCall {},
    Game {
        game: String,
    },
    Poll {},
    Dice {
        value: i32,
        emoticon: String,
    },
    /// `messageMediaVenue` — geo plus a human-readable place name.
    Venue {
        geo: GeoPoint,
        title: String,
        address: String,
    },
    /// `messageMediaGeoLive` — live location.
    GeoLive {
        geo: GeoPoint,
        heading: Option<i32>,
        period: i32,
    },
    /// Recognized but variable-length media (poll, invoice, story,
    /// giveaway, paid media, game). Presence is known; the payload is not
    /// modelled.
    Unsupported,
    /// Constructor not recognized by this library version.
    Unknown(u32),
}

impl MessageMedia {
    /// # Errors
    ///
    /// Returns [`Error::Serialization`] when a recognized media payload
    /// fails to decode or carries a conditional this parser does not
    /// support; unrecognized constructors come back as [`Self::Unknown`].
    #[allow(clippy::too_many_lines)] // one arm per schema ctor with every conditional in wire order
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            MESSAGE_MEDIA_EMPTY => Ok(Self::None),
            MESSAGE_MEDIA_PHOTO => {
                // messageMediaPhoto#e216eb63 flags:# spoiler:flags.3?true
                //   live_photo:flags.4?true photo:flags.0?Photo
                //   ttl_seconds:flags.2?int video:flags.4?Document
                let flags = r.read_i32()?;
                let photo = if flags & (1 << 0) != 0 {
                    Photo::read_from(r)?
                } else {
                    Photo::Empty { id: PhotoId(0) }
                };
                if flags & (1 << 2) != 0 {
                    let _ttl_seconds = r.read_i32()?;
                }
                if flags & (1 << 4) != 0 {
                    Document::read_from(r)?; // live_photo video
                }
                Ok(Self::Photo { photo })
            }
            MESSAGE_MEDIA_DOCUMENT => {
                // messageMediaDocument#52d8ccd9 flags:# nopremium:flags.3?true
                //   spoiler:flags.4?true video:flags.6?true round:flags.7?true
                //   voice:flags.8?true document:flags.0?Document
                //   alt_documents:flags.5?Vector<Document>
                //   video_cover:flags.9?Photo video_timestamp:flags.10?int
                //   ttl_seconds:flags.2?int
                let flags = r.read_i32()?;
                let document = if flags & (1 << 0) != 0 {
                    Some(Document::read_from(r)?)
                } else {
                    None
                };
                if flags & (1 << 5) != 0 {
                    let n = r.read_vector_header()?;
                    for _ in 0..n {
                        Document::read_from(r)?;
                    }
                }
                if flags & (1 << 9) != 0 {
                    Photo::read_from(r)?; // video_cover
                }
                if flags & (1 << 10) != 0 {
                    let _video_timestamp = r.read_i32()?;
                }
                if flags & (1 << 2) != 0 {
                    let _ttl_seconds = r.read_i32()?;
                }
                Ok(Self::Document {
                    document: document.unwrap_or(Document::Empty { id: DocumentId(0) }),
                    caption: String::new(),
                })
            }
            MESSAGE_MEDIA_WEB_PAGE => {
                // messageMediaWebPage#ddf10c3b flags:# webpage:WebPage
                let _flags = r.read_i32()?;
                let webpage = WebPage::read_from(r)?;
                Ok(Self::WebPage { webpage })
            }
            MESSAGE_MEDIA_GEO => {
                let geo = GeoPoint::read_from(r)?;
                Ok(Self::Geo { geo })
            }
            MESSAGE_MEDIA_DICE => {
                // messageMediaDice#8cbec07 flags:# value:int emoticon:string
                //   game_outcome:flags.0?messages.EmojiGameOutcome
                let flags = r.read_i32()?;
                let value = r.read_i32()?;
                let emoticon = String::from_utf8(r.read_bytes()?)?;
                if flags & (1 << 0) != 0 {
                    return Err(Error::Serialization(
                        "messageMediaDice game_outcome not supported".into(),
                    ));
                }
                Ok(Self::Dice { value, emoticon })
            }
            MESSAGE_MEDIA_VENUE => {
                let geo = GeoPoint::read_from(r)?;
                let title = String::from_utf8(r.read_bytes()?)?;
                let address = String::from_utf8(r.read_bytes()?)?;
                let _provider = String::from_utf8(r.read_bytes()?)?;
                let _venue_id = String::from_utf8(r.read_bytes()?)?;
                let _venue_type = String::from_utf8(r.read_bytes()?)?;
                Ok(Self::Venue {
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
                Ok(Self::GeoLive {
                    geo,
                    heading,
                    period,
                })
            }
            MESSAGE_MEDIA_CONTACT => {
                let phone_number = String::from_utf8(r.read_bytes()?)?;
                let first_name = String::from_utf8(r.read_bytes()?)?;
                let last_name = String::from_utf8(r.read_bytes()?)?;
                let vcard = String::from_utf8(r.read_bytes()?)?;
                let user_id = UserId(r.read_i64()?);
                Ok(Self::Contact {
                    user_id,
                    first_name,
                    last_name,
                    phone_number,
                    vcard,
                })
            }
            MESSAGE_MEDIA_POLL
            | MESSAGE_MEDIA_INVOICE
            | MESSAGE_MEDIA_STORY
            | MESSAGE_MEDIA_GIVEAWAY
            | MESSAGE_MEDIA_GIVEAWAY_RESULTS
            | MESSAGE_MEDIA_PAID_MEDIA
            | MESSAGE_MEDIA_GAME => {
                // Variable-length nested payloads without per-field skip
                // support. Mark as present-but-unsupported; the enclosing
                // message parse completes because we stop consuming here.
                Ok(Self::Unsupported)
            }
            other => {
                // Do NOT drain the reader (old behaviour corrupted the
                // stream); surface the ctor to the caller instead.
                Ok(Self::Unknown(other))
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
