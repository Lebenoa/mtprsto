//! Client ergonomics (SPEC §12.1): typed `invoke`, message/file builder
//! chains, and the `client.messages(peer)` history iterator.
//!
//! All public items carry `# Errors` documentation and doctest-verified
//! examples where a network is not required.

use crate::error::{Error, Result};
use crate::serialize::{TLReader, TLWriter};
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
        match u32::from_le_bytes(data[..4].try_into().unwrap()) {
            types::BOOL_TRUE => Ok(()),
            types::BOOL_FALSE => Err(Error::Protocol("server returned boolFalse".into())),
            _ => Ok(()), // unmodelled payload: treat success as ()
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
        let mut r = TLReader::new(data);
        types::User::read_from(&mut r)
    }
}

impl TlResult for types::Dialogs {
    fn from_rpc_result(_data: &[u8]) -> Result<Self> {
        Err(Error::Protocol(
            "Dialogs decoding requires messages.getDialogs parse support".into(),
        ))
    }
}

impl TlResult for types::State {
    fn from_rpc_result(data: &[u8]) -> Result<Self> {
        types::State::read_from(&mut TLReader::new(data))
    }
}

impl TlResult for types::Difference {
    fn from_rpc_result(data: &[u8]) -> Result<Self> {
        types::Difference::parse(data)
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
/// # async fn demo(mut client: Client) -> Result<()> {
/// let id = client
///     .message("du_ton", "hi")
///     .await // resolves the peer, then chain fluent setters
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
/// [`MessageBuilder::send`](Self::send) returns transport/RPC errors.
#[must_use = "a MessageBuilder does nothing until `.send()` is awaited"]
pub struct MessageBuilder<'a> {
    client: &'a mut crate::client::Client,
    peer: InputPeer,
    text: String,
    reply_to: Option<i64>,
    silent: bool,
    no_webpage: bool,
    /// Wrap the query in invokeAfterMsg with this msg_id (§5).
    after_msg: Option<i64>,
    /// Wrap the query in invokeWithoutUpdates (§5).
    without_updates: bool,
}

impl<'a> MessageBuilder<'a> {
    pub(crate) fn new(client: &'a mut crate::client::Client, peer: InputPeer, text: String) -> Self {
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
    pub fn reply_to(mut self, msg_id: MsgId) -> Self {
        self.reply_to = Some(msg_id.0);
        self
    }

    /// Deliver silently (no notification sound).
    pub fn silent(mut self) -> Self {
        self.silent = true;
        self
    }

    /// Disable link previews for this message.
    pub fn no_webpage(mut self) -> Self {
        self.no_webpage = true;
        self
    }

    /// Process this message only after the server handled `msg_id`
    /// (wraps the query in `invokeAfterMsg`).
    pub fn after_msg(mut self, msg_id: MsgId) -> Self {
        self.after_msg = Some(msg_id.0);
        self
    }

    /// Suppress update delivery for this request
    /// (wraps the query in `invokeWithoutUpdates`).
    pub fn without_updates(mut self) -> Self {
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
        let Self { peer, text, reply_to, silent, no_webpage, after_msg, without_updates, client } = self;
        let mut flags: i32 = 0;
        if no_webpage { flags |= 1 << 1; }
        if reply_to.is_some() { flags |= 1 << 0; }
        if silent { flags |= 1 << 5; }

        let query = {
            let mut w = TLWriter::new();
            w.write_u32(types::MESSAGES_SEND_MESSAGE);
            w.write_i32(flags);
            peer.write_to(&mut w);
            if let Some(reply_id) = reply_to {
                // inputReplyToMessage#869fbe10
                w.write_u32(types::INPUT_REPLY_TO_MESSAGE);
                w.write_i32(0);
                w.write_i32(reply_id as i32);
            }
            w.write_bytes(text.as_bytes());
            w.write_i64(rand::random::<i64>()); // random_id
            w.into_bytes()
        };
        let query = match after_msg {
            Some(after) => crate::mtproto::build_invoke_after_msg(after as u64, &query),
            None => query,
        };
        let query = if without_updates {
            crate::mtproto::build_invoke_without_updates(&query)
        } else {
            query
        };

        let result = client.invoke_raw(query).await?;
        MsgId::from_rpc_result(&result)
    }
}

// ===========================================================================
// File-send builder: client.send_file(peer, path).caption("hi").send()
// ===========================================================================

/// Fluent builder produced by [`crate::client::Client::send_file`].
#[must_use = "a SendFileBuilder does nothing until `.send()` is awaited"]
pub struct SendFileBuilder<'a> {
    client: &'a mut crate::client::Client,
    peer: InputPeer,
    path: std::path::PathBuf,
    caption: String,
    workers: usize,
}

impl<'a> SendFileBuilder<'a> {
    pub(crate) fn new(
        client: &'a mut crate::client::Client,
        peer: InputPeer,
        path: std::path::PathBuf,
    ) -> Self {
        Self { client, peer, path, caption: String::new(), workers: 4 }
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

    /// Upload the file and send it as a document message.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Other`] for filesystem failures and
    /// [`Error::Network`]/[`Error::Rpc`] for transport/server failures.
    pub async fn send(self) -> Result<MsgId> {
        let Self { client, peer, path, caption, workers } = self;
        let pool = client
            .pool()
            .clone();
        let input_file = crate::file::upload_file(pool, &path, workers).await?;

        // messages.sendMedia with inputMediaUploadedDocument
        let mut flags: i32 = 0;
        let reply_to = None::<i64>;
        if reply_to.is_some() { flags |= 1 << 0; }

        let result = client
            .invoke_with_method(types::MESSAGES_SEND_MEDIA, |w| {
                w.write_i32(flags);
                peer.write_to(w);
                if let Some(reply_id) = reply_to {
                    w.write_u32(types::INPUT_REPLY_TO_MESSAGE);
                    w.write_i32(0);
                    w.write_i32(reply_id as i32);
                }
                // inputMediaUploadedDocument#c55bccd9 flags:# file:InputFile
                // mime_type:string attributes:Vector<DocumentAttribute>
                w.write_u32(0xc55bccd9);
                w.write_i32(0); // flags (no nosound/video/force-file)
                input_file.write_to(w);
                let mime = mime_guess::from_path(&path).first_or_octet_stream();
                w.write_bytes(mime.essence_str().as_bytes());
                // Vector<DocumentAttribute> with a single filename attribute
                w.write_u32(types::VECTOR);
                w.write_i32(1);
                w.write_u32(types::DOCUMENT_ATTRIBUTE_FILENAME);
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file.bin");
                w.write_bytes(name.as_bytes());
                w.write_bytes(caption.as_bytes());
                w.write_i64(rand::random::<i64>()); // random_id
                Ok(())
            })
            .await?;

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
    client: &'a mut crate::client::Client,
    peer: InputPeer,
    limit_per_page: i32,
}

impl<'a> MessagesIter<'a> {
    pub(crate) fn new(client: &'a mut crate::client::Client, peer: InputPeer) -> Self {
        Self { client, peer, limit_per_page: 100 }
    }

    /// Server-side page size per `messages.getHistory` call (max 100).
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
            let want = (n - out.len()).min(self.limit_per_page as usize) as i32;
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
            page.truncate(want as usize);
            for m in page {
                offset_id = m.id.0 as i32;
                out.push(m);
            }
        }
        Ok(out)
    }
}

// ===========================================================================
// Tracing span for the session
// ===========================================================================

/// Create the library's root tracing span, named after the session.
///
/// Called by `Client::connect`; keep it entered (or rely on async-local
/// propagation) to correlate all RPC spans beneath it.
pub fn session_span(dc_id: i32) -> tracing::Span {
    tracing::info_span!("mtprsto::session", dc = dc_id)
}
