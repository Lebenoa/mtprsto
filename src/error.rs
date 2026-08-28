use std::fmt;

/// Result type alias for the MTProto library.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur in the MTProto library.
#[derive(Debug)]
pub enum Error {
    /// IO error from networking.
    Io(std::io::Error),
    /// Cryptographic error.
    Crypto(String),
    /// Serialization/deserialization error.
    Serialization(String),
    /// Transport layer error.
    Transport(String),
    /// MTProto protocol error.
    Protocol(String),
    /// API-level error.
    Api {
        error_code: i32,
        error_message: String,
    },
    /// The DH parameters failed verification.
    DhVerification(String),
    /// Server returned an unexpected response.
    UnexpectedResponse(String),
    /// The auth key was not yet created.
    NoAuthKey,
    /// Padding verification failed.
    PaddingError(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "IO error: {e}"),
            Error::Crypto(msg) => write!(f, "Crypto error: {msg}"),
            Error::Serialization(msg) => write!(f, "Serialization error: {msg}"),
            Error::Transport(msg) => write!(f, "Transport error: {msg}"),
            Error::Protocol(msg) => write!(f, "Protocol error: {msg}"),
            Error::Api {
                error_code,
                error_message,
            } => write!(f, "API error {error_code}: {error_message}"),
            Error::DhVerification(msg) => write!(f, "DH verification failed: {msg}"),
            Error::UnexpectedResponse(msg) => write!(f, "Unexpected response: {msg}"),
            Error::NoAuthKey => write!(f, "No authorization key"),
            Error::PaddingError(msg) => write!(f, "Padding error: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
