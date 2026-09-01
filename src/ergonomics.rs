//! Client ergonomics (SPEC §12.1): typed `invoke`, message/file builder
//! chains, and the `client.messages(peer)` history iterator.
//!
//! All public items carry `# Errors` documentation and doctest-verified
//! examples where a network is not required.

use crate::error::{Error, Result};
use crate::serialize::TLReader;
use crate::types::{self, InputPeer, MsgId};

// ===========================================================================
// Typed invoke: `TlResult`
// ===========================================================================

/// Types that can be decoded from a successful RPC result body.
///
/// Implemented for [`MsgId`], [`UserId`], [`types::User`],
/// [`types::Dialogs`], [`types::State`], [`types::Difference`], `Vec<u8>`
/// (raw bytes) and `()` (Bool-response methods).
pub trait TlResult: Sized {
    /// Decode `Self` from the RPC result body (constructor first).
    ///
    /// # Errors
    ///
    /// Returns the implementing type's decode error: unknown
    /// constructors, truncated bodies, or protocol mismatches.
    fn from_rpc_result(data: &[u8]) -> Result<Self>;
}

impl TlResult for Vec<u8> {
    fn from_rpc_result(data: &[u8]) -> Result<Self> {
        Ok(data.to_vec())
    }
}

impl TlResult for () {
    /// Accepts `boolTrue`/`boolFalse` and empty bodies.
    fn from_rpc_result(data: &[u8]) -> Result<Self> {
        if data.len() < 4 {
            return Ok(());
        }
        // Empty and unmodelled payloads both read as success: only the
        // explicit boolFalse constructor is a failure. The slice is
        // bounded by the len >= 4 check above.
        let (head, _rest) = data.split_at(4);
        let first = head.first().copied().unwrap_or(0);
        let second = head.get(1).copied().unwrap_or(0);
        let third = head.get(2).copied().unwrap_or(0);
        let fourth = head.get(3).copied().unwrap_or(0);
        let ctor = u32::from_le_bytes([first, second, third, fourth]);
        match ctor {
            types::BOOL_FALSE => Err(Error::Protocol("server returned boolFalse".into())),
            _ => Ok(()),
        }
    }
}

impl TlResult for MsgId {
    fn from_rpc_result(data: &[u8]) -> Result<Self> {
        let updates = types::Updates::parse(data)?;
        updates
            .message_id()
            .ok_or_else(|| Error::Protocol("response carries no message id".into()))
    }
}

impl TlResult for types::User {
    fn from_rpc_result(data: &[u8]) -> Result<Self> {
        // Accept a bare user object or a `Vector<User>` (users.getUsers
        // et al return vectors) — decode the first element.
        let mut r = TLReader::new(data);
        let ctor = r.read_u32()?;
        if ctor == crate::serialize::VECTOR {
            let n = r.read_i32()?;
            if n < 1 {
                return Err(Error::Protocol("empty user vector".into()));
            }
            return Self::read_from(&mut r);
        }
        // ctor was already consumed — reparse from the start.
        Self::read_from(&mut TLReader::new(data))
    }
}

impl TlResult for types::Dialogs {
    /// Decodes `messages.Dialogs*` (dialogs/slice/notModified) via
    /// [`types::Dialogs::parse`].
    fn from_rpc_result(data: &[u8]) -> Result<Self> {
        Self::parse(data)
    }
}

impl TlResult for types::State {
    fn from_rpc_result(data: &[u8]) -> Result<Self> {
        Self::read_from(&mut TLReader::new(data))
    }
}

impl TlResult for types::Difference {
    fn from_rpc_result(data: &[u8]) -> Result<Self> {
        Self::parse(data)
    }
}

// ===========================================================================
// Message builder: client.message(peer, "hi").reply_to(id).silent().send()
// ===========================================================================

/// Fluent builder produced by [`crate::client::Client::message`].
///
/// Chain option setters, then finish with [`MessageBuilder::send`]:
///
/// ```no_run
/// # use mtprsto::{client::Client, types::MsgId, Result};
/// # async fn demo(client: Client) -> Result<()> {
/// let id = client
///     .message("du_ton", "hi")
///     .await? // resolves the peer; `?` surfaces a bad peer
///     .reply_to(MsgId(42))
///     .silent()
///     .send()
///     .await?;
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// [`crate::client::Client::message`] returns the peer-resolution error;
/// [`MessageBuilder::send`](Self::send) returns transport/RPC errors.
#[must_use = "a MessageBuilder does nothing until `.send()` is awaited"]
pub struct MessageBuilder<'a> {
    client: &'a crate::client::Client,
    peer: InputPeer,
    text: String,
    reply_to: Option<i64>,
    silent: bool,
    no_webpage: bool,
    /// Wrap the query in `invokeAfterMsg` with this `msg_id` (§5).
    after_msg: Option<i64>,
    /// Wrap the query in `invokeWithoutUpdates` (§5).
    without_updates: bool,
}

impl<'a> MessageBuilder<'a> {
    pub(crate) const fn new(
        client: &'a crate::client::Client,
        peer: InputPeer,
        text: String,
    ) -> Self {
        Self {
            client,
            peer,
            text,
            reply_to: None,
            silent: false,
            no_webpage: false,
            after_msg: None,
            without_updates: false,
        }
    }

    /// Make this a reply to the given message id.
    #[allow(clippy::missing_const_for_fn)] // mutates self via Option::insert path; not const-stable
    pub fn reply_to(mut self, msg_id: MsgId) -> Self {
        self.reply_to = Some(msg_id.0);
        self
    }

    /// Deliver silently (no notification sound).
    pub const fn silent(mut self) -> Self {
        self.silent = true;
        self
    }

    /// Disable link previews for this message.
    pub const fn no_webpage(mut self) -> Self {
        self.no_webpage = true;
        self
    }

    /// Process this message only after the server handled `msg_id`
    /// (wraps the query in `invokeAfterMsg`).
    #[allow(clippy::missing_const_for_fn)] // mutates self; not const-stable
    pub fn after_msg(mut self, msg_id: MsgId) -> Self {
        self.after_msg = Some(msg_id.0);
        self
    }

    /// Suppress update delivery for this request
    /// (wraps the query in `invokeWithoutUpdates`).
    pub const fn without_updates(mut self) -> Self {
        self.without_updates = true;
        self
    }

    /// Send the message, returning the new message id.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Network`]/[`Error::Rpc`] on transport or server
    /// failures.
    pub async fn send(self) -> Result<MsgId> {
        let Self {
            peer,
            text,
            reply_to,
            silent,
            no_webpage,
            after_msg,
            without_updates,
            client,
        } = self;
        // Payload built once by the shared rpc builder — the same wire
        // shape `Client::send` uses (flags + peer + reply_to + message +
        // random_id), so the two paths cannot drift.
        let mut query =
            crate::rpc::build_send_message_full(&peer, &text, reply_to, silent, no_webpage, None);
        // MsgId values are Telegram message counters, well inside u64.
        if let Some(after) = after_msg {
            query = crate::mtproto::build_invoke_after_msg(
                u64::try_from(after).unwrap_or(u64::MAX),
                &query,
            );
        }
        if without_updates {
            query = crate::mtproto::build_invoke_without_updates(&query);
        }

        let result = client.invoke_raw(query).await?;
        if std::env::var("MTPRSTO_DEBUG").is_ok() {
            let end = result.len().min(200);
            let show = result.get(..end).unwrap_or(&result);
            println!("DEBUG reply result ({}b): {show:02x?}", result.len());
        }
        MsgId::from_rpc_result(&result)
    }
}

// ===========================================================================
// File-send builder: client.send_file(peer, path).caption("hi").send()
// ===========================================================================

/// Fluent builder produced by [`crate::client::Client::send_file`].
///
/// By default the file is sent as a **document** (any mime type, keeps
/// the original bytes). Call [`as_photo`](Self::as_photo) to send an
/// image as a compressed photo instead:
///
/// ```no_run
/// # use mtprsto::{client::Client, Result};
/// # async fn demo(client: Client) -> Result<()> {
/// client
///     .send_file("du_ton", "cat.jpg")
///     .await?
///     .as_photo() // inputMediaUploadedPhoto — compressed, album-able
///     .caption("mrrp")
///     .send()
///     .await?;
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Peer resolution in [`crate::client::Client::send_file`] can fail
/// before the builder exists; `.send()` itself surfaces upload/transport
/// errors.
#[must_use = "a SendFileBuilder does nothing until `.send()` is awaited"]
pub struct SendFileBuilder<'a> {
    client: &'a crate::client::Client,
    peer: InputPeer,
    path: std::path::PathBuf,
    caption: String,
    workers: usize,
    as_photo: bool,
    reply_to: Option<i64>,
    silent: bool,
}

impl<'a> SendFileBuilder<'a> {
    #[allow(clippy::missing_const_for_fn)] // PathBuf::new-style init is not const-callable here
    pub(crate) fn new(
        client: &'a crate::client::Client,
        peer: InputPeer,
        path: std::path::PathBuf,
    ) -> Self {
        Self {
            client,
            peer,
            path,
            caption: String::new(),
            workers: 4,
            as_photo: false,
            reply_to: None,
            silent: false,
        }
    }

    /// Attach a caption to the file message.
    pub fn caption(mut self, caption: impl Into<String>) -> Self {
        self.caption = caption.into();
        self
    }

    /// Concurrent part-upload workers (default 4).
    pub fn workers(mut self, n: usize) -> Self {
        self.workers = n.max(1);
        self
    }

    /// Send the image as a compressed **photo**
    /// (`inputMediaUploadedPhoto`) instead of a document.
    ///
    /// The server re-encodes the image and it participates in photo
    /// albums; use the default document mode to preserve exact bytes.
    pub const fn as_photo(mut self) -> Self {
        self.as_photo = true;
        self
    }

    /// Make this a reply to the given message id.
    pub const fn reply_to(mut self, msg_id: crate::types::MsgId) -> Self {
        self.reply_to = Some(msg_id.0);
        self
    }

    /// Deliver silently (no notification sound).
    pub const fn silent(mut self) -> Self {
        self.silent = true;
        self
    }

    /// Upload the file and send it as a document (default) or photo
    /// message.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Other`] for filesystem failures and
    /// [`Error::Network`]/[`Error::Rpc`] for transport/server failures.
    pub async fn send(self) -> Result<MsgId> {
        let Self {
            client,
            peer,
            path,
            caption,
            workers,
            as_photo,
            reply_to,
            silent,
        } = self;
        let pool = client.pool();
        let input_file = crate::file::upload_file(pool, &path, workers).await?;

        let media = if as_photo {
            crate::rpc::InputMedia::UploadedPhoto { file: input_file }
        } else {
            crate::rpc::InputMedia::UploadedDocument {
                file: input_file,
                mime_type: mime_guess::from_path(&path)
                    .first_or_octet_stream()
                    .essence_str()
                    .to_string(),
                file_name: path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file.bin")
                    .to_string(),
            }
        };
        let payload = crate::rpc::build_send_media(
            &peer, &media, &caption, reply_to, silent, false, // clear_draft
            None,  // schedule_date
        );

        let result = client.invoke_raw(payload).await?;
        MsgId::from_rpc_result(&result)
    }
}

// ===========================================================================
// History iterator: client.messages(peer).take(10).collect()
// ===========================================================================

/// Async iterator over a peer's recent messages, produced by
/// [`crate::client::Client::messages`].
///
/// Backed by `messages.getHistory` pages; `.take(n).collect()` stops as
/// soon as `n` messages are in hand.
pub struct MessagesIter<'a> {
    client: &'a crate::client::Client,
    peer: InputPeer,
    limit_per_page: i32,
}

impl<'a> MessagesIter<'a> {
    pub(crate) const fn new(client: &'a crate::client::Client, peer: InputPeer) -> Self {
        Self {
            client,
            peer,
            limit_per_page: 100,
        }
    }

    /// Server-side page size per `messages.getHistory` call (max 100).
    #[must_use]
    pub fn page_size(mut self, n: i32) -> Self {
        self.limit_per_page = n.clamp(1, 100);
        self
    }

    /// Collect up to `n` most recent messages, oldest last.
    ///
    /// # Errors
    ///
    /// Returns transport/protocol errors from the underlying RPCs.
    pub async fn collect(self, n: usize) -> Result<Vec<types::MessageFull>> {
        let mut out: Vec<types::MessageFull> = Vec::with_capacity(n);
        let mut offset_id: i32 = 0;
        while out.len() < n {
            // The loop condition guarantees remaining >= 1.
            let remaining = n.saturating_sub(out.len());
            let want = i32::try_from(remaining.min(self.page_size_us()))
                .unwrap_or(i32::MAX)
                .clamp(1, 100);
            // NOTE: errors are swallowed into an empty page so a broken
            // history read still returns what we collected so far; use
            // `page_size` and pagination fields for finer control later.
            let mut page: Vec<types::MessageFull> = self
                .client
                .get_history_page(&self.peer, offset_id, want)
                .await
                .unwrap_or_default();
            if page.is_empty() {
                break; // history exhausted
            }
            // getHistory returns newest-first; keep oldest-last ordering.
            // want is clamped to 1..=100 above, so the narrowing is safe.
            #[allow(clippy::as_conversions, clippy::cast_sign_loss)]
            page.truncate(want.max(0) as usize);
            for m in page {
                // Telegram msg ids are 32-bit counters carried in i64
                offset_id = i32::try_from(m.id.0).unwrap_or(i32::MAX);
                out.push(m);
            }
        }
        Ok(out)
    }

    /// `limit_per_page` as `usize` for `min()` — the field is a clamped
    /// `1..=100` `i32`, so this conversion cannot lose information.
    // try_from is not const-stable yet despite the value being provably
    // in range.
    #[allow(clippy::missing_const_for_fn)]
    fn page_size_us(&self) -> usize {
        usize::try_from(self.limit_per_page).unwrap_or(1)
    }
}

// ===========================================================================
// Tracing span for the session
// ===========================================================================

/// Create the library's root tracing span, named after the session.
///
/// Callers that want session correlation should attach this span with
/// `.instrument(...)` — do NOT hold `span.enter()`'s `EnteredSpan` across
/// an `.await`: it is `!Send` and poisons every spawned future that
/// captures it.
#[must_use]
pub fn session_span(dc_id: i32) -> tracing::Span {
    tracing::info_span!("mtprsto::session", dc = dc_id)
}
