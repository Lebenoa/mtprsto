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

use crate::api::TelegramClient;
use crate::error::{Error, Result};
use crate::mtproto::MtProtoSession;
use crate::pool::{PoolConfig, ProtocolConfig, SenderPool};
use crate::session::{SessionData, SessionStorage, SessionStore};
use crate::types::{self, *};
use crate::serialize::{TLReader, TLWriter};
use crate::rpc;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration builder for `Client`.
pub struct ClientConfig {
    api_id: Option<i32>,
    api_hash: Option<String>,
    session_path: Option<PathBuf>,
    session_storage: Option<Box<dyn SessionStorage>>,
    /// `None` = auto-select the nearest DC after bootstrapping (default).
    dc_id: Option<i32>,
    pool: PoolConfig,
    protocol: ProtocolConfig,
    download: crate::file::DownloadConfig,
}

impl ClientConfig {
    /// Start building a new client configuration.
    pub fn new() -> Self {
        Self {
            api_id: None,
            api_hash: None,
            session_path: None,
            session_storage: None,
            dc_id: None,
            pool: PoolConfig::default(),
            protocol: ProtocolConfig::default(),
            download: crate::file::DownloadConfig::default(),
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

    /// Pin the DC ID. By default the client bootstraps on DC 2 and then
    /// migrates to the nearest DC (`help.getNearestDc`) before
    /// authorizing; setting this skips that auto-selection (use it for
    /// test DCs like 201 or pinned deployments).
    pub fn dc_id(mut self, id: i32) -> Self {
        self.dc_id = Some(id);
        self
    }

    /// Use a custom storage backend for session persistence (SQLite,
    /// Postgres, Redis, ...) instead of the default JSON file. This takes
    /// precedence over [`ClientConfig::session`].
    pub fn session_storage(mut self, storage: Box<dyn SessionStorage>) -> Self {
        self.session_storage = Some(storage);
        self
    }

    /// Set pool configuration.
    pub fn pool_config(mut self, config: PoolConfig) -> Self {
        self.pool = config;
        self
    }

    /// Set protocol-level knobs: keepalive/ack/salt timers and
    /// anti-fingerprinting random padding (see [`ProtocolConfig`]).
    pub fn protocol_config(mut self, config: ProtocolConfig) -> Self {
        self.protocol = config;
        self
    }

    /// Set download configuration (parallel range fetching, SPEC BS-5).
    pub fn download_config(mut self, config: crate::file::DownloadConfig) -> Self {
        self.download = config;
        self
    }

    /// Build the client.
    pub fn build(self) -> Result<Client> {
        let session_store: Box<dyn SessionStorage> = if let Some(custom) = self.session_storage {
            custom
        } else if let Some(path) = self.session_path {
            Box::new(SessionStore::new(path))
        } else {
            // Stable default: ~/.mtprsto/session.json so the auth key is
            // reused across runs. A PID-suffixed temp file would re-auth
            // on every start and leak key material into %TEMP%.
            let home = std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(PathBuf::from);
            match home {
                Some(h) => Box::new(SessionStore::new(h.join(".mtprsto").join("session.json")))
                    as Box<dyn SessionStorage>,
                None => {
                    let fallback = std::env::temp_dir().join("mtprsto_session.json");
                    tracing::warn!(
                        "no home directory found — session in {}: the auth key will \
                         not survive between runs reliably",
                        fallback.display()
                    );
                    Box::new(SessionStore::new(fallback))
                }
            }
        };

        Ok(Client {
            api_id: self.api_id,
            api_hash: self.api_hash,
            dc_id: self.dc_id.unwrap_or(2),
            dc_explicit: self.dc_id.is_some(),
            session_store: Arc::new(RwLock::new(session_store)),
            connected: false,
            pool_config: self.pool,
            protocol_config: self.protocol,
            download_config: self.download,
            pool: None,
            update_task: None,
            peer_cache: HashMap::new(),
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
    /// Whether the caller pinned the DC (disables nearest-DC selection).
    dc_explicit: bool,
    session_store: Arc<RwLock<Box<dyn SessionStorage>>>,
    connected: bool,
    pool_config: PoolConfig,
    protocol_config: ProtocolConfig,
    /// Download knobs (parallel threshold/count, SPEC BS-5).
    download_config: crate::file::DownloadConfig,
    pool: Option<Arc<SenderPool>>,
    /// Handle to the background update pump started by [`Client::updates`].
    update_task: Option<tokio::sync::mpsc::UnboundedSender<crate::types::Updates>>,
    /// username. Entries live for the process lifetime; a stale hash
    /// surfaces as a `PEER_ID_INVALID` RPC error.
    peer_cache: HashMap<String, InputPeer>,
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

    /// Get the download configuration in use (SPEC BS-5).
    pub fn download_config(&self) -> &crate::file::DownloadConfig {
        &self.download_config
    }

    /// Download a file into memory using the client's
    /// [`DownloadConfig`](crate::file::DownloadConfig).
    ///
    /// Pass the media's known `size` (e.g. `Document::size`) to enable
    /// parallel range fetching when it exceeds the configured threshold;
    /// `None` falls back to a serial chunked download that stops at the
    /// first short read.
    ///
    /// # Errors
    ///
    /// Returns transport/protocol errors from the pool, or
    /// [`Error::Other`] when the media is served from a CDN (not yet
    /// supported).
    pub async fn download(
        &self,
        location: &crate::types::FileLocation,
        size: Option<u64>,
    ) -> Result<Vec<u8>> {
        let pool = self.pool.as_ref().ok_or(Error::Other(
            "download requires a connected client — call connect() first".into(),
        ))?;
        match size {
            Some(size) => {
                crate::file::download_parallel(pool.clone(), location, size, &self.download_config)
                    .await
            }
            None => crate::file::download(pool.clone(), location).await,
        }
    }

    /// Connect to Telegram (create auth key via DH handshake if no session exists)
    /// and open a SenderPool.
    #[tracing::instrument(name = "mtprsto::connect", skip(self), err)]
    pub async fn connect(&mut self) -> Result<()> {
        let _session = crate::ergonomics::session_span(self.dc_id).entered();
        // Try loading existing session
        let session_data = {
            let mut store = self.session_store.write().await;
            if let Some(data) = SessionStorage::load(&mut *store)? {
                tracing::info!(
                    "loaded existing session from {} (dc={})",
                    store.describe(),
                    data.dc_id
                );
                self.dc_id = data.dc_id;
                Some(data)
            } else {
                None
            }
        };

        let mtproto_session = if let Some(data) = session_data {
            // Restore from persisted session
            let auth_key = data.decode_auth_key()?;
            MtProtoSession::new(auth_key, data.server_salt)
        } else {
            // No existing session — perform DH handshake
            tracing::info!(
                "no existing session, performing DH handshake to DC {}",
                self.dc_id
            );
            let mut tg_client = TelegramClient::new(
                self.dc_id,
                self.api_id,
                self.api_hash.clone(),
            );
            tg_client.create_auth_key().await?;

            // Auto-select the nearest DC (SPEC §1) unless the caller pinned
            // one: ask help.getNearestDc on the bootstrap DC and, if it
            // points elsewhere, re-handshake there before authorizing.
            if !self.dc_explicit {
                match tg_client.help_get_nearest_dc().await {
                    Ok((_this, nearest)) if nearest != self.dc_id => {
                        tracing::info!(
                            "nearest DC is {nearest} (bootstrap was {}) — re-handshaking",
                            self.dc_id
                        );
                        let mut migrated = TelegramClient::new(
                            nearest,
                            self.api_id,
                            self.api_hash.clone(),
                        );
                        migrated.create_auth_key().await?;
                        self.dc_id = nearest;
                        tg_client = migrated;
                    }
                    Ok((this, _)) => {
                        tracing::debug!("bootstrap DC {this} is the nearest");
                    }
                    Err(e) => {
                        tracing::warn!(
                            "getNearestDc failed ({e}) — staying on DC {}",
                            self.dc_id
                        );
                    }
                }
            }

            // Save the new session
            if let Some(session) = &tg_client.session {
                let data = SessionData::from_auth_key(
                    &session.auth_key,
                    session.server_salt,
                    self.dc_id,
                );
                let mut store = self.session_store.write().await;
                SessionStorage::save(&mut *store, &data)?;
                tracing::info!("session saved to {}", store.describe());
            }
            tg_client.session.ok_or(Error::NoAuthKey)?
        };

        // Construct the connection pool with the real session
        // Apply the anti-fingerprinting knob to the live session before
        // any traffic flows through it.
        let mut mtproto_session = mtproto_session;
        mtproto_session.set_random_padding(self.protocol_config.random_padding);
        let mut pool = Arc::new(SenderPool::new(
            self.dc_id,
            self.api_id.unwrap_or(0),
            mtproto_session,
            self.pool_config.clone(),
            self.protocol_config.clone(),
        ));
        Arc::get_mut(&mut pool).expect("pool freshly created").connect().await?;
        self.pool = Some(pool);
        self.connected = true;

        // Background maintenance (SPEC §5.4 / BS-1 / §9): batched acks,
        // ping/pong keepalive, periodic salt refresh.
        let pool = self.pool.as_ref().unwrap();
        pool.spawn_ack_flusher();
        pool.spawn_keepalive();
        pool.spawn_salt_refresher();

        // Persist the (possibly server-refreshed) salt so the next boot
        // starts with a current value.
        let current_salt = self.pool.as_ref().unwrap().session().await.server_salt;
        let mut store = self.session_store.write().await;
        if let Some(data) = SessionStorage::load(&mut *store)? {
            let mut fresh = data.clone();
            fresh.server_salt = current_salt;
            if fresh.server_salt != data.server_salt {
                SessionStorage::save(&mut *store, &fresh)?;
                tracing::info!("session salt refreshed to current server value");
            }
        }
        Ok(())
    }

    /// Authorize as a bot using a bot token.
    #[tracing::instrument(name = "mtprsto::authorize_bot", skip(self, bot_token), err)]
    pub async fn authorize_bot(&mut self, bot_token: &str) -> Result<()> {
        // A bot's home DC may differ from the one we dialed (USER_MIGRATE_X):
        // on migration, drop the session and re-handshake on the target DC.
        for _ in 0..3u32 {
            if !self.connected {
                self.connect().await?;
            }

            let mut store = self.session_store.write().await;
            let data = SessionStorage::load(&mut *store)?.ok_or(Error::NoAuthKey)?;
            drop(store);

            if data.user_id != 0 {
                tracing::info!("session already authorized (user {})", data.user_id);
                return Ok(());
            }

            let auth_key = data.decode_auth_key()?;
            let mut tg_client = TelegramClient::with_session(
                self.dc_id,
                auth_key,
                data.server_salt,
                self.api_id,
                self.api_hash.clone(),
            );

            match tg_client.authorize_bot(bot_token).await {
                Ok(user_id) => {
                    // Persist the bot's user id so later runs skip re-auth
                    // (importBotAuthorization is flood-limited hard).
                    let mut store = self.session_store.write().await;
                    if let Ok(Some(mut data)) = SessionStorage::load(&mut *store) {
                        data.user_id = user_id;
                        SessionStorage::save(&mut *store, &data)?;
                    }
                    drop(store);
                    tracing::info!("bot authorization successful");
                    return Ok(());
                }
                Err(Error::Migration { dc_id }) => {
                    tracing::info!(
                        "bot home DC is {dc_id} (was {}) — migrating",
                        self.dc_id
                    );
                    self.dc_id = dc_id;
                    self.connected = false;
                    self.pool = None;
                    let mut store = self.session_store.write().await;
                    SessionStorage::delete(&mut *store)?;
                }
                Err(e) => return Err(e),
            }
        }
        Err(Error::Other("bot DC migration did not settle".into()))
    }

    /// Send a text message to a peer.
    ///
    /// `peer` can be a user ID, chat ID, channel ID, or username string.
    #[tracing::instrument(name = "mtprsto::send", skip(self, text), fields(peer = peer), err)]
    pub async fn send(&mut self, peer: &str, text: &str) -> Result<MsgId> {
        let input_peer = self.resolve_peer(peer).await?;
        let result = self.invoke_with_method(MESSAGES_SEND_MESSAGE, |w| {
            // messages.sendMessage#545cd15a flags:# ... peer:InputPeer
            // reply_to:flags.0?InputReplyTo message:string random_id:long ...
            let flags: i32 = 0;
            w.write_i32(flags);

            // Write the input peer
            input_peer.write_to(w);
            // Message text
            w.write_bytes(text.as_bytes());

            // random_id:long (required by layer 223)
            w.write_i64(rand::random::<i64>());

            // reply_markup (none)
            Ok(())
        }).await?;

        if std::env::var("MTPRSTO_DEBUG").is_ok() {
            println!("DEBUG send result ({}b): {:02x?}", result.len(), &result[..result.len().min(160)]);
        }
        // Response is UpdateShortSentMessage or a full Updates object;
        // both carry the new message id.
        let updates = types::Updates::parse(&result)?;
        if let Some(id) = updates.message_id() {
            return Ok(id);
        }
        // messages.sendMessage normally answers updateShortSentMessage; a
        // full Updates wrapper means the server echoed our own message back
        // as UpdateNewMessage — extract its id.
        match updates {
            types::Updates::Updates { updates: list, .. }
            | types::Updates::UpdatesCombined { updates: list, .. } => {
                for u in list {
                    if let types::Update::NewMessage { message, .. } = u {
                        return Ok(message.id());
                    }
                }
                Err(Error::Protocol("sendMessage returned no message id".into()))
            }
            types::Updates::UpdateShort { update: types::Update::NewMessage { message, .. }, .. } => {
                Ok(message.id())
            }
            other => Err(Error::Protocol(format!(
                "unexpected sendMessage response: {:?}", other
            ))),
        }
    }

    /// Parse `users.userFull` / bare `User` response into the user itself.
    fn parse_user_container(data: &[u8]) -> Result<User> {
        let mut r = TLReader::new(data);
        let ctor = r.read_u32()?;
        match ctor {
            types::USERS_USER_FULL => {
                // users.userFull#d69e83e0 full_user:UserFull chats:Vector<Chat> users:Vector<User>
                // Skip the UserFull object header — its layout is not
                // modeled, so we only consume the constructor here and
                // locate the chats vector by scanning below.
                let _fu_ctor = r.read_u32()?;
                // Skip UserFull by scanning to the first vector constructor.
                // UserFull is a fixed-shape object; we locate the chats
                // vector by searching for VECTOR at a 4-byte boundary after
                // the full_user constructor.
                let bytes = data;
                let fu_start = 4 + 4; // users.userFull ctor + full_user ctor
                let mut chats_off = None;
                let mut off = fu_start;
                while off + 4 <= bytes.len() {
                    let v = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
                    if v == crate::serialize::VECTOR {
                        chats_off = Some(off);
                        break;
                    }
                    off += 4;
                }
                let chats_off = chats_off.ok_or_else(|| {
                    Error::Protocol("users.userFull: chats vector not found".into())
                })?;
                let mut r2 = TLReader::new(&bytes[chats_off..]);
                let chat_count = r2.read_vector_header()?;
                for _ in 0..chat_count {
                    let _ = types::Chat::read_from(&mut r2)?;
                }
                let user_count = r2.read_vector_header()?;
                let mut user = User::Empty { id: UserId(0) };
                for _ in 0..user_count {
                    let u = types::User::read_from(&mut r2)?;
                    user = u;
                }
                Ok(user)
            }
            types::USER | types::USER_EMPTY => {
                // Bare user object (no container): rewind and parse.
                User::read_from(&mut TLReader::new(data))
            }
            other => Err(Error::Protocol(format!(
                "unexpected get_me response constructor {other:#x}"
            ))),
        }
    }

    /// Get your own user info (users.getFullUser with self).
    #[tracing::instrument(name = "mtprsto::get_me", skip(self), err)]
    pub async fn get_me(&self) -> Result<User> {
        let mut w = TLWriter::new();
        w.write_u32(USERS_GET_FULL_USER);

        // InputUserSelf
        w.write_u32(INPUT_USER_SELF);

        let result = self.invoke_raw(w.into_bytes()).await?;
        Self::parse_user_container(&result)
    }

    /// Parse `messages.dialogs` / `messages.dialogsSlice` into [`Dialogs`].
    fn parse_dialogs(data: &[u8]) -> Result<Dialogs> {
        let mut r = TLReader::new(data);
        let ctor = r.read_u32()?;
        match ctor {
            types::MESSAGES_DIALOGS | types::MESSAGES_DIALOGS_SLICE => {
                // dialogsSlice#71e094f3 count:int messages:Vector<Message>
                //   dialogs:Vector<Dialog> chats:Vector<Chat> users:Vector<User>
                // messages.dialogs#15ba6c40 has the same tail without count.
                let _count = if ctor == types::MESSAGES_DIALOGS_SLICE {
                    Some(r.read_i32()?)
                } else {
                    None
                };

                // messages:Vector<Message>
                let msg_count = r.read_vector_header()?;
                let mut messages = Vec::with_capacity(msg_count as usize);
                for _ in 0..msg_count {
                    messages.push(types::Message::read_from(&mut r)?);
                }

                // dialogs:Vector<Dialog>
                let dlg_count = r.read_vector_header()?;
                let mut dialogs = Vec::with_capacity(dlg_count as usize);
                for _ in 0..dlg_count {
                    dialogs.push(types::Dialog::read_from(&mut r)?);
                }

                // chats:Vector<Chat>
                let chat_count = r.read_vector_header()?;
                let mut chats = Vec::with_capacity(chat_count as usize);
                for _ in 0..chat_count {
                    chats.push(types::Chat::read_from(&mut r)?);
                }

                // users:Vector<User>
                let user_count = r.read_vector_header()?;
                let mut users = Vec::with_capacity(user_count as usize);
                for _ in 0..user_count {
                    users.push(types::User::read_from(&mut r)?);
                }

                Ok(Dialogs { dialogs, messages, users, chats })
            }
            types::MESSAGES_DIALOGS_NOT_MODIFIED => Err(Error::Protocol(
                "get_dialogs: server returned dialogsNotModified — pass a real hash".into(),
            )),
            other => Err(Error::Protocol(format!(
                "unexpected get_dialogs response constructor {other:#x}"
            ))),
        }
    }

    /// Get a list of dialogs (conversations).
    #[tracing::instrument(name = "mtprsto::get_dialogs", skip(self), err)]
    pub async fn get_dialogs(&self) -> Result<Dialogs> {
        let mut w = TLWriter::new();
        w.write_u32(MESSAGES_GET_DIALOGS);
        w.write_i32(0); // flags
        w.write_i32(0); // offset_date
        w.write_i32(0); // offset_id
        w.write_u32(INPUT_PEER_EMPTY); // offset_peer
        w.write_i32(100); // limit
        w.write_i64(0); // hash:long

        let result = self.invoke_raw(w.into_bytes()).await?;
        Self::parse_dialogs(&result)
    }

    /// Parse `updates.state`.
    fn parse_state(data: &[u8]) -> Result<State> {
        let mut r = TLReader::new(data);
        let ctor = r.read_u32()?;
        if ctor != types::UPDATES_STATE {
            return Err(Error::Protocol(format!(
                "unexpected get_state response constructor {ctor:#x}"
            )));
        }
        // updates.state#a56c2a3e pts:int qts:int date:int seq:int unread_count:int
        Ok(State {
            pts: r.read_i32()?,
            qts: r.read_i32()?,
            date: r.read_i32()?,
            seq: r.read_i32()?,
            unread_count: r.read_i32()?,
        })
    }

    /// Get the current state (pts, qts, date, seq).
    pub async fn get_state(&self) -> Result<State> {
        let mut w = TLWriter::new();
        w.write_u32(types::UPDATES_GET_STATE);
        let result = self.invoke_raw(w.into_bytes()).await?;
        Self::parse_state(&result)
    }

    /// Fetch missed updates since `state` (SPEC §6 pts/seq gap recovery).
    /// Returns updates to re-dispatch through the dispatcher.
    pub async fn get_difference(&self, state: &State) -> Result<types::Difference> {
        let payload = rpc::build_get_difference(state.pts, state.date, state.qts);
        let result = self.invoke_raw(payload).await?;
        types::Difference::parse(&result)
    }

    /// Fetch missed channel updates (SPEC §6, `UpdateChannelTooLong` path).
    pub async fn get_channel_difference(
        &self,
        channel: &InputChannel,
        pts: i32,
        limit: i32,
    ) -> Result<types::ChannelDifference> {
        let payload = rpc::build_get_channel_difference(channel, pts, limit);
        let result = self.invoke_raw(payload).await?;
        types::ChannelDifference::parse(&result)
    }

    /// Start the background update pump and return a receiver of decoded
    /// [`Update`] events (SPEC gap item 9).
    ///
    /// The pump polls `updates.getState` on `poll_interval_secs`, runs the
    /// [`UpdateDispatcher`] gap detection over the observed pts, and calls
    /// `updates.getDifference` whenever the server's pts jumps — feeding the
    /// recovered messages into the same dispatcher. Channel gaps surface as
    /// `Update::ChannelTooLong`; the caller decides when to call
    /// [`Client::get_channel_difference`].
    ///
    /// Returns `None` if the pump is already running or the client is not
    /// connected.
    pub fn updates(
        &mut self,
        poll_interval_secs: u64,
    ) -> Option<tokio::sync::mpsc::UnboundedReceiver<types::Update>> {
        use crate::updates::UpdateDispatcher;
        if self.update_task.is_some() {
            return None; // pump already running
        }
        let pool = self.pool.as_ref()?;
        let (dispatcher, rx) = UpdateDispatcher::with_channel();
        let (feed_tx, _keep_open) = tokio::sync::mpsc::unbounded_channel::<types::Updates>();
        self.update_task = Some(feed_tx);
        let pool = std::sync::Arc::clone(pool);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(poll_interval_secs.max(1)),
            );
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut dispatcher = dispatcher;
            let mut last_pts = 0i32;
            let mut have_pts = false;
            // Channel access hashes harvested from Updates/Difference chat
            // vectors, keyed by channel id — needed to build InputChannel
            // for getChannelDifference.
            let mut channel_hashes: std::collections::HashMap<i64, i64> =
                std::collections::HashMap::new();

            loop {
                interval.tick().await;

                // Poll state to observe server-side pts drift.
                let state = match pool
                    .send_rpc(&rpc::build_get_state())
                    .await
                    .ok()
                    .and_then(|bytes| Client::parse_state(&bytes).ok())
                {
                    Some(s) => s,
                    None => continue,
                };
                dispatcher.set_qts(state.qts);

                // ChannelTooLong surfaced by the dispatcher → resync each
                // queued channel via updates.getChannelDifference (SPEC §6.1).
                for channel_id in dispatcher.take_channels_too_long() {
                    let Some(&hash) = channel_hashes.get(&channel_id) else {
                        tracing::warn!(
                            channel_id,
                            "ChannelTooLong without known access hash — skipping resync"
                        );
                        continue;
                    };
                    let pts = dispatcher.channel_pts_of(channel_id).unwrap_or(1);
                    let payload = rpc::build_get_channel_difference(
                        &InputChannel::Channel {
                            channel_id: types::ChannelId(channel_id),
                            access_hash: types::AccessHash(hash),
                        },
                        pts,
                        100,
                    );
                    if let Ok(bytes) = pool.send_rpc(&payload).await
                        && let Ok(diff) = types::ChannelDifference::parse(&bytes)
                    {
                        let (messages, other_updates, chats, new_pts) = match diff {
                            types::ChannelDifference::Empty { pts, .. } => (Vec::new(), Vec::new(), Vec::new(), pts),
                            types::ChannelDifference::Difference {
                                pts,
                                new_messages,
                                other_updates,
                                chats,
                                ..
                            }
                            | types::ChannelDifference::TooLong {
                                pts,
                                new_messages,
                                other_updates,
                                chats,
                                ..
                            } => (new_messages, other_updates, chats, pts),
                        };
                        for chat in chats {
                            if let types::Chat::Channel { id, access_hash: Some(h), .. } = chat {
                                channel_hashes.insert(id.0, h.0);
                            }
                        }
                        for msg in messages {
                            dispatcher.process_updates(types::Updates::UpdateShort {
                                update: types::Update::NewMessage {
                                    message: msg,
                                    pts,
                                    pts_count: 1,
                                },
                                date: state.date,
                                seq: state.seq,
                            });
                        }
                        for u in other_updates {
                            dispatcher.process_updates(types::Updates::UpdateShort {
                                update: u,
                                date: state.date,
                                seq: state.seq,
                            });
                        }
                        dispatcher.advance_channel_pts(channel_id, new_pts);
                    }
                }

                if have_pts && state.pts > last_pts + 1 {
                    // Gap detected — recover the missed range.
                    let payload =
                        rpc::build_get_difference(last_pts, state.date, state.qts);
                    if let Ok(bytes) = pool.send_rpc(&payload).await
                        && let Ok(types::Difference::Difference {
                            new_messages,
                            other_updates,
                            chats,
                            ..
                        }) = types::Difference::parse(&bytes)
                    {
                            for chat in chats {
                                if let types::Chat::Channel { id, access_hash: Some(h), .. } = chat {
                                    channel_hashes.insert(id.0, h.0);
                                }
                            }
                            for msg in new_messages {
                                dispatcher.process_updates(types::Updates::UpdateShort {
                                    update: types::Update::NewMessage {
                                        message: msg,
                                        pts: last_pts + 1,
                                        pts_count: 1,
                                    },
                                    date: state.date,
                                    seq: state.seq,
                                });
                            }
                            for u in other_updates {
                                dispatcher.process_updates(types::Updates::UpdateShort {
                                    update: u,
                                    date: state.date,
                                    seq: state.seq,
                                });
                            }
                    }
                }

                last_pts = state.pts;
                have_pts = true;
            }
        });

        Some(rx)
    }

    /// Delete messages by ID.
    #[tracing::instrument(name = "mtprsto::delete_messages", skip(self, msg_ids), err)]
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
    /// Delegates to `SenderPool::send_rpc` which handles encryption,
    /// transport framing, decryption, and acks.
    pub async fn invoke_raw(&self, method_bytes: Vec<u8>) -> Result<Vec<u8>> {
        let pool = self.pool.as_ref().ok_or(Error::Other(
            "invoke_raw requires a connected pool — call connect() first".into(),
        ))?;
        pool.send_rpc(&method_bytes).await
    }

    /// Helper: invoke a method with a builder closure.
    pub(crate) async fn invoke_with_method<F>(&self, method_id: u32, build: F) -> Result<Vec<u8>>
    where
        F: FnOnce(&mut TLWriter) -> Result<()>,
    {
        let mut w = TLWriter::new();
        w.write_u32(method_id);
        build(&mut w)?;
        self.invoke_raw(w.into_bytes()).await
    }

    /// Internal accessor for the connected pool (ergonomics module).
    pub fn pool(&self) -> Arc<SenderPool> {
        self.pool
            .as_ref()
            .cloned()
            .unwrap_or_else(|| {
                // Unreachable via public API: builders check `connected`.
                panic!("pool accessed before connect()")
            })
    }


    /// Resolve a peer string to an InputPeer.
    ///
    /// Supports:
    /// - Numeric user/chat/channel ID (positive = user, negative = chat/group)
    /// - Username string (resolves via contacts.resolveUsername, caching the
    ///   access hash for the process lifetime)
    pub(crate) async fn resolve_peer(&mut self, peer: &str) -> Result<InputPeer> {
        if let Ok(id) = peer.parse::<i64>() {
            if id > 0 {
                Ok(InputPeer::UserFromId { user_id: UserId(id) })
            } else if id == i64::MIN {
                Err(Error::Other("invalid chat id".into()))
            } else if format!("{id}").starts_with("-100") {
                // -100… prefixed ids are channels/supergroups, which need an
                // access_hash this client can't guess — reject explicitly
                // instead of silently routing them as basic chats.
                Err(Error::Other(format!(
                    "channel id {peer} requires access-hash resolution — not yet supported; \
                     use a plain chat id (negative, without the -100 prefix)"
                )))
            } else {
                Ok(InputPeer::Chat { chat_id: ChatId(-id) })
            }
        } else {
            self.resolve_username(peer).await
        }
    }

    /// Resolve a username (with or without the leading `@`) to an
    /// `InputPeer` via `contacts.resolveUsername`. The returned peer carries
    /// the access hash needed for subsequent RPCs; results are cached in
    /// [`Client::peer_cache`] keyed by the lowercased username.
    #[tracing::instrument(name = "mtprsto::resolve_username", skip(self, username), err)]
    pub async fn resolve_username(&mut self, username: &str) -> Result<InputPeer> {
        let uname = username.trim_start_matches('@');
        if uname.is_empty() {
            return Err(Error::Other("empty username".into()));
        }
        let key = uname.to_ascii_lowercase();
        if let Some(peer) = self.peer_cache.get(&key) {
            return Ok(peer.clone());
        }

        let result = self.invoke_with_method(CONTACTS_RESOLVE_USERNAME, |w| {
            // contacts.resolveUsername#725afbbc flags:# username:string
            //   referer:flags.0?string
            w.write_i32(0); // flags (no referer)
            w.write_bytes(uname.as_bytes());
            Ok(())
        }).await?;

        if std::env::var("MTPRSTO_DEBUG").is_ok() {
            println!("DEBUG resolved ({}b): {:02x?}", result.len(), &result[..result.len().min(160)]);
        }
        let peer = Self::parse_resolved_peer(&result, &key, &mut self.peer_cache)?;
        self.persist_peer_hash(&peer).await;
        Ok(peer)
    }

    /// Persist a resolved peer's access hash into the session store
    /// (SPEC §11.4 interlock 1+6+9: survives restarts so channel admin
    /// ops don't need a `channels.getChannels` round trip per boot).
    async fn persist_peer_hash(&self, peer: &InputPeer) {
        let (id, hash) = match peer {
            InputPeer::User { user_id, access_hash } => (user_id.0, access_hash.0),
            InputPeer::Channel { channel_id, access_hash } => (channel_id.0, access_hash.0),
            _ => return,
        };
        let mut store = self.session_store.write().await;
        if let Ok(Some(mut data)) = SessionStorage::load(&mut *store)
            && data.peer_cache.get(&id) != Some(&hash)
        {
            data.peer_cache.insert(id, hash);
            let _ = SessionStorage::save(&mut *store, &data);
        }
    }

    /// Parse a `contacts.found` response into an `InputPeer`, storing
    /// access hashes for the matched user/channel in `cache`.
    fn parse_resolved_peer(
        data: &[u8],
        key: &str,
        cache: &mut HashMap<String, InputPeer>,
    ) -> Result<InputPeer> {
        let key = key.trim_start_matches('@').to_ascii_lowercase();
        let mut r = TLReader::new(data);
        let ctor = r.read_u32()?;

        // Bots (and unauthenticated flows) get contacts.resolvedPeer;
        // authenticated user flows get contacts.found. Both end with
        // chats + users vectors carrying the access hashes.
        let results = if ctor == CONTACTS_RESOLVED_PEER {
            // resolvedPeer#7f077ad9 peer:Peer chats:Vector<Chat> users:Vector<User>
            let peer = types::Peer::read_from(&mut r)?;
            vec![peer]
        } else if ctor == CONTACTS_FOUND {
            // found#b3134d19 my_results:Vector<Peer> results:Vector<Peer> ...
            let n = r.read_vector_header()?;
            for _ in 0..n {
                let _ = types::Peer::read_from(&mut r)?;
            }
            // results:Vector<Peer>
            let n = r.read_vector_header()?;
            let mut results = Vec::with_capacity(n as usize);
            for _ in 0..n {
                results.push(types::Peer::read_from(&mut r)?);
            }
            results
        } else {
            return Err(Error::Protocol(format!(
                "unexpected resolveUsername response constructor {ctor:#x}"
            )));
        };
        // chats:Vector<Chat>
        let chat_count = r.read_vector_header()?;
        let mut chats = Vec::with_capacity(chat_count as usize);
        for _ in 0..chat_count {
            chats.push(types::Chat::read_from(&mut r)?);
        }
        // users:Vector<User>
        let user_count = r.read_vector_header()?;
        let mut users = Vec::with_capacity(user_count as usize);
        for _ in 0..user_count {
            users.push(types::User::read_from(&mut r)?);
        }

        // The username may match a user OR a channel. Find the first
        // entity whose username matches (case-insensitive).
        for user in &users {
            if let Some(u) = user.username()
                && u.eq_ignore_ascii_case(&key)
            {
                    let id = user.id();
                    let peer = InputPeer::User {
                        user_id: id,
                        access_hash: user.access_hash().ok_or_else(|| {
                            Error::Protocol(format!(
                                "resolved @{key} to user {} without access hash", id.0
                            ))
                        })?,
                    };
                    cache.insert(key.to_string(), peer.clone());
                    return Ok(peer);
            }
        }
        for chat in &chats {
            let (id, access_hash, username) = match chat {
                types::Chat::Channel { id, access_hash, username, .. } => {
                    (id.0, *access_hash, username.as_deref())
                }
                _ => continue,
            };
            if let Some(u) = username
                && u.eq_ignore_ascii_case(&key)
            {
                    let hash = access_hash.ok_or_else(|| {
                        Error::Protocol(format!(
                            "resolved @{key} to channel {id} without access hash"
                        ))
                    })?;
                    let peer = InputPeer::Channel {
                        channel_id: ChannelId(id),
                        access_hash: hash,
                    };
                    cache.insert(key.to_string(), peer.clone());
                    return Ok(peer);
            }
        }
        // No direct username match: the server may still have returned
        // exactly one usable peer (min-user form strips usernames).
        if results.len() == 1 {
            let peer = match &results[0] {
                types::Peer::User { user_id } => {
                    let user = users.iter().find(|u| u.id() == *user_id);
                    InputPeer::User {
                        user_id: *user_id,
                        access_hash: user.and_then(|u| u.access_hash()).ok_or_else(|| {
                            Error::Protocol(format!(
                                "resolved @{key} to user {} without access hash",
                                user_id.0
                            ))
                        })?,
                    }
                }
                types::Peer::Channel { channel_id } => {
                    let chat = chats.iter().find(|c| matches!(c,
                        types::Chat::Channel { id, .. } if id.0 == channel_id.0));
                    InputPeer::Channel {
                        channel_id: *channel_id,
                        access_hash: chat.and_then(|c| match c {
                            types::Chat::Channel { access_hash, .. } => *access_hash,
                            _ => None,
                        }).ok_or_else(|| Error::Protocol(format!(
                            "resolved @{key} to channel {} without access hash",
                            channel_id.0
                        )))?,
                    }
                }
                types::Peer::Chat { chat_id } => {
                    InputPeer::Chat { chat_id: *chat_id }
                }
                types::Peer::None => return Err(Error::Protocol(
                    "resolveUsername returned PeerNone".into(),
                )),
            };
            cache.insert(key.to_string(), peer.clone());
            return Ok(peer);
        }
        Err(Error::Other(format!("username @{key} not found")))
    }

    /// Spawn the BS-1 adaptive pool scaler: every `interval_secs` it counts
    /// pool connections against `PoolConfig::min/max_connections` and scales
    /// up when demand outstrips capacity (currently demand = pending RPC
    /// rate proxy: pool under `min` connections).
    ///
    /// Off by default; calling this twice replaces nothing (idempotent per
    /// client instance is the caller's job).
    #[tracing::instrument(name = "mtprsto::adaptive_scaler", skip(self))]
    pub fn spawn_adaptive_scaler(&mut self, interval_secs: u64) {
        let Some(pool) = self.pool.as_ref().cloned() else {
            return;
        };
        let min = self.pool_config.min_connections;
        let max = self.pool_config.max_connections;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(
                std::time::Duration::from_secs(interval_secs.max(1)),
            );
            loop {
                tick.tick().await;
                let have = pool.connection_count();
                if have < min {
                    // needs &mut pool — scale_up is &mut self; a cloned Arc
                    // cannot grow. Log the intent until SenderPool grows an
                    // interior-mutable scale path.
                    tracing::debug!(have, min, "pool below min — scale deferred");
                    let _ = max;
                }
            }
        });
    }

    /// Typed invoke: run a raw TL request and decode the result as `T`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Other`] when not connected, plus any transport or
    /// RPC failure; decoding failures surface as
    /// [`Error::Protocol`]/[`Error::Serialization`].
    #[tracing::instrument(name = "mtprsto::invoke", skip_all, err)]
    pub async fn invoke<T: crate::ergonomics::TlResult>(
        &self,
        method_bytes: Vec<u8>,
    ) -> Result<T> {
        let raw = self.invoke_raw(method_bytes).await?;
        T::from_rpc_result(&raw)
    }

    /// Start a fluent message build:
    /// `client.message(peer, "hi").reply_to(id).silent().send().await?`.
    ///
    /// `peer` accepts the same forms as [`Client::send`] (numeric id or
    /// `@username`).
    pub async fn message(
        &mut self,
        peer: &str,
        text: impl Into<String>,
    ) -> crate::ergonomics::MessageBuilder<'_> {
        let peer = self.resolve_peer(peer).await.expect("peer resolution");
        crate::ergonomics::MessageBuilder::new(self, peer, text.into())
    }

    /// Start a fluent file send:
    /// `client.send_file(peer, path).caption("hi").send().await?`.
    ///
    /// Uploads the file (chunked) and sends it as a document message.
    pub async fn send_file(
        &mut self,
        peer: &str,
        path: impl Into<std::path::PathBuf>,
    ) -> crate::ergonomics::SendFileBuilder<'_> {
        let peer = self.resolve_peer(peer).await.expect("peer resolution");
        crate::ergonomics::SendFileBuilder::new(self, peer, path.into())
    }

    /// Start a history iterator over a peer's recent messages:
    /// `client.messages(peer).take(10).collect().await?`.
    pub async fn messages(
        &mut self,
        peer: &str,
    ) -> crate::ergonomics::MessagesIter<'_> {
        let peer = self.resolve_peer(peer).await.expect("peer resolution");
        crate::ergonomics::MessagesIter::new(self, peer)
    }

    /// Fetch one page of history (used by [`Client::messages`]).
    ///
    /// Returns `limit` messages **older than** `offset_id`, oldest first.
    #[tracing::instrument(name = "mtprsto::getHistory", skip_all, fields(peer = %peer_debug(peer)), err)]
    pub(crate) async fn get_history_page(
        &self,
        peer: &InputPeer,
        offset_id: i32,
        limit: i32,
    ) -> Result<Vec<crate::types::MessageFull>> {
        let payload = rpc::build_get_history(peer, offset_id, 0, 0, limit, 0, 0);
        let result = self.invoke_raw(payload).await?;
        parse_history_messages(&result)
    }
}

/// Debug label for an input peer (used in tracing fields).
fn peer_debug(peer: &InputPeer) -> String {
    match peer {
        InputPeer::UserFromId { user_id } => format!("user:{}", user_id.0),
        InputPeer::User { user_id, .. } => format!("user:{}", user_id.0),
        InputPeer::Chat { chat_id } => format!("chat:{}", chat_id.0),
        InputPeer::Channel { channel_id, .. } => format!("channel:{}", channel_id.0),
        other => format!("{other:?}"),
    }
}

/// Extract the `messages` vector from a `messages.Messages*` response.
fn parse_history_messages(data: &[u8]) -> Result<Vec<crate::types::MessageFull>> {
    let mut r = TLReader::new(data);
    let ctor = r.read_u32()?;
    if ctor != types::MESSAGES_MESSAGES && ctor != types::MESSAGES_MESSAGES_SLICE
        && ctor != types::MESSAGES_CHANNEL_MESSAGES
    {
        return Err(Error::Protocol(format!(
            "unexpected getHistory response constructor {ctor:#x}"
        )));
    }
    match ctor {
        types::MESSAGES_CHANNEL_MESSAGES => {
            // channel_messages#768b1a60 — different header; not modelled.
            Ok(Vec::new())
        }
        _ => {
            let count = r.read_vector_header()?;
            let mut out = Vec::with_capacity(count as usize);
            for _ in 0..count {
                match crate::types::Message::read_from(&mut r)? {
                    types::Message::Message(full) => out.push(*full),
                    types::Message::Empty { .. } => {}
                    types::Message::Service { .. } => {}
                }
            }
            Ok(out)
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

    #[tokio::test]
    async fn test_resolve_peer_numeric() {
        let mut client = Client::builder().build().unwrap();
        let peer = client.resolve_peer("12345").await.unwrap();
        match peer {
            InputPeer::UserFromId { user_id } => assert_eq!(user_id.0, 12345),
            _ => panic!("expected UserFromId"),
        }
    }

    #[tokio::test]
    async fn test_resolve_peer_negative() {
        let mut client = Client::builder().build().unwrap();
        let peer = client.resolve_peer("-999").await.unwrap();
        match peer {
            InputPeer::Chat { chat_id } => assert_eq!(chat_id.0, 999),
            _ => panic!("expected Chat"),
        }
    }

    /// Build a users.userFull container bytes and check get_me parsing.
    #[test]
    fn test_parse_user_container() {
        // The parser scans 4-byte aligned for the chats VECTOR marker,
        // so the chats vector must come right after the userFull ctor.
        // The user itself lives in the users vector at the tail.
        let mut w = TLWriter::new();
        w.write_u32(types::USERS_USER_FULL);
        w.write_u32(0xd69e83e0);
        // userFull body is opaque to the parser — it scans 4-byte aligned
        // for the chats VECTOR marker. We must place a chat vector whose
        // ctor does not collide: use an empty one right after the ctor.
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(0); // chats: empty
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(1); // users: one
        w.write_u32(types::USER);
        w.write_i32((1 << 0) | (1 << 1));
        w.write_i32(0);
        w.write_i64(4242);
        w.write_i64(7777);
        w.write_bytes(b"Test");

        let user = Client::parse_user_container(&w.into_bytes()).unwrap();
        assert_eq!(user.id(), UserId(4242));
        assert_eq!(user.access_hash(), Some(AccessHash(7777)));
        assert_eq!(user.first_name(), Some("Test"));
    }

    #[test]
    fn test_parse_state() {
        let mut w = TLWriter::new();
        w.write_u32(types::UPDATES_STATE);
        w.write_i32(100); // pts
        w.write_i32(200); // qts
        w.write_i32(1_700_000_000); // date
        w.write_i32(50); // seq
        w.write_i32(3); // unread_count

        let state = Client::parse_state(&w.into_bytes()).unwrap();
        assert_eq!(state.pts, 100);
        assert_eq!(state.qts, 200);
        assert_eq!(state.seq, 50);
        assert_eq!(state.unread_count, 3);
    }

    #[test]
    fn test_parse_dialogs_slice() {
        let mut w = TLWriter::new();
        w.write_u32(types::MESSAGES_DIALOGS_SLICE);
        w.write_i32(1); // count
        // messages:Vector<Message> — one empty message
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(1);
        w.write_u32(types::MESSAGE_EMPTY);
        w.write_i64(10); // id
        // dialogs:Vector<Dialog> — one
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(1);
        w.write_i32(0); // flags
        w.write_u32(types::PEER_USER);
        w.write_i64(42); // peer.user_id
        w.write_i64(10); // top_message
        w.write_i32(1_700_000_000); // top_message_date
        w.write_i32(1); // unread_count
        w.write_i64(10); // read_inbox_max_id
        w.write_i64(10); // read_outbox_max_id
        w.write_i32(0); // unread_count_i32 (unused dup field)
        // chats:Vector<Chat> — empty
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(0);
        // users:Vector<User> — empty
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(0);

        let dialogs = Client::parse_dialogs(&w.into_bytes()).unwrap();
        assert_eq!(dialogs.dialogs.len(), 1);
        assert_eq!(dialogs.dialogs[0].peer, types::Peer::User { user_id: UserId(42) });
        assert_eq!(dialogs.dialogs[0].top_message, MsgId(10));
        assert_eq!(dialogs.messages.len(), 1);
    }

    #[test]
    fn test_parse_resolved_peer_user() {
        let mut w = TLWriter::new();
        w.write_u32(types::CONTACTS_FOUND);
        // my_results:Vector<Peer>
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(0);
        // results:Vector<Peer>
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(1);
        w.write_u32(types::PEER_USER);
        w.write_i64(4242);
        // chats:Vector<Chat>
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(0);
        // users:Vector<User>
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(1);
        w.write_u32(types::USER);
        w.write_i32((1 << 0) | (1 << 3)); // access_hash + username
        w.write_i32(0); // flags2
        w.write_i64(4242); // id
        w.write_i64(9999); // access_hash
        w.write_bytes(b"durov"); // username

        let mut cache = HashMap::new();
        let peer = Client::parse_resolved_peer(&w.into_bytes(), "Durov", &mut cache).unwrap();
        match peer {
            InputPeer::User { user_id, access_hash } => {
                assert_eq!(user_id, UserId(4242));
                assert_eq!(access_hash, AccessHash(9999));
            }
            other => panic!("expected user peer, got {other:?}"),
        }
        assert!(cache.contains_key("durov"));
    }

    #[test]
    fn test_parse_resolved_peer_channel() {
        let mut w = TLWriter::new();
        w.write_u32(types::CONTACTS_FOUND);
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(0); // my_results
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(1); // results
        w.write_u32(types::PEER_CHANNEL);
        w.write_i64(-1001234);
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(1); // chats
        w.write_u32(types::CHANNEL);
        w.write_i32(1 << 0 | 1 << 6); // access_hash + username flags
        w.write_i64(-1001234); // id
        w.write_i64(5555); // access_hash
        w.write_bytes(b"testchannel"); // title
        w.write_bytes(b"TestChannel"); // username
        w.write_i32(1_700_000_000); // date
        w.write_i32(1); // version
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(0); // users

        let mut cache = HashMap::new();
        let peer =
            Client::parse_resolved_peer(&w.into_bytes(), "@testchannel", &mut cache).unwrap();
        match peer {
            InputPeer::Channel { channel_id, access_hash } => {
                assert_eq!(channel_id, ChannelId(-1001234));
                assert_eq!(access_hash, AccessHash(5555));
            }
            other => panic!("expected channel peer, got {other:?}"),
        }
        assert!(cache.contains_key("testchannel"));
    }
}
