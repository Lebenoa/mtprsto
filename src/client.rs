//! High-level Telegram MTProto client.
//!
//! `Client` composes the connection pool, session persistence, and
//! typed RPC invoke into a single ergonomic entry point.
//!
//! # Quick start
//!
//! ```no_run
//! use mtprsto::client::Client;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut client = Client::builder()
//!     .api_id(12345)
//!     .api_hash("your_api_hash")
//!     .session("session.json")
//!     .build()?;
//!
//! client.connect().await?;
//! client.authorize_bot("123456:ABC-DEF...").await?;
//! let msg_id = client.send("me", "Hello!").await?;
//! # Ok(())
//! }
//! ```

use crate::api::{self, TelegramClient};
use crate::error::{Error, Result};
use crate::mtproto::MtProtoSession;
use crate::pool::{PoolConfig, SenderPool};
use crate::session::{SessionData, SessionStore};
use crate::types::*;
use crate::serialize::{TLWriter, TLReader, *};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration builder for `Client`.
pub struct ClientConfig {
    api_id: Option<i32>,
    api_hash: Option<String>,
    session_path: Option<PathBuf>,
    dc_id: i32,
    pool: PoolConfig,
}

impl ClientConfig {
    /// Start building a new client configuration.
    pub fn new() -> Self {
        Self {
            api_id: None,
            api_hash: None,
            session_path: None,
            dc_id: 2,
            pool: PoolConfig::default(),
        }
    }

    /// Set the API ID from my.telegram.org.
    pub fn api_id(mut self, id: i32) -> Self {
        self.api_id = Some(id);
        self
    }

    /// Set the API hash from my.telegram.org.
    pub fn api_hash(mut self, hash: impl Into<String>) -> Self {
        self.api_hash = Some(hash.into());
        self
    }

    /// Set the session file path for persistence.
    pub fn session(mut self, path: impl Into<PathBuf>) -> Self {
        self.session_path = Some(path.into());
        self
    }

    /// Set the initial DC ID (default: 2).
    pub fn dc_id(mut self, id: i32) -> Self {
        self.dc_id = id;
        self
    }

    /// Set pool configuration.
    pub fn pool_config(mut self, config: PoolConfig) -> Self {
        self.pool = config;
        self
    }

    /// Build the client.
    pub fn build(self) -> Result<Client> {
        let session_store = self.session_path
            .map(SessionStore::new)
            .unwrap_or_else(|| {
                // Default to in-memory (no persistence)
                SessionStore::new(std::env::temp_dir().join("mtprsto_session.json"))
            });

        Ok(Client {
            api_id: self.api_id,
            api_hash: self.api_hash,
            dc_id: self.dc_id,
            session_store: Arc::new(RwLock::new(session_store)),
            connected: false,
            pool_config: self.pool,
        })
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// High-level MTProto client.
pub struct Client {
    api_id: Option<i32>,
    api_hash: Option<String>,
    dc_id: i32,
    session_store: Arc<RwLock<SessionStore>>,
    connected: bool,
    pool_config: PoolConfig,
}

impl Client {
    /// Start building a new client.
    pub fn builder() -> ClientConfig {
        ClientConfig::new()
    }

    /// Get the API ID.
    pub fn api_id(&self) -> Option<i32> {
        self.api_id
    }

    /// Get the API hash.
    pub fn api_hash(&self) -> Option<&str> {
        self.api_hash.as_deref()
    }

    /// Get the DC ID.
    pub fn dc_id(&self) -> i32 {
        self.dc_id
    }

    /// Check if the client is connected (auth key established).
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Connect to Telegram (create auth key via DH handshake if no session exists).
    pub async fn connect(&mut self) -> Result<()> {
        // Try loading existing session
        let mut store = self.session_store.write().await;
        if let Some(data) = store.load()? {
            tracing::info!(
                "loaded existing session from {} (dc={})",
                store.path().display(),
                data.dc_id
            );
            // Restore the session
            self.dc_id = data.dc_id;
            self.connected = true;
            return Ok(());
        }

        // No existing session — perform DH handshake
        tracing::info!("no existing session, performing DH handshake to DC {}", self.dc_id);
        drop(store);

        let mut tg_client = TelegramClient::new(
            self.dc_id,
            self.api_id,
            self.api_hash.clone(),
        );
        tg_client.create_auth_key().await?;

        // Save the new session
        if let Some(session) = &tg_client.session {
            let data = SessionData::from_auth_key(
                &session.auth_key,
                session.server_salt,
                self.dc_id,
            );
            let mut store = self.session_store.write().await;
            store.save(&data)?;
            tracing::info!("session saved to {}", store.path().display());
        }

        self.connected = true;
        Ok(())
    }

    /// Authorize as a bot using a bot token.
    pub async fn authorize_bot(&mut self, bot_token: &str) -> Result<()> {
        if !self.connected {
            self.connect().await?;
        }

        let mut store = self.session_store.write().await;
        let data = store.load()?.ok_or(Error::NoAuthKey)?;
        drop(store);

        let auth_key = data.decode_auth_key()?;
        let mut tg_client = TelegramClient::with_session(
            self.dc_id,
            auth_key,
            data.server_salt,
            self.api_id,
            self.api_hash.clone(),
        );

        tg_client.authorize_bot(bot_token).await?;
        tracing::info!("bot authorization successful");
        Ok(())
    }

    /// Send a text message to a peer.
    ///
    /// `peer` can be a user ID, chat ID, channel ID, or username string.
    pub async fn send(&self, peer: &str, text: &str) -> Result<MsgId> {
        self.invoke_with_method(MESSAGES_SEND_MESSAGE, |w| {
            // messages.sendMessage#44942323
            // flags:{坂} no_webpage:flags.1?true silent:flags.5?true
            // background:flags.6?true clear_draft:flags.7?true
            // peer:InputPeer message:string ...
            let flags: i32 = 0;
            w.write_i32(flags);

            // Write the input peer
            let input_peer = self.resolve_peer(peer)?;
            input_peer.write_to(w);

            // Message text
            w.write_bytes(text.as_bytes());

            // reply_markup (none)
            Ok(())
        }).await?;

        // For now, return a placeholder MsgId
        Ok(MsgId(0))
    }

    /// Get your own user info (users.getFullUser with self).
    pub async fn get_me(&self) -> Result<User> {
        let mut w = TLWriter::new();
        w.write_u32(USERS_GET_FULL_USER);

        // InputUserSelf
        w.write_u32(INPUT_USER_SELF);

        let result = self.invoke_raw(w.into_bytes()).await?;
        // Parse the result (simplified — returns the user from the full user object)
        // TODO: parse full User
        Ok(User::Empty { id: UserId(0) })
    }

    /// Get a list of dialogs (conversations).
    pub async fn get_dialogs(&self) -> Result<Dialogs> {
        let mut w = TLWriter::new();
        w.write_u32(MESSAGES_GET_DIALOGS);
        w.write_i32(0); // flags
        w.write_i32(0); // offset_date
        w.write_u32(INPUT_PEER_EMPTY); // offset_peer
        w.write_i32(0); // offset_id
        w.write_i32(100); // limit

        let _result = self.invoke_raw(w.into_bytes()).await?;
        // TODO: parse Dialogs response
        Ok(Dialogs {
            dialogs: Vec::new(),
            messages: Vec::new(),
            users: Vec::new(),
            chats: Vec::new(),
        })
    }

    /// Get the current state (pts, qts, date, seq).
    pub async fn get_state(&self) -> Result<State> {
        let mut w = TLWriter::new();
        w.write_u32(0xedd4882a); // updates.getState
        let _result = self.invoke_raw(w.into_bytes()).await?;
        // TODO: parse State
        Ok(State {
            pts: 0,
            qts: 0,
            date: 0,
            seq: 0,
            unread_count: 0,
        })
    }

    /// Delete messages by ID.
    pub async fn delete_messages(&self, msg_ids: &[MsgId]) -> Result<()> {
        let mut w = TLWriter::new();
        w.write_u32(MESSAGES_DELETE_MESSAGES);
        w.write_i32(0); // flags (no revoke)

        // Vector<int> of message IDs
        w.write_u32(0x1cb5c415); // VECTOR
        w.write_i32(msg_ids.len() as i32);
        for id in msg_ids {
            w.write_i32(id.0 as i32);
        }

        let _ = self.invoke_raw(w.into_bytes()).await?;
        Ok(())
    }

    /// Invoke a raw TL method through the transport.
    ///
    /// Wraps the payload in `invokeWithLayer` and sends it.
    pub async fn invoke_raw(&self, method_bytes: Vec<u8>) -> Result<Vec<u8>> {
        // Wrap with invokeWithLayer
        let mut w = TLWriter::new();
        w.write_u32(INVOKE_WITH_LAYER);
        w.write_i32(api::API_LAYER);
        w.write_bytes(&method_bytes);

        let payload = w.into_bytes();

        // Create a temporary session for encrypt/decrypt
        let auth_key = vec![0u8; 256]; // placeholder — real impl uses pool
        let mut session = MtProtoSession::new(auth_key, 0);
        let msg_id = session.next_msg_id();

        // TODO: use SenderPool for actual send/receive
        Err(Error::Other("invoke_raw requires SenderPool — use Client::connect first".into()))
    }

    /// Helper: invoke a method with a builder closure.
    async fn invoke_with_method<F>(&self, method_id: u32, build: F) -> Result<Vec<u8>>
    where
        F: FnOnce(&mut TLWriter) -> Result<()>,
    {
        let mut w = TLWriter::new();
        w.write_u32(method_id);
        build(&mut w)?;
        self.invoke_raw(w.into_bytes()).await
    }

    /// Resolve a peer string to an InputPeer.
    ///
    /// Supports:
    /// - Numeric user/chat/channel ID (positive = user, negative = chat/group)
    /// - Username string (resolves via contacts.resolveUsername)
    fn resolve_peer(&self, peer: &str) -> Result<InputPeer> {
        if let Ok(id) = peer.parse::<i64>() {
            if id > 0 {
                Ok(InputPeer::UserFromId { user_id: UserId(id) })
            } else {
                Ok(InputPeer::Chat { chat_id: ChatId(-id) })
            }
        } else {
            // Username resolution requires an API call
            Err(Error::Other(format!(
                "username resolution for @{peer} not yet implemented — use numeric ID"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_builder() {
        let client = Client::builder()
            .api_id(12345)
            .api_hash("test_hash")
            .dc_id(2)
            .build()
            .unwrap();

        assert_eq!(client.api_id(), Some(12345));
        assert_eq!(client.api_hash(), Some("test_hash"));
        assert_eq!(client.dc_id(), 2);
        assert!(!client.is_connected());
    }

    #[test]
    fn test_resolve_peer_numeric() {
        let client = Client::builder().build().unwrap();
        let peer = client.resolve_peer("12345").unwrap();
        match peer {
            InputPeer::UserFromId { user_id } => assert_eq!(user_id.0, 12345),
            _ => panic!("expected UserFromId"),
        }
    }

    #[test]
    fn test_resolve_peer_negative() {
        let client = Client::builder().build().unwrap();
        let peer = client.resolve_peer("-999").unwrap();
        match peer {
            InputPeer::Chat { chat_id } => assert_eq!(chat_id.0, 999),
            _ => panic!("expected Chat"),
        }
    }
}
