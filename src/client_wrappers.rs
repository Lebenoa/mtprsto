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
/// `channelMessages#c776ba4e flags pts count offset_id_offset? tail`
/// (the `channels.getMessages` answer); `messagesNotModified#74535f21`
/// carries nothing (empty result).
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
        MESSAGES_CHANNEL_MESSAGES => {
            // channelMessages#c776ba4e flags:# inexact:flags.1?true pts:int
            //   count:int offset_id_offset:flags.2?int messages topics chats users
            let flags = r.read_i32()?;
            let _pts = r.read_i32()?;
            let _count = r.read_i32()?;
            if flags & (1 << 2) != 0 {
                let _offset_id_offset = r.read_i32()?;
            }
            read_messages_body(&mut r)
        }
        MESSAGES_MESSAGES_NOT_MODIFIED => Ok(Vec::new()),
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

    /// Fetch messages by id from `peer`.
    ///
    /// Channel peers go through `channels.getMessages` (the plain
    /// `messages.getMessages` answers `CHANNEL_INVALID` there); every
    /// other peer uses the plain method. Deleted/unknown ids come back
    /// as `Message::Empty` entries rather than errors.
    ///
    /// The object form of the `&str` wrappers: for callers that already
    /// hold an [`InputPeer`] (e.g. from their own peer cache).
    ///
    /// # Errors
    ///
    /// Transport or RPC failure; an unresolvable channel access hash
    /// surfaces as `CHANNEL_INVALID`/`CHANNEL_PRIVATE`.
    #[tracing::instrument(name = "mtprsto::get_messages", skip(self, msg_ids), err)]
    pub async fn get_messages(
        &self,
        peer: &InputPeer,
        msg_ids: &[MsgId],
    ) -> Result<Vec<Message>> {
        let payload = match peer {
            InputPeer::Channel { channel_id, access_hash } => {
                rpc::build_channels_get_messages(
                    &InputChannel::Channel { channel_id: *channel_id, access_hash: *access_hash },
                    msg_ids,
                )
            }
            _ => rpc::build_get_messages(msg_ids),
        };
        let result = self.invoke_raw(payload).await?;
        messages_from_container(&result)
    }

    /// The newest `limit` messages of `peer`, newest first.
    ///
    /// `messages.getHistory` accepts user chats and channels alike;
    /// this is a plain one-shot page, unlike the [`crate::client::Client::messages`]
    /// iterator (which pages oldest-first).
    ///
    /// The object form of the `&str` wrappers: for callers that already
    /// hold an [`InputPeer`] (e.g. from their own peer cache).
    ///
    /// # Errors
    ///
    /// Transport or RPC failure.
    #[tracing::instrument(name = "mtprsto::get_recent_messages", skip(self), err)]
    pub async fn get_recent_messages(
        &self,
        peer: &InputPeer,
        limit: i32,
    ) -> Result<Vec<Message>> {
        let payload = rpc::build_get_history(peer, 0, 0, 0, limit, 0, 0);
        let result = self.invoke_raw(payload).await?;
        messages_from_container(&result)
    }

    /// Send an album (2–10 media items) to a peer.
    ///
    /// # Errors
    ///
    /// Transport or RPC failure; peer must already be resolvable.
    #[tracing::instrument(name = "mtprsto::send_multi_media", skip(self, items), err)]
    pub async fn send_multi_media(
        &self,
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
        &self,
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
        &self,
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
    pub async fn read_history(&self, peer: &str, max_id: i32) -> Result<AffectedMessages> {
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
    pub async fn search(&self, peer: &str, query: &str, limit: i32) -> Result<Vec<Message>> {
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
        &self,
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
    pub async fn resolve_phone(&self, phone: &str) -> Result<InputPeer> {
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
    pub async fn search_contacts(&self, q: &str, limit: i32) -> Result<Vec<User>> {
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
        &self,
        title: &str,
        about: &str,
        broadcast: bool,
        megagroup: bool,
    ) -> Result<Vec<Chat>> {
        let payload = rpc::build_create_channel(title, about, broadcast, megagroup);
        let result = self.invoke_raw(payload).await?;
        let chats = Self::chats_from_updates(&result, crate::types::CHANNELS_CREATE_CHANNEL)?;
        // Persist the fresh channel's access hash: it is what later
        // `-100…` id resolution (resolve_peer) resolves against.
        for chat in &chats {
            if let Chat::Channel { id: ChatId(cid), access_hash: Some(hash), .. } = chat {
                self.persist_peer_hash(&InputPeer::Channel {
                    channel_id: ChannelId(*cid),
                    access_hash: *hash,
                })
                .await;
            }
        }
        Ok(chats)
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
        Self::chats_from_updates(&result, crate::types::CHANNELS_INVITE_TO_CHANNEL)
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
        Self::chats_from_updates(&result, crate::types::CHANNELS_GET_CHANNELS)
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
        &self,
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
        &self,
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
            Peer::User { user_id } => Some(InputPeer::User { user_id: *user_id, access_hash: AccessHash(0) }),
            Peer::Chat { chat_id } => Some(InputPeer::Chat { chat_id: *chat_id }),
            Peer::Channel { channel_id } => {
                Some(InputPeer::Channel { channel_id: *channel_id, access_hash: AccessHash(0) })
            }
            Peer::None => None,
        }
    }

    /// Extract the `chats` vector from an `Updates*` response.
    pub(crate) fn chats_from_updates(data: &[u8], method_ctor: u32) -> Result<Vec<Chat>> {
        if std::env::var("MTPRSTO_DEBUG").is_ok() {
            println!(
                "DEBUG chats_from_updates body ({}b): {:02x?}",
                data.len(),
                &data[..data.len().min(160)]
            );
        }
        // The generated TL parsers (e.g. `MessagesInvitedUsers` ->
        // `Updates` -> `Update` -> `Message` -> `MessageAction`) match
        // over hundreds of constructors; in debug builds every arm's
        // locals get distinct stack slots, so a single nested parse can
        // need far more than the 8 MiB main-thread stack (observed
        // STATUS_STACK_OVERFLOW in the channel_admin example). Parse on
        // a dedicated thread with a generous stack — the same mitigation
        // recursive-descent parsers (incl. rustc) use. Release builds
        // collapse the frames, but the headroom is cheap either way.
        const PARSE_STACK: usize = 64 * 1024 * 1024;
        std::thread::scope(|scope| {
            let handle = std::thread::Builder::new()
                .stack_size(PARSE_STACK)
                .spawn_scoped(scope, move || Self::parse_chats_response(data, method_ctor))
                .map_err(|e| {
                    Error::Other(format!("failed to spawn parse thread: {e}"))
                })?;
            handle.join().map_err(|p| {
                Error::Other(format!("response parse panicked: {p:?}"))
            })?
        })
    }

    /// Schema-driven chat-list extraction: routes on the RESPONSE ctor,
    /// cross-checked against the generated method->response map
    /// (`expected_response_ctors`) when the method is known to the
    /// generator. Handles the three wire shapes chat-returning methods
    /// produce: `messages.chats`, `messages.invitedUsers` (chats nested
    /// in its Updates payload), and Updates containers.
    pub(crate) fn parse_chats_response(data: &[u8], method_ctor: u32) -> Result<Vec<Chat>> {
        let expected = crate::types::gen_fns::expected_response_ctors(method_ctor);
        let mut r = crate::serialize::TLReader::new(data);
        let ctor = r.read_u32()?;
        // The schema understates reality for some methods: production
        // answers channels.inviteToChannel (declared messages.InvitedUsers)
        // with a bare Updates# container for megagroups/bots. Updates
        // ctor ids are therefore always acceptable; the map only adds
        // method-specific expectations.
        const UPDATES_ID: u32 = 0x74ae4240;
        const UPDATES_COMBINED_ID: u32 = 0x725b04c3;
        const UPDATE_SHORT_ID: u32 = 0x78d4dec1;
        const UPDATES_TOO_LONG_ID: u32 = 0xe317af7e;
        let is_updates_shape = matches!(
            ctor,
            UPDATES_ID | UPDATES_COMBINED_ID | UPDATE_SHORT_ID | UPDATES_TOO_LONG_ID
        );
        if !expected.is_empty()
            && !expected.contains(&ctor)
            && !is_updates_shape
        {
            return Err(crate::error::Error::Serialization(format!(
                "unexpected response ctor {ctor:#x} for method {method_ctor:#x}"
            )));
        }
        match ctor {
            crate::types::MESSAGES_CHATS => {
                let n = r.read_vector_header()?;
                let mut chats = Vec::with_capacity(n.max(0) as usize);
                for _ in 0..n {
                    chats.push(Chat::read_from(&mut r)?);
                }
                Ok(chats)
            }
            crate::types::MESSAGES_INVITED_USERS => {
                // The router already consumed the wrapper ctor, so call
                // read_from on the FULL buffer — the generated parser
                // expects to read (and validate) the ctor itself.
                let invited = crate::types::MessagesInvitedUsers::read_from(
                    &mut crate::serialize::TLReader::new(data),
                )?;
                Ok(match invited.updates {
                    crate::types::GenUpdates::Updates { ref chats, .. }
                    | crate::types::GenUpdates::UpdatesCombined { ref chats, .. } => chats.clone(),
                    _ => Vec::new(),
                })
            }
            _ => {
                // Updates containers and everything else: reuse the
                // curated Updates parser (r position reset).
                let updates = crate::types::Updates::parse(data)?;
                Ok(updates.chats())
            }
        }
    }
}
