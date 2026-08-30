//! Chat and Channel variants plus admin/banned rights.

use super::*;
use crate::error::{Error, Result};
use crate::serialize::TLReader;
#[allow(unused_imports)]
use std::fmt;

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
    /// `chatForbidden#7328209` — the account cannot view this chat.
    Forbidden { id: ChatId, title: String },
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
                // chat#41cbf256 flags:# id:long title:string photo:ChatPhoto
                //   participants_count:int date:int version:int
                //   migrated_to:flags.6?InputChannel admin_rights:flags.14?
                //   default_banned_rights:flags.18?ChatBannedRights
                let flags = r.read_i32()?;
                let id = ChatId(r.read_i64()?);
                let title = String::from_utf8(r.read_bytes()?)?;
                // photo:ChatPhoto — always present (same two ctors as channel)
                let photo_ctor = r.read_u32()?;
                match photo_ctor {
                    crate::types::CHAT_PHOTO => {
                        let pflags = r.read_i32()?;
                        let _photo_id = r.read_i64()?;
                        if pflags & (1 << 1) != 0 {
                            let _thumb = r.read_bytes()?;
                        }
                        let _dc_id = r.read_i32()?;
                    }
                    crate::types::CHAT_PHOTO_EMPTY => {}
                    other => {
                        return Err(Error::Serialization(format!(
                            "unknown ChatPhoto constructor {other:#x} in chat"
                        )))
                    }
                }
                let participants_count = r.read_i32()?;
                let date = r.read_i32()?;
                let version = r.read_i32()?;
                if flags & (1 << 6) != 0 {
                    // migrated_to:InputChannel — inputChannel#f35aec28 or inputChannelFromMessage
                    let ictor = r.read_u32()?;
                    match ictor {
                        crate::types::INPUT_CHANNEL => {
                            let _cid = r.read_i64()?;
                            let _ahash = r.read_i64()?;
                        }
                        other => {
                            return Err(Error::Serialization(format!(
                                "unsupported InputChannel constructor {other:#x} in chat.migrated_to"
                            )))
                        }
                    }
                }
                if flags & (1 << 14) != 0 {
                    let _ctor = r.read_u32()?;
                    let _rights = r.read_i32()?;
                }
                if flags & (1 << 18) != 0 {
                    let _ctor = r.read_u32()?;
                    let _bflags = r.read_i32()?;
                    let _until = r.read_i32()?;
                }
                let creator = flags & (1 << 0) != 0;
                let kicked = flags & (1 << 1) != 0;
                let left = flags & (1 << 2) != 0;
                let deactivated = flags & (1 << 5) != 0;
                Ok(Chat::Chat {
                    id, title, photo: None, participants_count, date, version,
                    creator, kicked, left, deactivated,
                })
            }
            CHAT_EMPTY => {
                let id = ChatId(r.read_i64()?);
                Ok(Chat::Empty { id })
            }
            CHAT_FORBIDDEN => {
                // chatForbidden#7328209: id:int then title:string
                let id = ChatId(r.read_i64()?);
                let title = String::from_utf8(r.read_bytes()?)?;
                Ok(Chat::Forbidden { id, title })
            }
            CHANNEL => {
                // channel#d49f34c6 flags:# flags2:# id:long
                //   access_hash:flags.13?long title:string username:flags.6?string
                //   photo:ChatPhoto date:int
                //   restriction_reason:flags.9?Vector<RestrictionReason>
                //   admin_rights:flags.14?ChatAdminRights
                //   banned_rights:flags.15?ChatBannedRights
                //   default_banned_rights:flags.18?ChatBannedRights
                //   participants_count:flags.17?int usernames:flags2.0?Vector<Username>
                //   stories_max_id:flags2.4?RecentStory color:flags2.7?PeerColor
                //   profile_color:flags2.8?PeerColor emoji_status:flags2.9?EmojiStatus
                //   level:flags2.10?int subscription_until_date:flags2.11?int
                //   bot_verification_icon:flags2.13?long
                //   send_paid_messages_stars:flags2.14?long
                //   linked_monoforum_id:flags2.18?long linked_community_id:flags2.20?long
                let flags = r.read_i32()?;
                let flags2 = r.read_i32()?;
                let id = ChannelId(r.read_i64()?);
                let access_hash = if flags & (1 << 13) != 0 {
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
                // photo:ChatPhoto — always present. chatPhoto#1c6e1c11
                // flags:# has_video photo_id:long stripped_thumb:flags.1?bytes dc_id:int
                // | chatPhotoEmpty#37c1011c
                let photo_ctor = r.read_u32()?;
                match photo_ctor {
                    crate::types::CHAT_PHOTO => {
                        let pflags = r.read_i32()?;
                        let _photo_id = r.read_i64()?;
                        if pflags & (1 << 1) != 0 {
                            let _thumb = r.read_bytes()?;
                        }
                        let _dc_id = r.read_i32()?;
                    }
                    crate::types::CHAT_PHOTO_EMPTY => {}
                    other => {
                        return Err(Error::Serialization(format!(
                            "unknown ChatPhoto constructor {other:#x} in channel"
                        )))
                    }
                }
                let date = r.read_i32()?;
                // restriction_reason:flags.9?Vector<restrictionReason#d072acb4
                //   platform:string reason:string text:string>
                if flags & (1 << 9) != 0 {
                    let n = r.read_vector_header()?;
                    for _ in 0..n {
                        let _ctor = r.read_u32()?;
                        let _platform = r.read_bytes()?;
                        let _reason = r.read_bytes()?;
                        let _text = r.read_bytes()?;
                    }
                }
                let admin_rights = if flags & (1 << 14) != 0 {
                    // chatAdminRights#5fb224d5 flags:#
                    let _ctor = r.read_u32()?;
                    Some(ChatAdminRights { flags: r.read_i32()? })
                } else {
                    None
                };
                let banned_rights = if flags & (1 << 15) != 0 {
                    // chatBannedRights#9f120418 flags:# until_date:int
                    let _ctor = r.read_u32()?;
                    let bflags = r.read_i32()?;
                    let until = r.read_i32()?;
                    Some(ChatBannedRights { flags: bflags, until_date: until })
                } else {
                    None
                };
                if flags & (1 << 18) != 0 {
                    // default_banned_rights — same shape
                    let _ctor = r.read_u32()?;
                    let _bflags = r.read_i32()?;
                    let _until = r.read_i32()?;
                }
                let participants_count = if flags & (1 << 17) != 0 {
                    Some(r.read_i32()?)
                } else {
                    None
                };
                // usernames:flags2.0?Vector<username#b4073647 flags:# username:string>
                if flags2 & (1 << 0) != 0 {
                    let n = r.read_vector_header()?;
                    for _ in 0..n {
                        let _uflags = r.read_i32()?;
                        let _username = r.read_bytes()?;
                    }
                }
                if flags2 & (1 << 4) != 0 {
                    // recentStory#711d692d flags:# live max_id — parse to
                    // keep the stream aligned.
                    let sflags = r.read_i32()?;
                    let _max_id = if sflags & (1 << 1) != 0 {
                        Some(r.read_i32()?)
                    } else {
                        None
                    };
                }
                // color:flags2.7?peerColor#b54b5acf and
                // profile_color:flags2.8?peerColor — same shape:
                // flags:# color:flags.0?int background_emoji_id:flags.1?long
                for present in [(flags2 & (1 << 7) != 0), (flags2 & (1 << 8) != 0)] {
                    if !present {
                        continue;
                    }
                    let cflags = r.read_i32()?;
                    if cflags & (1 << 0) != 0 {
                        let _ = r.read_i32()?;
                    }
                    if cflags & (1 << 1) != 0 {
                        let _ = r.read_i64()?;
                    }
                }
                // emoji_status:flags2.9?EmojiStatus
                if flags2 & (1 << 9) != 0 {
                    let ector = r.read_u32()?;
                    match ector {
                        crate::types::EMOJI_STATUS_EMPTY => {}
                        crate::types::EMOJI_STATUS => {
                            let _eflags = r.read_i32()?;
                            let _doc = r.read_i64()?;
                        }
                        other => {
                            return Err(Error::Serialization(format!(
                                "unknown EmojiStatus constructor {other:#x} in channel"
                            )))
                        }
                    }
                }
                if flags2 & (1 << 10) != 0 { let _level = r.read_i32()?; }
                if flags2 & (1 << 11) != 0 { let _sub = r.read_i32()?; }
                if flags2 & (1 << 13) != 0 { let _bot_verification_icon = r.read_i64()?; }
                if flags2 & (1 << 14) != 0 { let _paid_stars = r.read_i64()?; }
                if flags2 & (1 << 18) != 0 { let _linked_monoforum = r.read_i64()?; }
                if flags2 & (1 << 20) != 0 { let _linked_community = r.read_i64()?; }

                let megagroup = flags & (1 << 8) != 0;
                let broadcast = flags & (1 << 5) != 0;
                let verified = flags & (1 << 7) != 0;
                let scam = flags & (1 << 19) != 0;
                let fake = flags & (1 << 25) != 0;
                let left = flags & (1 << 2) != 0;
                Ok(Chat::Channel {
                    id, access_hash, title, username, photo: None, date,
                    version: participants_count.unwrap_or(0),
                    megagroup, broadcast, verified, scam, fake, left,
                    signature_names_default: false,
                    admin_rights,
                    banned_rights,
                })
            }
            CHANNEL_FORBIDDEN => {
                let flags = r.read_i32()?;
                let id = ChannelId(r.read_i64()?);
                let access_hash = AccessHash(r.read_i64()?);
                let title = String::from_utf8(r.read_bytes()?)?;
                if flags & (1 << 16) != 0 {
                    let _until_date = r.read_i32()?;
                }
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
