//! Typed Client wrappers over the `rpc::build_*` payload builders
//! (SPEC §7 surface, P0 wrapper gap). Split from `client.rs` so the
//! connection/session logic stays in one file and the RPC surface can
//! grow without churn.
//!
//! Every wrapper follows the same shape:
//! 1. build payload via `crate::rpc`,
//! 2. `invoke_raw` through the pool,
//! 3. decode the typed response.

use crate::types;

use crate::client::Client;
use crate::error::{Error, Result};
use crate::rpc;
use crate::serialize::TLReader;
use crate::ergonomics::TlResult;
use crate::types::*;

// ===========================================================================
// Response types
// ===========================================================================

/// Parsed `messages.botCallbackAnswer#36585ea4`.
#[derive(Debug, Clone, Default)]
pub struct BotCallbackAnswer {
    /// Show the message as an alert popup instead of a toast.
    pub alert: bool,
    /// The message contains a URL.
    pub has_url: bool,
    /// Render with the native OS UI.
    pub native_ui: bool,
    /// Bot-supplied message text.
    pub message: Option<String>,
    /// URL to open (when `has_url`).
    pub url: Option<String>,
    /// Seconds the answer may be cached client-side.
    pub cache_time: i32,
}

impl BotCallbackAnswer {
    /// Decode from a `messages.botCallbackAnswer` payload (ctor included).
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = TLReader::new(data);
        let ctor = r.read_u32()?;
        if ctor != MESSAGES_BOT_CALLBACK_ANSWER {
            return Err(Error::Protocol(format!(
                "expected messages.botCallbackAnswer, got {ctor:#x}"
            )));
        }
        let flags = r.read_i32()?;
        let message = if flags & (1 << 0) != 0 {
            Some(String::from_utf8(r.read_bytes()?)?)
        } else {
            None
        };
        let url = if flags & (1 << 2) != 0 {
            Some(String::from_utf8(r.read_bytes()?)?)
        } else {
            None
        };
        let cache_time = r.read_i32()?;
        Ok(Self {
            alert: flags & (1 << 1) != 0,
            has_url: flags & (1 << 3) != 0,
            native_ui: flags & (1 << 4) != 0,
            message,
            url,
            cache_time,
        })
    }
}

/// Parsed `messages.affectedMessages#84d19185`.
#[derive(Debug, Clone, Copy, Default)]
pub struct AffectedMessages {
    pub pts: i32,
    pub pts_count: i32,
}

impl AffectedMessages {
    /// Decode from an `messages.affectedMessages` payload (ctor included).
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = TLReader::new(data);
        let ctor = r.read_u32()?;
        if ctor != MESSAGES_AFFECTED_MESSAGES {
            return Err(Error::Protocol(format!(
                "expected messages.affectedMessages, got {ctor:#x}"
            )));
        }
        Ok(Self {
            pts: r.read_i32()?,
            pts_count: r.read_i32()?,
        })
    }
}

// ===========================================================================
// helpers
// ===========================================================================

/// Pull the `messages` vector out of a `messages.Messages*` response.
///
/// Full shapes (layer 225): `messages.messages#1d73e7ea messages topics
/// chats users`; `messagesSlice#5f206716 flags count next_rate?
/// offset_id_offset? search_flood? then the same tail`;
/// `channelMessages#c776ba4e flags pts count offset_id_offset? tail`.
fn messages_from_container(data: &[u8]) -> Result<Vec<Message>> {
    let mut r = TLReader::new(data);
    let ctor = r.read_u32()?;
    match ctor {
        MESSAGES_MESSAGES => read_messages_body(&mut r),
        MESSAGES_MESSAGES_SLICE => {
            let flags = r.read_i32()?;
            let _slice_count = r.read_i32()?;
            if flags & (1 << 0) != 0 {
                let _next_rate = r.read_i32()?;
            }
            if flags & (1 << 2) != 0 {
                let _offset_id_offset = r.read_i32()?;
            }
            if flags & (1 << 3) != 0 {
                return Err(Error::Protocol(
                    "messagesSlice carries search_flood (SearchPostsFlood) — not supported"
                        .into(),
                ));
            }
            read_messages_body(&mut r)
        }
        _ => Err(Error::Protocol(format!(
            "expected messages.Messages*, got {ctor:#x}"
        ))),
    }
}

/// messages/topics vectors plus the chats/users tail of messages.Messages*.
fn read_messages_body(r: &mut TLReader) -> Result<Vec<Message>> {
    let count = r.read_vector_header()?;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        out.push(Message::read_from(r)?);
    }
    // topics:Vector<ForumTopic> — must be consumed before chats/users.
    let topic_count = r.read_vector_header()?;
    for _ in 0..topic_count {
        let tctor = r.read_u32()?;
        if tctor == types::FORUM_TOPIC_DELETED {
            let _id = r.read_i32()?;
        } else {
            return Err(Error::Protocol(format!(
                "unsupported ForumTopic constructor {tctor:#x} in messages container"
            )));
        }
    }
    let _chats = crate::types::read_chat_vector_public(r)?;
    let _users = crate::types::read_user_vector_public(r)?;
    Ok(out)
}

impl Client {
    // =======================================================================
    // Messages
    // =======================================================================

    /// Send an album (2–10 media items) to a peer.
    ///
    /// # Errors
    ///
    /// Transport or RPC failure; peer must already be resolvable.
    #[tracing::instrument(name = "mtprsto::send_multi_media", skip(self, items), err)]
    pub async fn send_multi_media(
        &mut self,
        peer: &str,
        items: Vec<(rpc::InputMedia, String)>,
    ) -> Result<MsgId> {
        let input_peer = self.resolve_peer(peer).await?;
        let single: Vec<rpc::InputSingleMedia> = items
            .into_iter()
            .map(|(media, message)| rpc::InputSingleMedia { media, message })
            .collect();
        let payload = rpc::build_send_multi_media(&input_peer, &single, None, false, false, None);
        let result = self.invoke_raw(payload).await?;
        MsgId::from_rpc_result(&result)
    }

    /// Edit a sent message's text. Returns `()`-style success.
    ///
    /// # Errors
    ///
    /// Transport or RPC failure (e.g. `MESSAGE_ID_INVALID`).
    #[tracing::instrument(name = "mtprsto::edit_message", skip(self), err)]
    pub async fn edit_message(
        &mut self,
        peer: &str,
        msg_id: i32,
        text: &str,
    ) -> Result<()> {
        let input_peer = self.resolve_peer(peer).await?;
        let payload = rpc::build_edit_message(&input_peer, msg_id, Some(text));
        self.invoke_raw(payload).await?;
        Ok(())
    }

    /// Delete the entire chat history with `peer`.
    ///
    /// Returns pts/pts_count so callers can advance update state.
    ///
    /// # Errors
    ///
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::delete_history", skip(self), err)]
    pub async fn delete_history(
        &mut self,
        peer: &str,
        max_id: i32,
        just_clear: bool,
        revoke: bool,
    ) -> Result<AffectedMessages> {
        let input_peer = self.resolve_peer(peer).await?;
        let payload = rpc::build_delete_history(&input_peer, max_id, just_clear, revoke);
        let result = self.invoke_raw(payload).await?;
        // messages.deleteHistory answers messages.affectedHistory (pts,
        // pts_count, offset) — map it onto the shared shape.
        let mut r = TLReader::new(&result);
        let ctor = r.read_u32()?;
        match ctor {
            MESSAGES_AFFECTED_HISTORY => Ok(AffectedMessages {
                pts: r.read_i32()?,
                pts_count: r.read_i32()?,
            }),
            MESSAGES_AFFECTED_MESSAGES => {
                Ok(AffectedMessages { pts: r.read_i32()?, pts_count: r.read_i32()? })
            }
            other => Err(Error::Protocol(format!(
                "expected messages.affectedHistory, got {other:#x}"
            ))),
        }
    }

    /// Mark history with `peer` as read up to `max_id`.
    ///
    /// # Errors
    ///
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::read_history", skip(self), err)]
    pub async fn read_history(&mut self, peer: &str, max_id: i32) -> Result<AffectedMessages> {
        let input_peer = self.resolve_peer(peer).await?;
        let payload = rpc::build_read_history(&input_peer, max_id);
        let result = self.invoke_raw(payload).await?;
        AffectedMessages::parse(&result)
    }

    /// Search messages in `peer` matching `query`.
    ///
    /// # Errors
    ///
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::search", skip(self), err)]
    pub async fn search(&mut self, peer: &str, query: &str, limit: i32) -> Result<Vec<Message>> {
        let input_peer = self.resolve_peer(peer).await?;
        let payload = rpc::build_search(&input_peer, query, limit);
        let result = self.invoke_raw(payload).await?;
        messages_from_container(&result)
    }

    /// Press an inline button and fetch the bot's answer.
    ///
    /// `data` is the `callback_data` carried by the pressed button.
    ///
    /// # Errors
    ///
    /// Transport or RPC failure (`BOT_RESPONSE_TIMEOUT` etc.).
    #[tracing::instrument(name = "mtprsto::get_bot_callback_answer", skip(self, data), err)]
    pub async fn get_bot_callback_answer(
        &mut self,
        peer: &str,
        msg_id: MsgId,
        data: &[u8],
    ) -> Result<BotCallbackAnswer> {
        let input_peer = self.resolve_peer(peer).await?;
        let payload = rpc::build_get_bot_callback_answer(&input_peer, msg_id.0 as i32, data);
        let result = self.invoke_raw(payload).await?;
        BotCallbackAnswer::parse(&result)
    }

    // =======================================================================
    // Contacts / users
    // =======================================================================

    /// Resolve a phone number to a peer.
    ///
    /// # Errors
    /// Transport or RPC failure (`PHONE_NUMBER_INVALID` etc.).
    #[tracing::instrument(name = "mtprsto::resolve_phone", skip(self), err)]
    pub async fn resolve_phone(&mut self, phone: &str) -> Result<InputPeer> {
        let payload = rpc::build_resolve_phone(phone);
        let result = self.invoke_raw(payload).await?;
        // contacts.resolvedPeer#7f077ad9 peer:Peer chats:... users:...
        let mut r = TLReader::new(&result);
        let ctor = r.read_u32()?;
        if ctor != CONTACTS_RESOLVED_PEER {
            return Err(Error::Protocol(format!(
                "expected contacts.resolvedPeer, got {ctor:#x}"
            )));
        }
        let peer = Peer::read_from(&mut r)?;
        Self::peer_to_input_peer(&peer)
            .ok_or_else(|| Error::Protocol("resolvedPeer carries no usable peer".into()))
    }

    /// Search contacts and global public peers by name/username.
    ///
    /// # Errors
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::search_contacts", skip(self), err)]
    pub async fn search_contacts(&mut self, q: &str, limit: i32) -> Result<Vec<User>> {
        let payload = rpc::build_contacts_search(q, limit);
        let result = self.invoke_raw(payload).await?;
        // contacts.found#b3134d19 my_results:Vector<Peer> results:Vector<Peer>
        // chats:Vector<Chat> users:Vector<User>
        let mut r = TLReader::new(&result);
        let ctor = r.read_u32()?;
        if ctor != CONTACTS_FOUND {
            return Err(Error::Protocol(format!(
                "expected contacts.found, got {ctor:#x}"
            )));
        }
        let my_n = r.read_vector_header()?;
        for _ in 0..my_n {
            let _ = Peer::read_from(&mut r)?;
        }
        let res_n = r.read_vector_header()?;
        for _ in 0..res_n {
            let _ = Peer::read_from(&mut r)?;
        }
        let chat_n = r.read_vector_header()?;
        for _ in 0..chat_n {
            let _ = Chat::read_from(&mut r)?;
        }
        let user_n = r.read_vector_header()?;
        let mut users = Vec::with_capacity(user_n as usize);
        for _ in 0..user_n {
            users.push(User::read_from(&mut r)?);
        }
        Ok(users)
    }

    /// Fetch full user objects for the given input users.
    ///
    /// # Errors
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::get_users", skip(self, users), err)]
    pub async fn get_users(&self, users: &[InputUser]) -> Result<Vec<User>> {
        let payload = rpc::build_get_users(users);
        let result = self.invoke_raw(payload).await?;
        // Vector<User>
        let mut r = TLReader::new(&result);
        let n = r.read_vector_header()?;
        let mut out = Vec::with_capacity(n as usize);
        for _ in 0..n {
            out.push(User::read_from(&mut r)?);
        }
        Ok(out)
    }

    // =======================================================================
    // Channels
    // =======================================================================

    /// Create a channel (broadcast) or supergroup.
    ///
    /// # Errors
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::create_channel", skip(self), err)]
    pub async fn create_channel(
        &mut self,
        title: &str,
        about: &str,
        broadcast: bool,
        megagroup: bool,
    ) -> Result<Vec<Chat>> {
        let payload = rpc::build_create_channel(title, about, broadcast, megagroup);
        let result = self.invoke_raw(payload).await?;
        Self::chats_from_updates(&result)
    }

    /// Invite users to a channel/supergroup.
    ///
    /// # Errors
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::invite_to_channel", skip(self, users), err)]
    pub async fn invite_to_channel(
        &self,
        channel: &InputChannel,
        users: &[InputUser],
    ) -> Result<Vec<Chat>> {
        let payload = rpc::build_invite_to_channel(channel, users);
        let result = self.invoke_raw(payload).await?;
        Self::chats_from_updates(&result)
    }

    /// Edit admin rights for a user in a channel.
    ///
    /// # Errors
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::edit_admin", skip(self), err)]
    pub async fn edit_admin(
        &self,
        channel: &InputChannel,
        user: &InputUser,
        admin_rights: i32,
        rank: &str,
    ) -> Result<()> {
        let payload = rpc::build_edit_admin(channel, user, admin_rights, rank);
        self.invoke_raw(payload).await?;
        Ok(())
    }

    /// Fetch basic info for the given channels.
    ///
    /// # Errors
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::get_channels", skip(self, channels), err)]
    pub async fn get_channels(&self, channels: &[InputChannel]) -> Result<Vec<Chat>> {
        let payload = rpc::build_get_channels(channels);
        let result = self.invoke_raw(payload).await?;
        Self::chats_from_updates(&result)
    }

    /// List channel participants (recent or search filter).
    ///
    /// Returns raw `Vector<ChannelParticipant>` bytes — participant
    /// subtypes are not yet modelled.
    ///
    /// # Errors
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::get_participants", skip(self, filter), err)]
    pub async fn get_participants(
        &self,
        channel: &InputChannel,
        filter: &rpc::ChannelParticipantsFilter,
        offset: i32,
        limit: i32,
        hash: i64,
    ) -> Result<(i32, Vec<u8>)> {
        let payload = rpc::build_get_participants(channel, filter, offset, limit, hash);
        let result = self.invoke_raw(payload).await?;
        // channels.channelParticipants#9ab0feaf count:int participants:...
        let mut r = TLReader::new(&result);
        let ctor = r.read_u32()?;
        if ctor != CHANNELS_CHANNEL_PARTICIPANTS {
            return Err(Error::Protocol(format!(
                "expected channels.channelParticipants, got {ctor:#x}"
            )));
        }
        let count = r.read_i32()?;
        // Copy the raw vector bytes (ctor..end) for later typed parsing.
        let start = r.position();
        Ok((count, result[start..].to_vec()))
    }

    /// Leave a channel/supergroup.
    ///
    /// # Errors
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::leave_channel", skip(self, channel), err)]
    pub async fn leave_channel(&self, channel: &InputChannel) -> Result<()> {
        let payload = rpc::build_leave_channel(channel);
        self.invoke_raw(payload).await?;
        Ok(())
    }

    // =======================================================================
    // Photos
    // =======================================================================

    /// Replace the current user's profile photo with an already-uploaded
    /// photo (`InputPhoto` reference).
    ///
    /// # Errors
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::update_profile_photo", skip(self, id), err)]
    pub async fn update_profile_photo(
        &mut self,
        fallback: bool,
        id: &rpc::InputPhoto,
    ) -> Result<Vec<u8>> {
        let payload = rpc::build_update_profile_photo(fallback, id);
        self.invoke_raw(payload).await
    }

    /// Upload a local file and set it as the profile photo.
    ///
    /// # Errors
    /// Filesystem, transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::upload_profile_photo", skip(self, path), err)]
    pub async fn upload_profile_photo(
        &mut self,
        path: impl Into<std::path::PathBuf>,
    ) -> Result<Vec<u8>> {
        let path = path.into();
        let pool = self.pool().clone();
        let input_file = crate::file::upload_file(pool, &path, 4).await?;

        // photos.uploadProfilePhoto#388a3b5 flags:# fallback:flags.3?true
        // bot:flags.5?InputUser file:flags.0?InputFile video:flags.1?InputFile
        // video_start_ts:flags.2?double video_emoji_markup:flags.4?VideoSize
        let mut w = crate::serialize::TLWriter::new();
        w.write_u32(PHOTOS_UPLOAD_PROFILE_PHOTO);
        w.write_i32(1 << 0); // file flag
        input_file.write_to(&mut w);
        self.invoke_raw(w.into_bytes()).await
    }

    /// Delete profile photos by reference.
    ///
    /// Returns the deleted photo ids (`Vector<long>`).
    ///
    /// # Errors
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::delete_photos", skip(self, photos), err)]
    pub async fn delete_photos(&self, photos: &[rpc::InputPhoto]) -> Result<Vec<i64>> {
        let payload = rpc::build_delete_photos(photos);
        let result = self.invoke_raw(payload).await?;
        let mut r = TLReader::new(&result);
        let n = r.read_vector_header()?;
        let mut out = Vec::with_capacity(n as usize);
        for _ in 0..n {
            out.push(r.read_i64()?);
        }
        Ok(out)
    }

    /// Fetch a user's profile photos.
    ///
    /// Returns raw `photos.Photos` bytes (slice/full variants not modelled).
    ///
    /// # Errors
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::get_user_photos", skip(self, user), err)]
    pub async fn get_user_photos(
        &self,
        user: &InputUser,
        offset: i32,
        max_id: i64,
        limit: i32,
    ) -> Result<Vec<u8>> {
        let payload = rpc::build_get_user_photos(user, offset, max_id, limit);
        self.invoke_raw(payload).await
    }
}

// ===========================================================================
// Shared decode helpers
// ===========================================================================

impl Client {
    /// Convert a bare `Peer` to an `InputPeer` where the server allowed it.
    ///
    /// Channels/users need an access hash which a bare `Peer` does not
    /// carry, so only basic-group chats map cleanly; the rest fall back to
    /// the hash-less legacy forms the server accepts in some flows.
    pub(crate) fn peer_to_input_peer(peer: &Peer) -> Option<InputPeer> {
        match peer {
            Peer::User { user_id } => Some(InputPeer::UserFromId { user_id: *user_id }),
            Peer::Chat { chat_id } => Some(InputPeer::Chat { chat_id: *chat_id }),
            Peer::Channel { channel_id } => {
                Some(InputPeer::Channel { channel_id: *channel_id, access_hash: AccessHash(0) })
            }
            Peer::None => None,
        }
    }

    /// Extract the `chats` vector from an `Updates*` response.
    pub(crate) fn chats_from_updates(data: &[u8]) -> Result<Vec<Chat>> {
        if std::env::var("MTPRSTO_DEBUG").is_ok() {
            println!(
                "DEBUG chats_from_updates body ({}b): {:02x?}",
                data.len(),
                &data[..data.len().min(160)]
            );
        }
        let updates = crate::types::Updates::parse(data)?;
        Ok(updates.chats())
    }
}

// ===========================================================================
// Login tokens (SPEC §3, P0 #3) — QR-code login flows
// ===========================================================================

impl Client {
    /// QR-code login, step 1: request a login token to render as
    /// `tg://login?token=...`.
    ///
    /// # Errors
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::export_login_token", skip(self, except_ids), err)]
    pub async fn export_login_token(
        &mut self,
        except_ids: &[i64],
    ) -> Result<crate::api::AuthLoginToken> {
        let mut w = crate::serialize::TLWriter::new();
        w.write_u32(types::AUTH_EXPORT_LOGIN_TOKEN);
        w.write_i32(self.api_id().unwrap_or(0));
        w.write_bytes(self.api_hash().unwrap_or("").as_bytes());
        w.write_i32(except_ids.len() as i32);
        for id in except_ids {
            w.write_i64(*id);
        }
        let result = self.invoke_raw(w.into_bytes()).await?;
        crate::api::parse_login_token_response(&result)
    }

    /// QR-code login, caller side: import a token scanned from another
    /// device and poll until approved.
    ///
    /// # Errors
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::import_login_token", skip(self, token), err)]
    pub async fn import_login_token(
        &mut self,
        token: &[u8],
    ) -> Result<crate::api::AuthLoginToken> {
        let mut w = crate::serialize::TLWriter::new();
        w.write_u32(types::AUTH_IMPORT_LOGIN_TOKEN);
        w.write_bytes(token);
        let result = self.invoke_raw(w.into_bytes()).await?;
        crate::api::parse_login_token_response(&result)
    }

    /// QR-code login, other side: accept a token scanned from a QR code.
    /// Returns the authorized user id.
    ///
    /// # Errors
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::accept_login_token", skip(self, token), err)]
    pub async fn accept_login_token(&mut self, token: &[u8]) -> Result<UserId> {
        let mut w = crate::serialize::TLWriter::new();
        w.write_u32(types::AUTH_ACCEPT_LOGIN_TOKEN);
        w.write_bytes(token);
        let result = self.invoke_raw(w.into_bytes()).await?;
        let mut r = TLReader::new(&result);
        let ctor = r.read_u32()?;
        if ctor != AUTH_AUTHORIZATION {
            return Err(Error::Protocol(format!(
                "expected auth.Authorization, got {ctor:#x}"
            )));
        }
        // Skip flags + optionals, land on user.
        let flags = r.read_i32()?;
        if flags & (1 << 0) != 0 {
            let _ = r.read_i32()?; // tmp_sessions
        }
        if flags & (1 << 1) != 0 {
            let _ = r.read_i32()?; // otherwise_relogin_days
        }
        if flags & (1 << 2) != 0 {
            let _ = r.read_bytes()?; // future_auth_token
        }
        let user = User::read_from(&mut r)?;
        Ok(user.id())
    }

    /// `help.getConfig` — the canonical DC option list and config expiry
    /// (SPEC §1: refresh the hard-coded DC table from this).
    pub async fn get_config(&self) -> Result<ServerConfig> {
        let result = self.invoke_raw(rpc::build_get_config()).await?;
        let mut r = TLReader::new(&result);
        let ctor = r.read_u32()?;
        if ctor != types::CONFIG {
            return Err(Error::Protocol(format!(
                "expected config#cc1a241e, got {ctor:#x}"
            )));
        }
        // flags, date, expires, test_mode(Bool ctor), this_dc
        let _flags = r.read_i32()?;
        let _date = r.read_i32()?;
        let expires = r.read_i32()?;
        let _test_mode = r.read_u32()?;
        let this_dc = r.read_i32()?;
        // dc_options:Vector<DcOption>
        let _vec_ctor = r.read_u32()?;
        let count = r.read_i32()?;
        let mut dc_options = Vec::with_capacity(count.max(0) as usize);
        for _ in 0..count {
            let opt_ctor = r.read_u32()?;
            if opt_ctor != types::DC_OPTION {
                return Err(Error::Protocol(format!(
                    "expected dcOption#18b7a10d, got {opt_ctor:#x}"
                )));
            }
            let opt_flags = r.read_i32()?;
            let dc_id = r.read_i32()?;
            let ip = String::from_utf8(r.read_bytes()?)
                .map_err(|_| Error::Protocol("invalid UTF-8 in dcOption ip".into()))?;
            let port = r.read_i32()?;
            if opt_flags & (1 << 10) != 0 {
                let _secret = r.read_bytes()?;
            }
            dc_options.push(DcOption {
                dc_id,
                ip_address: ip,
                port,
                ipv6: opt_flags & (1 << 0) != 0,
                cdn: opt_flags & (1 << 3) != 0,
                static_: opt_flags & (1 << 4) != 0,
            });
        }
        Ok(ServerConfig {
            expires,
            this_dc,
            dc_options,
        })
    }
}

/// One `dcOption#18b7a10d` entry from `help.getConfig`.
#[derive(Debug, Clone)]
pub struct DcOption {
    pub dc_id: i32,
    pub ip_address: String,
    pub port: i32,
    pub ipv6: bool,
    pub cdn: bool,
    pub static_: bool,
}

/// Parsed `help.getConfig` response (subset — DC options and expiry).
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Seconds this config stays valid (refresh after it lapses, BS-6).
    pub expires: i32,
    /// The DC this client is currently talking to.
    pub this_dc: i32,
    /// Canonical DC option list.
    pub dc_options: Vec<DcOption>,
}
