//! API reply types (SendMessageResult, SentCode, Authorization, ...).

use super::*;
#[allow(unused_imports)]
use std::fmt;

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
