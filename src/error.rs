//! Typed errors for the MTProto library.
//!
//! Every error variant corresponds to a specific failure mode in the
//! Telegram MTProto stack. The `Display` implementation follows
//! `{kind}: {detail} [dc=N key=0x{short}]` so logs are searchable.
//!
//! `is_transient()` returns `true` for errors that a retry loop can
//! safely swallow and back off from.

use std::fmt;
use std::time::Instant;

/// Result type alias for the MTProto library.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur in the MTProto library.
#[derive(Debug)]
pub enum Error {
    // --- Transport layer ---
    /// I/O error from networking (TCP connect, read, write, timeout).
    Network(std::io::Error),

    /// Server says "move to DC N".
    Migration { dc_id: i32 },

    /// Flood wait: the server is rate-limiting this method on this DC.
    FloodWait {
        seconds: i32,
        retry_after: Instant,
    },

    // --- Auth ---
    /// The auth key is not yet created.
    NoAuthKey,

    /// The auth key was rejected or removed by the server.
    AuthKeyInvalid { dc_id: i32, key_short: String },

    /// The auth key is not registered on this DC.
    AuthKeyUnregistered { dc_id: i32, key_short: String },

    // --- RPC errors ---
    /// Any Telegram RPC error (4xx/5xx). `error_code` is Telegram's.
    Rpc {
        error_code: i32,
        error_message: String,
    },

    /// `PHONE_CODE_INVALID` or similar auth code error.
    InvalidCode { detail: String },

    /// `2FA password required` or wrong password.
    InvalidPassword { detail: String },

    /// Sign up is required (new account).
    SignUpRequired,

    // --- File handling ---
    /// `FILE_REFERENCE_EXPIRED` — the file reference must be refreshed.
    FileReferenceExpired { detail: String },

    // --- Protocol ---
    /// Cryptographic error.
    Crypto(String),

    /// Serialization/deserialization error.
    Serialization(String),

    /// Transport layer error (connection, framing).
    Transport(String),

    /// MTProto protocol error (bad msg_key, nonce mismatch, etc.).
    Protocol(String),

    /// The server rejected a message with `bad_msg_notification`. `code`
    /// is Telegram's bad_msg_code (16 msg_id too low, 17 too high, 18
    /// bad msg_key, 20 salt invalidated, 32-48 sequence issues, 64
    /// invalid container, 65 not authorised, 96 flood/ban).
    BadMessage { code: i32, description: String },

    /// The server reported `rpc_answer_unknown` or `rpc_answer_dropped`:
    /// the answer is not ready or was dropped. Retry with backoff.
    RpcDropped { detail: String },

    /// DH parameter verification failed.
    DhVerification(String),

    /// Server returned an unexpected response constructor.
    UnexpectedResponse(String),

    /// Padding verification failed.
    PaddingError(String),

    /// Generic fallback error — never use for known cases.
    Other(String),
}

impl Error {
    /// Returns `true` for errors that a retry loop can safely retry.
    ///
    /// Covers: `FloodWait`, `Network`, `FileReferenceExpired`,
    /// `AuthKeyUnregistered`, and some `Rpc` codes.
    pub fn is_transient(&self) -> bool {
        match self {
            Error::Network(_) => true,
            Error::FloodWait { .. } => true,
            Error::FileReferenceExpired { .. } => true,
            Error::AuthKeyUnregistered { .. } => true,
            Error::Rpc { error_code, .. } => {
                // Transient RPC codes: 420 (FLOOD), 500-599 (server errors),
                // 401 (unauthorized — key may be expired)
                matches!(error_code, 401 | 420 | 500..=599)
            }
            Error::RpcDropped { .. } => true,
            Error::Transport(_) => true,
            _ => false,
        }
    }

    /// Returns `true` if this error represents an auth failure that
    /// requires re-authentication (not just a retry).
    pub fn is_auth_error(&self) -> bool {
        matches!(
            self,
            Error::AuthKeyInvalid { .. }
                | Error::AuthKeyUnregistered { .. }
                | Error::InvalidPassword { .. }
                | Error::SignUpRequired
        )
    }

    /// Returns the DC ID associated with the error, if any.
    pub fn dc_id(&self) -> Option<i32> {
        match self {
            Error::Migration { dc_id } => Some(*dc_id),
            Error::AuthKeyInvalid { dc_id, .. } => Some(*dc_id),
            Error::AuthKeyUnregistered { dc_id, .. } => Some(*dc_id),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Network(e) => write!(f, "Network: {e}"),
            Error::Migration { dc_id } => write!(f, "Migration: move to DC {dc_id}"),
            Error::FloodWait { seconds, .. } => write!(f, "FloodWait: wait {seconds}s"),
            Error::NoAuthKey => write!(f, "NoAuthKey: no authorization key available"),
            Error::AuthKeyInvalid { dc_id, key_short } => {
                write!(f, "AuthKeyInvalid [dc={dc_id} key=0x{key_short}]")
            }
            Error::AuthKeyUnregistered { dc_id, key_short } => {
                write!(f, "AuthKeyUnregistered [dc={dc_id} key=0x{key_short}]")
            }
            Error::Rpc {
                error_code,
                error_message,
            } => write!(f, "Rpc: {error_message} [code={error_code}]"),
            Error::InvalidCode { detail } => write!(f, "InvalidCode: {detail}"),
            Error::InvalidPassword { detail } => write!(f, "InvalidPassword: {detail}"),
            Error::SignUpRequired => write!(f, "SignUpRequired: new account"),
            Error::FileReferenceExpired { detail } => {
                write!(f, "FileReferenceExpired: {detail}")
            }
            Error::Crypto(msg) => write!(f, "Crypto: {msg}"),
            Error::Serialization(msg) => write!(f, "Serialization: {msg}"),
            Error::Transport(msg) => write!(f, "Transport: {msg}"),
            Error::Protocol(msg) => write!(f, "Protocol: {msg}"),
            Error::BadMessage { code, description } => {
                write!(f, "BadMessage: {description} [code={code}]")
            }
            Error::RpcDropped { detail } => write!(f, "RpcDropped: {detail}"),
            Error::DhVerification(msg) => write!(f, "DhVerification: {msg}"),
            Error::UnexpectedResponse(msg) => write!(f, "UnexpectedResponse: {msg}"),
            Error::PaddingError(msg) => write!(f, "PaddingError: {msg}"),
            Error::Other(msg) => write!(f, "Other: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Network(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Network(e)
    }
}

impl From<std::string::FromUtf8Error> for Error {
    fn from(e: std::string::FromUtf8Error) -> Self {
        Error::Serialization(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Serialization(format!("JSON: {e}"))
    }
}

impl From<base64::DecodeSliceError> for Error {
    fn from(e: base64::DecodeSliceError) -> Self {
        Error::Serialization(format!("base64: {e}"))
    }
}

/// Classify an RPC error into a typed `Error` from its code and message.
///
/// The raw TL parsing lives in `mtproto::parse_rpc_error`; this performs
/// the message-based classification (FloodWait, Migration, auth errors…).
pub fn classify_rpc_error(error_code: i32, error_message: &str) -> Error {
    // Classify known error messages
    if let Some(secs_str) = error_message.strip_prefix("FLOOD_WAIT_")
        && let Ok(secs) = secs_str.parse::<i32>()
    {
        return Error::FloodWait {
            seconds: secs,
            retry_after: Instant::now() + std::time::Duration::from_secs(secs as u64),
        };
    }

    if error_message.contains("PHONE_CODE_INVALID") || error_message.contains("PHONE_CODE_EMPTY") {
        return Error::InvalidCode { detail: error_message.to_string() };
    }

    if error_message.contains("PASSWORD_HASH_INVALID") {
        return Error::InvalidPassword { detail: error_message.to_string() };
    }

    if error_message.contains("SIGN_UP_REQUIRED") || error_message.contains("first unoccupied") {
        return Error::SignUpRequired;
    }

    if error_message.contains("FILE_REFERENCE_EXPIRED") || error_message.contains("FILE_REFERENCE") {
        return Error::FileReferenceExpired { detail: error_message.to_string() };
    }

    if error_message.contains("AUTH_KEY_UNREGISTERED") {
        return Error::AuthKeyUnregistered {
            dc_id: 0,
            key_short: "????".into(),
        };
    }

    // PHONE_MIGRATE_X / USER_MIGRATE_X / NETWORK_MIGRATE_X — the peer
    // lives on DC X and the client must migrate.
    for prefix in ["PHONE_MIGRATE_", "USER_MIGRATE_", "NETWORK_MIGRATE_"] {
        if let Some(dc_str) = error_message.strip_prefix(prefix)
            && let Ok(dc_id) = dc_str.parse::<i32>()
        {
            return Error::Migration { dc_id };
        }
    }

    if let Some(dc_str) = error_message.strip_prefix("MIGRATE_")
        && let Ok(dc_id) = dc_str.parse::<i32>()
    {
        return Error::Migration { dc_id };
    }

    Error::Rpc {
        error_code,
        error_message: error_message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flood_wait_is_transient() {
        let err = Error::FloodWait {
            seconds: 60,
            retry_after: Instant::now(),
        };
        assert!(err.is_transient());
        assert!(!err.is_auth_error());
    }

    #[test]
    fn test_network_is_transient() {
        let err = Error::Network(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "refused",
        ));
        assert!(err.is_transient());
    }

    #[test]
    fn test_rpc_401_is_transient() {
        let err = Error::Rpc {
            error_code: 401,
            error_message: "unauthorized".into(),
        };
        assert!(err.is_transient());
    }

    #[test]
    fn test_rpc_400_not_transient() {
        let err = Error::Rpc {
            error_code: 400,
            error_message: "bad request".into(),
        };
        assert!(!err.is_transient());
    }

    #[test]
    fn test_migration_dc_id() {
        let err = Error::Migration { dc_id: 2 };
        assert_eq!(err.dc_id(), Some(2));
    }

    #[test]
    fn test_classify_user_migrate() {
        let e = classify_rpc_error(303, "USER_MIGRATE_5");
        assert!(matches!(e, Error::Migration { dc_id: 5 }));
        let e = classify_rpc_error(303, "PHONE_MIGRATE_2");
        assert!(matches!(e, Error::Migration { dc_id: 2 }));
        let e = classify_rpc_error(303, "NETWORK_MIGRATE_4");
        assert!(matches!(e, Error::Migration { dc_id: 4 }));
        assert_eq!(e.dc_id(), Some(4));
    }

    #[test]
    fn test_auth_key_invalid_is_auth_error() {
        let err = Error::AuthKeyInvalid { dc_id: 2, key_short: "abcd".into() };
        assert!(err.is_auth_error());
        assert!(!err.is_transient());
    }

    #[test]
    fn test_display_format() {
        let err = Error::Rpc {
            error_code: 400,
            error_message: "PEER_ID_INVALID".into(),
        };
        let s = err.to_string();
        assert!(s.contains("PEER_ID_INVALID"));
        assert!(s.contains("400"));
    }
}
