//! Connection pool for `MTProto` sessions.
//!
//! `SenderPool` manages multiple TCP connections to a Telegram DC,
//! load-balancing RPC requests across them and handling reconnection.
//!
//! # Architecture
//!
//! ```text
//! Client
//!   └─ SenderPool (per DC)
//!        ├─ Connection 0 (main)
//!        ├─ Connection 1 (aux)
//!        ├─ Connection 2 (aux)
//!        └─ Connection 3 (aux, scales up to 8)
//!
//! Each Connection is wrapped in its own Mutex so I/O on one
//! doesn't block the others.
//! ```

// Wire-format engine: byte wrangling is this module's job — TL field
// order, int32 wire ids, offset arithmetic over length-checked
// buffers. The cast/index/arithmetic classes are inherent to that
// job; they are relaxed once here, invariants held by hand. Every
// other lint still applies.
#![allow(clippy::as_conversions, clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::string_slice
)]
#![allow(clippy::unreadable_literal)] // ids/hex quoted verbatim from the TL schema

use crate::error::{Error, Result};
use crate::mtproto::MtProtoSession;
use crate::transport;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// Which wire transport the pool prefers when opening connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransportPolicy {
    /// TCP + Obfuscated2 only. Never touches WebSocket. Default.
    #[default]
    TcpOnly,
    /// TCP first; after repeated TCP failures on this DC, fall back to
    /// WebSocket (`wss://`) for subsequent connects until TCP succeeds again.
    Auto,
}

/// Protocol-level timings and knobs (ack batching, keepalive, salt
/// refresh, padding). Defaults follow gotd/mtproto practice; tighten or
/// loosen via [`crate::Client::builder`].
#[derive(Debug, Clone)]
pub struct ProtocolConfig {
    /// Interval between keepalive `ping_delay_disconnect` messages.
    pub ping_interval: std::time::Duration,
    /// Reconnect if no pong arrives within this window.
    pub pong_timeout: std::time::Duration,
    /// How often to pre-fetch future server salts.
    pub salt_refresh_interval: std::time::Duration,
    /// Send a batched `msgs_ack` once this many results are pending.
    pub ack_batch_max: usize,
    /// ...or after this long, whichever comes first.
    pub ack_flush_interval: std::time::Duration,
    /// Anti-fingerprinting random padding blocks on every encrypted
    /// message (gotd/Telegram-Desktop parity). Disable only if a proxy
    /// or test harness needs deterministic message sizes.
    pub random_padding: bool,
    /// Compress outgoing TL payloads above this size with
    /// `gzip_packed` (gotd default: 1024; `0` disables).
    pub compress_threshold: usize,
}

impl Default for ProtocolConfig {
    fn default() -> Self {
        Self {
            ping_interval: PING_INTERVAL,
            pong_timeout: PONG_TIMEOUT,
            salt_refresh_interval: SALT_REFRESH_INTERVAL,
            ack_batch_max: ACK_BATCH_MAX,
            ack_flush_interval: ACK_FLUSH_INTERVAL,
            random_padding: true,
            compress_threshold: 1024,
        }
    }
}

/// Configuration for the connection pool.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Minimum number of connections (main + aux).
    pub min_connections: usize,
    /// Maximum number of connections.
    pub max_connections: usize,
    /// Which transport to prefer (see [`TransportPolicy`]). Default `TcpOnly`.
    pub transport_policy: TransportPolicy,
    /// Threshold for scaling up: if inflight > 2 * `aux_count` for > 10s.
    pub scale_up_threshold: u32,
    /// Seconds of high load before scaling up.
    pub scale_up_duration_secs: u64,
    /// Seconds of low load before scaling back down.
    pub scale_down_duration_secs: u64,
    /// TCP keepalive interval in seconds.
    pub keepalive_secs: u64,
    /// Maximum reconnect attempts before giving up.
    pub max_reconnect_attempts: u32,
    /// Base reconnect delay (exponential backoff, capped at 60s).
    pub reconnect_base_delay_ms: u64,
}

/// Default pool configuration.
impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_connections: 4,
            max_connections: 8,
            scale_up_threshold: 2,
            scale_up_duration_secs: 10,
            scale_down_duration_secs: 60,
            keepalive_secs: 30,
            max_reconnect_attempts: 5,
            reconnect_base_delay_ms: 1000,
            transport_policy: TransportPolicy::TcpOnly,
        }
    }
}

/// A single connection to a Telegram DC: an Obfuscated2 codec (CTR streams
/// plus `TcpStream`) behind its own mutex so I/O on one connection does not
/// block the others.
struct PooledConnection {
    codec: Mutex<PoolCodec>,
    /// One-in-flight RPC permit: held for a whole send→answer exchange.
    /// Concurrent callers that round-robin onto a busy connection wait
    /// here instead of interleaving frames — a second request's answer
    /// would otherwise be drained by the first waiter's frame loop
    /// (message ids correlate strictly per connection).
    rpc_permit: Mutex<()>,
}

/// A codec over any supported wire transport. `send_frame`/`recv_frame`
/// behave identically regardless of the variant.
enum PoolCodec {
    /// Plain Intermediate — gotd's default for non-obfuscated-only DC
    /// options, verified against production DCs.
    Tcp(transport::IntermediateTransport),
    #[cfg(feature = "ws")]
    Ws(transport::Obfuscated2Transport<crate::ws::WsTransport>),
}

impl PoolCodec {
    async fn send_frame(&mut self, payload: &[u8]) -> Result<()> {
        match self {
            Self::Tcp(c) => c.send(payload).await,
            #[cfg(feature = "ws")]
            Self::Ws(c) => c.send_frame(payload).await,
        }
    }

    async fn recv_frame(&mut self) -> Result<Vec<u8>> {
        match self {
            Self::Tcp(c) => c.recv().await,
            #[cfg(feature = "ws")]
            Self::Ws(c) => c.recv_frame().await,
        }
    }
}

/// What [`SenderPool::connect_one`] actually did, so the tracker owner can
/// update failover state correctly (TCP success resets; TCP failure counts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectOutcome {
    /// TCP connected (tracker resets).
    TcpOk,
    /// TCP failed and no WS connect happened (failure counts).
    TcpFail,
    /// WS was preferred and connected (tracker untouched).
    #[cfg(feature = "ws")]
    Ws,
    /// TCP failed, WS fallback connected (failure counts).
    #[cfg(feature = "ws")]
    TcpFailThenWs,
}

/// Consecutive-TCP-failure tracker backing `TransportPolicy::Auto`:
/// 2 failures within 5 minutes switch new connects to `wss://`; a TCP
/// success resets the tracker.
#[derive(Debug, Default)]
struct TcpFailover {
    recent: Vec<std::time::Instant>,
}

impl TcpFailover {
    const WINDOW: std::time::Duration = std::time::Duration::from_secs(300);
    const THRESHOLD: usize = 2;

    fn record_failure(&mut self) {
        let now = std::time::Instant::now();
        self.recent
            .retain(|t| now.duration_since(*t) <= Self::WINDOW);
        self.recent.push(now);
    }

    fn record_success(&mut self) {
        self.recent.clear();
    }

    const fn should_prefer_ws(&self) -> bool {
        self.recent.len() >= Self::THRESHOLD
    }
}

/// A pool of connections to a single DC.
pub struct SenderPool {
    /// DC ID this pool connects to.
    dc_id: i32,
    /// API ID used in the initConnection wrapper for RPCs.
    api_id: i32,
    /// Shared session (auth key, salt, etc.).
    session: Arc<RwLock<MtProtoSession>>,
    /// Active connections, each with its own mutex.
    connections: Vec<Arc<PooledConnection>>,
    /// TCP failure tracker for `TransportPolicy::Auto` failover.
    tcp_failover: Mutex<TcpFailover>,
    /// Pool configuration.
    config: PoolConfig,
    /// Protocol timers/knobs (see [`ProtocolConfig`]).
    protocol: ProtocolConfig,
    /// Next connection index for round-robin.
    next_index: Mutex<usize>,
    /// Received `msg_ids` awaiting a batched `msgs_ack` (flushed at
    /// [`ProtocolConfig::ack_batch_max`] pending or after
    /// [`ProtocolConfig::ack_flush_interval`], per
    /// SPEC §5.4).
    pending_acks: Arc<Mutex<Vec<u64>>>,
}

/// Flush pending acks once this many are queued (SPEC §5.4: "16 pending").
const ACK_BATCH_MAX: usize = 16;
/// Flush pending acks at least this often (SPEC §5.4: "every ~10 s").
const ACK_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
/// Ping every connection on this cadence (SPEC BS-1: keepalive every 30 s).
const PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
/// Reconnect a connection after this long without a pong (SPEC BS-1: 90 s).
const PONG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
/// Refresh the server salt on this cadence (SPEC §9: salt validity ~30 min).
const SALT_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_mins(25);

impl SenderPool {
    /// Create a new pool for the given DC with an existing session.
    #[must_use]
    pub fn new(
        dc_id: i32,
        api_id: i32,
        session: MtProtoSession,
        config: PoolConfig,
        protocol: ProtocolConfig,
    ) -> Self {
        Self {
            dc_id,
            api_id,
            session: Arc::new(RwLock::new(session)),
            connections: Vec::new(),
            tcp_failover: Mutex::new(TcpFailover::default()),
            config,
            protocol,
            next_index: Mutex::new(0),
            pending_acks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Create a pool with just a DC ID and config (no session yet).
    #[must_use]
    pub fn without_session(dc_id: i32, config: PoolConfig) -> Self {
        let session = MtProtoSession::new(vec![0u8; 256], 0);
        Self::new(dc_id, 0, session, config, ProtocolConfig::default())
    }

    /// Set the session on the pool.
    pub async fn set_session(&self, session: MtProtoSession) {
        let mut s = self.session.write().await;
        *s = session;
    }

    /// Get a read reference to the session.
    pub async fn session(&self) -> tokio::sync::RwLockReadGuard<'_, MtProtoSession> {
        self.session.read().await
    }

    /// Get a mutable reference to the session.
    pub async fn session_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, MtProtoSession> {
        self.session.write().await
    }

    /// Open the initial connections to the DC.
    ///
    /// Each connection performs the Obfuscated2 handshake; all framed I/O
    /// goes through the per-connection codec (`send_frame`/`recv_frame`).
    ///
    /// # Errors
    ///
    /// Returns an error when a connection to the DC cannot be established
    /// for the main or any auxiliary connection.
    pub async fn connect(&mut self) -> Result<()> {
        if !self.connections.is_empty() {
            return Ok(());
        }

        // Main connection: failure/success updates the tracker inline.
        let prefer_ws = self.tcp_failover.lock().await.should_prefer_ws();
        let (outcome, res) =
            Self::connect_one(self.dc_id, self.config.transport_policy, prefer_ws).await;
        self.update_failover(outcome).await;
        let codec = res?;
        self.connections.push(Arc::new(PooledConnection {
            codec: Mutex::new(codec),
            rpc_permit: Mutex::new(()),
        }));

        // Open additional aux connections in parallel — log failures but
        // don't abort (tokio::spawn needs 'static futures, so the tracker
        // is snapshotted up front and outcomes are folded in afterwards).
        let aux_count =
            (self.config.min_connections.min(self.config.max_connections)).saturating_sub(1);
        let dc_id = self.dc_id;
        let policy = self.config.transport_policy;
        let prefer_ws = self.tcp_failover.lock().await.should_prefer_ws();
        let mut handles = Vec::with_capacity(aux_count);
        for _ in 0..aux_count {
            handles.push(tokio::spawn(async move {
                Self::connect_one(dc_id, policy, prefer_ws).await.1
            }));
        }
        let results = futures_collect(handles).await;
        for (i, res) in results.into_iter().enumerate() {
            match res {
                Ok(codec) => {
                    self.connections.push(Arc::new(PooledConnection {
                        codec: Mutex::new(codec),
                        rpc_permit: Mutex::new(()),
                    }));
                }
                Err(e) => {
                    tracing::warn!(
                        "aux connection {} to DC {} failed: {}",
                        i + 1,
                        self.dc_id,
                        e
                    );
                }
            }
        }

        Ok(())
    }

    /// Fold a connect outcome into the failover tracker.
    async fn update_failover(&self, outcome: ConnectOutcome) {
        let mut t = self.tcp_failover.lock().await;
        match outcome {
            ConnectOutcome::TcpOk => t.record_success(),
            ConnectOutcome::TcpFail => t.record_failure(),
            #[cfg(feature = "ws")]
            ConnectOutcome::Ws => {}
            #[cfg(feature = "ws")]
            ConnectOutcome::TcpFailThenWs => t.record_failure(),
        }
    }

    /// Round-robin over connections. Each request/response pair is confined
    /// to a single connection (held under its mutex for the whole exchange),
    /// so a response can never be drained by a different request. NOTE: any
    /// server-initiated frame that arrives before the RPC result will be
    /// returned as if it were the result; updates are not yet routed to a
    /// separate queue, and the write-only ack is not msg_id-correlated.
    ///
    /// On I/O error the dead connection is reconnected transparently and the
    /// SAME encrypted payload is retried once. `MTProto` dedupes by `msg_id`
    /// (identical bytes ⇒ identical `msg_id`), so the server treats the retry
    /// as a retransmit, not a new request — safe even for non-idempotent
    /// methods.
    ///
    /// # Errors
    ///
    /// Returns an error when the pool has no connections, the reconnect
    /// after an I/O error fails, or the retried send fails again.
    pub async fn send_raw(&self, data: &[u8]) -> Result<Vec<u8>> {
        if self.connections.is_empty() {
            return Err(Error::Transport("pool has no connections".into()));
        }

        let idx = {
            let mut ni = self.next_index.lock().await;
            let idx = *ni;
            *ni = (*ni + 1) % self.connections.len();
            idx
        };

        let conn = &self.connections[idx];

        match Self::send_on_connection(conn, data).await {
            Ok(resp) => Ok(resp),
            Err(Error::Network(e)) => {
                tracing::warn!(
                    "I/O error on connection {} to DC {}: {}",
                    idx,
                    self.dc_id,
                    e
                );
                // Reconnect the dead connection, then retry once (same bytes)
                self.reconnect_connection(conn).await?;
                Self::send_on_connection(conn, data).await
            }
            Err(e) => Err(e),
        }
    }

    /// Send and receive one obfuscated frame on a single connection
    /// (lock its codec, do I/O, unlock).
    async fn send_on_connection(conn: &PooledConnection, data: &[u8]) -> Result<Vec<u8>> {
        let mut codec = conn.codec.lock().await;
        codec.send_frame(data).await?;
        codec.recv_frame().await
    }

    /// Send an encrypted message, allocate `msg_id`/`seq_no` atomically,
    /// and receive the response.
    ///
    /// # Errors
    ///
    /// Propagates errors from the underlying `send_raw` round trip.
    #[tracing::instrument(name = "mtprsto::send_encrypted", skip_all, err)]
    #[allow(clippy::significant_drop_tightening)] // the scoped lock blocks already end at last use
    pub async fn send_encrypted(&self, payload: &[u8]) -> Result<(u64, Vec<u8>)> {
        // Allocate msg_id and seq_no under write lock
        let (msg_id, seq_no) = {
            let mut session = self.session.write().await;
            let msg_id = session.next_msg_id();
            let seq_no = session.next_seq_no(true);
            (msg_id, seq_no)
        };

        // Encrypt under read lock
        let encrypted = {
            let session = self.session.read().await;
            session.encrypt_message(payload, msg_id, seq_no)
        };

        let response = self.send_raw(&encrypted).await?;
        Ok((msg_id, response))
    }

    /// High-level RPC: wraps method bytes in `invokeWithLayer`, encrypts,
    /// sends on one connection, decrypts the response, sends a write-only
    /// ack, unwraps gzip/`rpc_result`, classifies `rpc_error`, and returns the
    /// inner result bytes.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure, protocol-level rejection
    /// (`rpc_error`, bad-message notification, session state that never
    /// settles), or a response that never arrives within the read timeout.
    ///
    /// # Panics
    ///
    /// Cannot panic: the retry loop returns on every path, so the trailing
    /// `unreachable!` guard is never actually reached.
    #[tracing::instrument(name = "mtprsto::send_rpc", skip_all, err)]
    #[allow(clippy::too_many_lines)] // one function owning the RPC lifecycle: send → retry → classify → ack; splitting it would scatter the msg_id state machine
    pub async fn send_rpc(&self, method_bytes: &[u8]) -> Result<Vec<u8>> {
        use crate::serialize::{
            BAD_SERVER_SALT, MSGS_ACK, NEW_SERVER_SALT, NEW_SESSION_CREATED, PONG, RPC_RESULT,
        };

        // Every network read is bounded: a blackholed/dead connection
        // (or a server that just stops answering a dialect) must not
        // block the caller — or the runtime — forever.
        const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

        // invokeWithLayer(initConnection(query)) — the server rejects bare
        // RPCs with INPUT_FETCH_ERROR / API_ID_* errors because it can't
        // establish the client context.
        let full_payload = crate::mtproto::build_invoke_with_layer(
            crate::api::API_LAYER,
            &crate::mtproto::build_init_connection(
                self.api_id,
                "mtprsto",
                "unknown",
                env!("CARGO_PKG_VERSION"),
                "en",
                method_bytes,
            ),
        );

        // Service notifications (bad_server_salt, new_session_created)
        // mean the request was NOT processed — adopt the announced state
        // and re-send. Anything else keeps the request, so late answers
        // are DRAINED on the same connection instead of re-sent.
        // Pick ONE connection for the whole RPC (including service-state
        // re-sends): hopping connections re-earns new_session_created on
        // every fresh socket.
        let idx = {
            let mut ni = self.next_index.lock().await;
            let idx = *ni;
            *ni = (*ni + 1) % self.connections.len().max(1);
            idx
        };
        let conn = self
            .connections
            .get(idx)
            .ok_or_else(|| Error::Transport("pool has no connections".into()))?;
        // One in-flight RPC per connection: the permit is held across
        // every attempt of this exchange (including service-state
        // re-sends) so a concurrent caller can never interleave frames
        // on the same socket — answers correlate strictly by msg_id.
        let _permit = conn.rpc_permit.lock().await;
        // The scoped session/codec guards below drop at their last use —
        // clippy's drop-point model cannot see that through the async
        // block (significant_drop_tightening / _in_scrutinee).
        #[allow(
            clippy::significant_drop_tightening,
            clippy::significant_drop_in_scrutinee
        )]
        'outer: for attempt in 0..4u32 {
            let (msg_id, mut response) = match async {
                let (msg_id, encrypted) = {
                    let mut session = self.session.write().await;
                    let msg_id = session.next_msg_id();
                    let seq_no = session.next_seq_no(true);
                    (
                        msg_id,
                        session.encrypt_message(&full_payload, msg_id, seq_no),
                    )
                };
                let mut codec = conn.codec.lock().await;
                tokio::time::timeout(READ_TIMEOUT, codec.send_frame(&encrypted))
                    .await
                    .map_err(|_| Error::Transport("send_frame read timed out".into()))??;
                let response = tokio::time::timeout(READ_TIMEOUT, codec.recv_frame())
                    .await
                    .map_err(|_| Error::Transport("read timed out".into()))??;
                Ok::<_, Error>((msg_id, response))
            }
            .await
            {
                Ok(v) => v,
                Err(Error::Network(e)) => {
                    tracing::warn!("I/O error on connection {idx} to DC {}: {e}", self.dc_id);
                    self.reconnect_connection(conn).await?;
                    continue; // same encrypted bytes re-sent; server dedupes
                }
                Err(Error::Transport(msg)) if msg.contains("read timed out") => {
                    tracing::warn!(
                        "read timed out on connection {idx} to DC {}: {msg}",
                        self.dc_id
                    );
                    self.reconnect_connection(conn).await?;
                    continue; // re-send on a fresh connection
                }
                Err(e) => return Err(e),
            };

            // Frame loop: our answer may be preceded by service messages
            // or delayed behind an answer to a previous (already-processed)
            // attempt on this connection.
            let mut frames = 0u32;
            loop {
                let (resp_msg_id, plaintext) = {
                    let mut session = self.session.write().await;
                    session.decrypt_message(&response)?
                };
                self.queue_ack(resp_msg_id).await;
                let body = Self::unwrap_gzip(&plaintext)?;
                let items = Self::container_items(&body)?;
                // gzip_packed also wraps INDIVIDUAL items inside the
                // container (large rpc payloads get compressed per-item
                // while service messages ride along uncompressed).
                let mut plain_items = Vec::with_capacity(items.len());
                for item in items {
                    plain_items.push(Self::unwrap_item_gzip(item)?);
                }

                let mut re_send = false;
                let mut conclusive = false;
                for item in &plain_items {
                    if item.len() < 4 {
                        continue;
                    }
                    let ctor = u32::from_le_bytes(
                        // invariant: the `item.len() < 4` guard above
                        // guarantees a full 4-byte constructor here
                        #[allow(clippy::unwrap_used)]
                        item[0..4].try_into().unwrap(),
                    );
                    match ctor {
                        BAD_SERVER_SALT => {
                            // The query was NOT processed — adopt the fresh
                            // salt and re-send it.
                            self.adopt_service_state(ctor, item).await;
                            re_send = true;
                            conclusive = true;
                        }
                        NEW_SERVER_SALT | NEW_SESSION_CREATED => {
                            // Service state adoption only. The server DOES
                            // process the triggering message when it sends
                            // new_session_created — re-sending would execute
                            // the query twice (double sendCode, self-inflicted
                            // flood on the duplicate).
                            self.adopt_service_state(ctor, item).await;
                        }
                        crate::serialize::BAD_MSG_NOTIFICATION => {
                            let (bad_msg_id, _seqno, code) =
                                crate::mtproto::parse_bad_msg_notification(item)?;
                            return Err(classify_bad_msg(code, bad_msg_id));
                        }
                        crate::serialize::FUTURE_SALTS | crate::serialize::MSGS_STATE_INFO => {
                            return Ok(item.clone());
                        }
                        // The server asks US to resend / report state. This
                        // client keeps no outbound journal; ack (done above)
                        // and hope the server re-asks or recovers.
                        crate::serialize::MSGS_RESEND_REQ | crate::serialize::MSGS_STATE_REQ => {
                            tracing::warn!(
                                "server sent a resend/state request; no outbound journal, ignoring"
                            );
                        }
                        // Everything below is informational and never the
                        // RPC answer: acks/pongs, MTProto service info
                        // about other messages' fate, and server-initiated
                        // updates pushes that precede our answer on the
                        // same connection. Drain (the frame is acked
                        // above) and keep waiting for rpc_result.
                        MSGS_ACK
                        | PONG
                        | crate::serialize::MSG_DETAILED_INFO
                        | crate::serialize::MSG_NEW_DETAILED_INFO
                        | crate::types::UPDATES
                        | crate::types::UPDATES_COMBINED
                        | crate::types::UPDATE_SHORT
                        | crate::types::UPDATE_SHORT_SENT_MESSAGE => {}
                        RPC_RESULT => {
                            // rpc_result#f35c6d01 req_msg_id:long result
                            let req = u64::from_le_bytes(
                                // invariant: RPC_RESULT matched, so the item
                                // carries its full 12-byte header
                                #[allow(clippy::unwrap_used)]
                                item[4..12].try_into().unwrap(),
                            );
                            if req == msg_id {
                                return Self::parse_rpc_result_body(item);
                            }
                            // Stale answer for an earlier msg_id — drain.
                        }
                        other => {
                            // Not an rpc_result — handing this back as the
                            // answer produced "expected messages.Messages*,
                            // got 0x…" upstream. Drain and keep reading;
                            // the frame bound below ends a hopeless wait.
                            tracing::debug!(
                                "draining non-answer item {other:#010x} \
                                 while waiting for msg {msg_id}"
                            );
                        }
                    }
                }
                if re_send {
                    if attempt < 3 {
                        continue 'outer;
                    }
                    return Err(Error::Protocol(
                        "server session state did not settle".into(),
                    ));
                }
                if conclusive || frames >= 16 {
                    return Err(Error::Protocol(format!(
                        "no rpc_result for msg {msg_id} after {frames} frame(s)"
                    )));
                }
                frames += 1;
                // Our answer is still in flight — read the next frame on
                // the SAME connection without re-sending (bounded).
                response = {
                    let mut codec = conn.codec.lock().await;
                    match tokio::time::timeout(READ_TIMEOUT, codec.recv_frame()).await {
                        Ok(r) => r?,
                        Err(_) => {
                            return Err(Error::Protocol(format!(
                                "no rpc_result for msg {msg_id} after {frames} frame(s) — read timed out"
                            )));
                        }
                    }
                };
            }
        }
        unreachable!("retry loop returns on every path")
    }

    /// Adopt server-announced session state from a service message.
    /// Returns `true` when the caller should re-send its request.
    async fn adopt_service_state(&self, ctor: u32, item: &[u8]) -> bool {
        use crate::serialize::{BAD_SERVER_SALT, NEW_SERVER_SALT, NEW_SESSION_CREATED};
        if item.len() < 28 {
            return false;
        }
        let new_salt = match ctor {
            // bad_server_salt#edab447b ... new_salt at [20..28]
            // new_session_created#9ec20908 first_msg_id:long
            // unique_id:long server_salt:long — salt at [20..28] in both
            BAD_SERVER_SALT | NEW_SESSION_CREATED => u64::from_le_bytes(
                // invariant: the `item.len() < 28` guard above guarantees
                // this full 8-byte field
                #[allow(clippy::unwrap_used)]
                item[20..28].try_into().unwrap(),
            ),
            // new_server_salt#1160b89c new_server_salt:long — salt at [4..12]
            NEW_SERVER_SALT => u64::from_le_bytes(
                // invariant: the `item.len() < 28` guard above guarantees
                // this full 8-byte field
                #[allow(clippy::unwrap_used)]
                item[4..12].try_into().unwrap(),
            ),
            _ => return false,
        };
        let mut session = self.session.write().await;
        if session.server_salt != new_salt {
            tracing::debug!(ctor = ctor, "adopting server-announced salt");
            session.server_salt = new_salt;
        }
        true
    }

    /// Split a frame body into its messages: one item for a bare
    /// message, all items for a `msg_container`.
    fn container_items(data: &[u8]) -> Result<Vec<&[u8]>> {
        use crate::serialize::MSG_CONTAINER;

        if data.len() < 4
            || u32::from_le_bytes(
                // invariant: the `data.len() < 4` guard above guarantees
                // this full 4-byte constructor
                #[allow(clippy::unwrap_used)]
                data[0..4].try_into().unwrap(),
            ) != MSG_CONTAINER
        {
            return Ok(vec![data]);
        }
        let mut r = crate::serialize::TLReader::new(data);
        let _ctor = r.read_u32()?;
        let count = r.read_i32()?;
        let mut off = 8usize;
        let mut items = Vec::with_capacity(count.max(0) as usize);
        for _ in 0..count.max(0) {
            if off + 12 > data.len() {
                break;
            }
            off += 12; // msg_id:long seq_no:int
            if off + 4 > data.len() {
                break;
            }
            let len = i32::from_le_bytes(
                // invariant: the `off + 4 > data.len()` guard above
                // guarantees this full 4-byte length field
                #[allow(clippy::unwrap_used)]
                data[off..off + 4].try_into().unwrap(),
            ) as usize;
            off += 4;
            if off + len > data.len() {
                break;
            }
            items.push(&data[off..off + len]);
            off += (len + 3) & !3;
        }
        Ok(items)
    }

    /// Pick the meaningful message out of a `msg_container`: the
    /// `rpc_result` item if present, else the first non-service item.
    /// Service noise (`msgs_ack`, `pong`, `new_session_created`) is skipped.
    fn choose_container_item(data: &[u8]) -> Result<Option<&[u8]>> {
        use crate::serialize::{MSG_CONTAINER, MSGS_ACK, NEW_SESSION_CREATED, PONG};

        if data.len() < 4
            || u32::from_le_bytes(
                // invariant: the `data.len() < 4` guard above guarantees
                // this full 4-byte constructor
                #[allow(clippy::unwrap_used)]
                data[0..4].try_into().unwrap(),
            ) != MSG_CONTAINER
        {
            return Ok(Some(data));
        }
        let mut r = crate::serialize::TLReader::new(data);
        let _ctor = r.read_u32()?;
        let count = r.read_i32()?;
        let bytes = data;
        let mut off = 8usize;
        let mut first: Option<&[u8]> = None;
        let mut rpc: Option<&[u8]> = None;
        for _ in 0..count.max(0) {
            if off + 12 > bytes.len() {
                break;
            }
            off += 12; // msg_id:long seq_no:int
            if off + 4 > bytes.len() {
                break;
            }
            let len = i32::from_le_bytes(
                // invariant: the `off + 4 > bytes.len()` guard above
                // guarantees this full 4-byte length field
                #[allow(clippy::unwrap_used)]
                bytes[off..off + 4].try_into().unwrap(),
            ) as usize;
            off += 4;
            if off + len > bytes.len() {
                break;
            }
            let item = &bytes[off..off + len];
            let item_ctor = u32::from_le_bytes(item[0..4].try_into().unwrap_or([0; 4]));
            if item_ctor == crate::serialize::RPC_RESULT && rpc.is_none() {
                rpc = Some(item);
            }
            if first.is_none()
                && item_ctor != MSGS_ACK
                && item_ctor != PONG
                && item_ctor != NEW_SESSION_CREATED
                && item_ctor != crate::serialize::NEW_SERVER_SALT
            {
                first = Some(item);
            }
            off += (len + 3) & !3; // 4-byte aligned
        }
        Ok(rpc.or(first))
    }

    /// Parse an `rpc_result#f35c6d01` wrapper: request `msg_id`, gzip, and
    /// `rpc_error` classification. `data` starts at the `rpc_result` ctor.
    fn parse_rpc_result_body(data: &[u8]) -> Result<Vec<u8>> {
        use crate::serialize::{
            RPC_ANSWER_DROPPED, RPC_ANSWER_DROPPED_RUNNING, RPC_ANSWER_UNKNOWN, RPC_ERROR,
            RPC_RESULT, TLReader,
        };

        let mut r = TLReader::new(data);
        let ctor = r.read_u32()?;
        if ctor != RPC_RESULT {
            return Err(Error::Protocol(format!(
                "expected rpc_result, got {ctor:#x}"
            )));
        }
        let _req_msg_id = r.read_u64()?;
        let inner = data[r.position()..].to_vec();
        // rpc_result body may itself be gzipped
        let inner = Self::unwrap_gzip(&inner)?;
        // rpc_error is delivered INSIDE rpc_result
        if inner.len() < 4 {
            return Err(Error::Protocol(format!(
                "rpc_result body too short: {} bytes",
                inner.len()
            )));
        }
        let inner_ctor = u32::from_le_bytes(
            // invariant: the `inner.len() < 4` guard above guarantees
            // this full 4-byte constructor
            #[allow(clippy::unwrap_used)]
            inner[..4].try_into().unwrap(),
        );
        if inner_ctor == RPC_ERROR {
            let (code, msg) = crate::mtproto::parse_rpc_error(&inner)?;
            return Err(crate::error::classify_rpc_error(code, &msg));
        }
        // rpc_answer_* reply kinds (SPEC §5.2): the answer is unknown,
        // still being computed, or was dropped — the caller should retry
        // with backoff. Only rpc_answer_dropped_running is definitive
        // enough to surface directly; the others map to the same error.
        if matches!(
            inner_ctor,
            RPC_ANSWER_UNKNOWN | RPC_ANSWER_DROPPED | RPC_ANSWER_DROPPED_RUNNING
        ) {
            return Err(Error::RpcDropped {
                detail: format!("rpc_result contained rpc_answer ctor {inner_ctor:#x}"),
            });
        }
        // A bad_msg_notification can also arrive inside rpc_result.
        if inner_ctor == crate::serialize::BAD_MSG_NOTIFICATION {
            let (bad_msg_id, _seqno, code) = crate::mtproto::parse_bad_msg_notification(&inner)?;
            return Err(classify_bad_msg(code, bad_msg_id));
        }
        Ok(inner)
    }

    /// Detect `gzip_packed` (possibly nested) and return decompressed bytes.
    fn unwrap_gzip(data: &[u8]) -> Result<Vec<u8>> {
        use crate::serialize::{GZIP_PACKED, TLReader};

        if data.len() < 4 {
            return Ok(data.to_vec());
        }
        let ctor = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if ctor != GZIP_PACKED {
            return Ok(data.to_vec());
        }

        let mut r = TLReader::new(data);
        r.read_u32()?;
        let packed = r.read_bytes()?;
        let mut decoder = flate2::read::GzDecoder::new(&packed[..]);
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut decoder, &mut out)
            .map_err(|e| Error::Serialization(format!("gzip decompress: {e}")))?;
        Ok(out)
    }

    /// `unwrap_gzip` for one container item — the compressed payload
    /// replaces the `gzip_packed` item in place. Bounded recursion for
    /// the (hypothetical) nested-wrap case.
    fn unwrap_item_gzip(item: &[u8]) -> Result<Vec<u8>> {
        const MAX_NESTING: usize = 4;
        let mut cur = item.to_vec();
        for _ in 0..MAX_NESTING {
            if cur.len() < 4 {
                return Ok(cur);
            }
            let ctor = u32::from_le_bytes(
                // invariant: the len guard above guarantees 4 bytes
                #[allow(clippy::unwrap_used)]
                cur[0..4].try_into().unwrap(),
            );
            if ctor != crate::serialize::GZIP_PACKED {
                return Ok(cur);
            }
            cur = Self::unwrap_gzip(&cur)?;
        }
        Err(Error::Protocol(
            "gzip_packed nested more than 4 deep".into(),
        ))
    }

    /// Ask the server for its future salt windows
    /// (`getFutureSalts#b921bd04 num:int` → `future_salts#ae500895`).
    ///
    /// Returns `(req_msg_id, server_now, windows)`. Useful for pre-warming
    /// salts around clock-boundary reconnects; the pool otherwise keeps its
    /// salt fresh via `bad_server_salt` / `new_server_salt` handling.
    ///
    /// # Errors
    /// Returns any failure from the salt-request round-trip: transport,
    /// decryption, decompression, or response parsing.
    pub async fn get_future_salts(
        &self,
        num: i32,
    ) -> Result<(u64, i32, Vec<crate::mtproto::SaltWindow>)> {
        // getFutureSalts is a BARE service message, not an RPC method —
        // wrapping it in invokeWithLayer yields INPUT_METHOD_INVALID
        // (the ctor the server reports back is getFutureSalts itself).
        let req = crate::mtproto::build_get_future_salts(num);
        let (_msg_id, response) = self.send_encrypted(&req).await?;
        let plaintext = {
            let mut session = self.session.write().await;
            session.decrypt_message(&response)?.1
        };
        let body = Self::unwrap_gzip(&plaintext)?;
        let body = Self::choose_container_item(&body)?.map_or_else(|| body.clone(), <[u8]>::to_vec);
        crate::mtproto::parse_future_salts(&body)
    }

    /// Ask the server for the delivery state of the given messages
    /// (`msgs_state_req#da69fb52` → `msgs_state_info#04deb57d`).
    ///
    /// Returns the raw `info` byte string (one status byte per requested
    /// `msg_id`, bit 2 = message is known to the server).
    ///
    /// # Errors
    /// Returns an error if the request round-trip fails or the server
    /// answers with something other than `msgs_state_info`.
    pub async fn query_msgs_state(&self, msg_ids: &[u64]) -> Result<Vec<u8>> {
        let req = crate::mtproto::build_msgs_state_req(msg_ids);
        let payload = self.send_rpc(&req).await?;
        let mut r = crate::serialize::TLReader::new(&payload);
        let ctor = r.read_u32()?;
        if ctor != crate::serialize::MSGS_STATE_INFO {
            return Err(Error::Protocol(format!(
                "expected msgs_state_info, got {ctor:#x}"
            )));
        }
        let _req_msg_id = r.read_u64()?;
        r.read_bytes()
    }

    /// Queue an ack for the given received `msg_id` (SPEC §5.4 batching:
    /// flush immediately at [`ProtocolConfig::ack_batch_max`] pending,
    /// otherwise wait for the flusher task). Best-effort — flush errors are
    /// logged, not fatal.
    async fn queue_ack(&self, resp_msg_id: u64) {
        let flush = {
            let mut pending = self.pending_acks.lock().await;
            pending.push(resp_msg_id);
            pending.len() >= self.protocol.ack_batch_max
        };
        if flush {
            self.flush_acks().await;
        }
    }

    /// Write one batched `msgs_ack` for every queued `msg_id` (write-only —
    /// no reply expected).
    async fn flush_acks(&self) {
        let ids: Vec<u64> = {
            let mut pending = self.pending_acks.lock().await;
            std::mem::take(&mut *pending)
        };
        if ids.is_empty() {
            return;
        }
        let ack = crate::mtproto::build_msgs_ack(&ids);
        let encrypted = {
            let mut session = self.session.write().await;
            let ack_msg_id = session.next_msg_id();
            let ack_seq_no = session.next_seq_no(false);
            session.encrypt_message(&ack, ack_msg_id, ack_seq_no)
        };
        if let Err(e) = self.write_raw(&encrypted).await {
            tracing::debug!("ack write failed (non-fatal): {e}");
        }
    }

    /// Spawn the periodic ack flusher (every
    /// [`ProtocolConfig::ack_flush_interval`]). Call once after `connect`.
    #[allow(clippy::unused_async)] // uniform spawned-task signature with the other pool loops; no awaits today
    pub fn spawn_ack_flusher(self: &Arc<Self>) {
        let protocol = self.protocol.clone();
        let pool_arc = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(protocol.ack_flush_interval);
            loop {
                tick.tick().await;
                pool_arc.flush_acks().await;
            }
        });
    }

    /// Spawn the ping/pong keepalive: pings every idle connection on
    /// [`ProtocolConfig::ping_interval`]; a connection that goes
    /// [`ProtocolConfig::pong_timeout`] without
    /// a pong is silently disconnected and reconnected with the same
    /// `auth_key` (SPEC BS-1).
    pub fn spawn_keepalive(self: &Arc<Self>) {
        let protocol = self.protocol.clone();
        let pool_arc = self.clone();
        for i in 0..pool_arc.connections.len() {
            let conn = pool_arc.connections[i].clone();
            let pool_arc = pool_arc.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(protocol.ping_interval).await;
                    // Only ping when the codec is idle; a locked codec means
                    // an RPC exchange is in flight and traffic itself proves
                    // liveness.
                    let Ok(mut codec) = conn.codec.try_lock() else {
                        continue;
                    };
                    let ping_id = rand::random::<i64>();
                    // gotd parity: ping_delay_disconnect makes the server reap the
                    // connection itself if we stop pinging (delay = interval
                    // + timeout, like gotd's pingLoop).
                    let delay =
                        (protocol.ping_interval.as_secs() + protocol.pong_timeout.as_secs()) as i32;
                    let ping = crate::mtproto::build_ping_delay_disconnect(ping_id, delay);
                    let encrypted = {
                        let mut session = pool_arc.session.write().await;
                        let msg_id = session.next_msg_id();
                        let seq_no = session.next_seq_no(true);
                        session.encrypt_message(&ping, msg_id, seq_no)
                    };
                    if let Err(e) = codec.send_frame(&encrypted).await {
                        tracing::debug!("keepalive ping failed: {e}");
                        continue;
                    }
                    match tokio::time::timeout(protocol.pong_timeout, codec.recv_frame()).await {
                        Ok(Ok(resp)) => {
                            let decrypted = pool_arc.session.write().await.decrypt_message(&resp);
                            if let Ok((_, plaintext)) = decrypted {
                                let body = Self::unwrap_gzip(&plaintext)
                                    .unwrap_or_else(|_| plaintext.clone());
                                match crate::mtproto::parse_pong(&body) {
                                    Ok(_) => {}
                                    Err(_) => {
                                        tracing::debug!("keepalive got non-pong frame; discarded");
                                    }
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            tracing::warn!(
                                "keepalive recv failed on DC {} conn {i}: {e}",
                                pool_arc.dc_id
                            );
                            if let Err(e) = pool_arc.reconnect_connection(&conn).await {
                                tracing::warn!("keepalive reconnect failed: {e}");
                                break;
                            }
                        }
                        Err(_) => {
                            tracing::warn!(
                                "no pong within {:?} on DC {} conn {i} — reconnecting",
                                protocol.pong_timeout,
                                pool_arc.dc_id
                            );
                            if let Err(e) = pool_arc.reconnect_connection(&conn).await {
                                tracing::warn!("keepalive reconnect failed: {e}");
                                break;
                            }
                        }
                    }
                }
            });
        }
    }

    /// Spawn the periodic salt refresher: every
    /// [`ProtocolConfig::salt_refresh_interval`]
    /// ask for future salt windows and adopt the one currently valid
    /// (SPEC §9: salt validity ~30 min).
    pub fn spawn_salt_refresher(self: &Arc<Self>) {
        let protocol = self.protocol.clone();
        let pool_arc = self.clone();
        tokio::spawn(async move {
            // Sleep FIRST: a tokio interval fires its first tick
            // immediately, which would spam getFutureSalts at startup.
            loop {
                tokio::time::sleep(protocol.salt_refresh_interval).await;
                match pool_arc.get_future_salts(3).await {
                    Ok((_req_id, server_now, windows)) => {
                        let fresh = windows
                            .iter()
                            .find(|w| w.valid_since <= server_now && server_now < w.valid_until);
                        if let Some(w) = fresh {
                            let mut session = pool_arc.session.write().await;
                            if session.server_salt != w.salt {
                                tracing::debug!("salt refreshed via get_future_salts");
                                session.server_salt = w.salt;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!("salt refresh failed (non-fatal): {e}");
                    }
                }
            }
        });
    }

    /// Write one obfuscated frame to the next round-robin connection
    /// WITHOUT reading a response. Used for acks, which get no reply.
    async fn write_raw(&self, data: &[u8]) -> Result<()> {
        if self.connections.is_empty() {
            return Err(Error::Transport("pool has no connections".into()));
        }

        let idx = {
            let mut ni = self.next_index.lock().await;
            let idx = *ni;
            *ni = (*ni + 1) % self.connections.len();
            idx
        };
        let conn = &self.connections[idx];
        let mut codec = conn.codec.lock().await;
        codec.send_frame(data).await
    }

    /// Open one connection honouring `transport_policy`.
    ///
    /// `TcpOnly`: plain `connect_obfuscated2`. `Auto`: TCP first; when the
    /// failure tracker says the DC's TCP path is blocked (2 failures within
    /// 5 min) new connects go straight to `wss://` — until a TCP attempt
    /// succeeds again, which resets the tracker (SPEC BS-6). `prefer_ws`
    /// reflects the tracker snapshot taken before the attempt; success and
    /// failure are reported back through [`ConnectOutcome`] so the caller
    /// (which owns the tracker) can update it.
    #[allow(unused_variables)]
    async fn connect_one(
        dc_id: i32,
        policy: TransportPolicy,
        prefer_ws: bool,
    ) -> (ConnectOutcome, Result<PoolCodec>) {
        #[cfg(not(feature = "ws"))]
        let ws_requested = false;
        #[cfg(feature = "ws")]
        let ws_requested = matches!(policy, TransportPolicy::Auto) && prefer_ws;
        #[cfg(feature = "ws")]
        if ws_requested {
            match Self::connect_ws(dc_id).await {
                Ok(c) => return (ConnectOutcome::Ws, Ok(PoolCodec::Ws(c))),
                Err(ws_err) => {
                    tracing::warn!("ws connect to DC {dc_id} failed, trying tcp: {ws_err}");
                }
            }
        }
        match Self::connect_tcp(dc_id).await {
            Ok(c) => (ConnectOutcome::TcpOk, Ok(PoolCodec::Tcp(c))),
            Err(tcp_err) => {
                // TCP tried (and failed) — record it; on the NEXT connect
                // the tracker will prefer ws.
                #[cfg(feature = "ws")]
                if !ws_requested && matches!(policy, TransportPolicy::Auto) {
                    match Self::connect_ws(dc_id).await {
                        Ok(c) => {
                            return (ConnectOutcome::TcpFailThenWs, Ok(PoolCodec::Ws(c)));
                        }
                        Err(ws_err) => {
                            tracing::warn!("ws fallback to DC {dc_id} failed: {ws_err}");
                        }
                    }
                }
                let _ = (&ws_requested, &policy);
                (ConnectOutcome::TcpFail, Err(tcp_err))
            }
        }
    }

    async fn connect_tcp(dc_id: i32) -> Result<transport::IntermediateTransport> {
        Ok(transport::IntermediateTransport::new(
            transport::connect(dc_id).await?,
        ))
    }

    #[cfg(feature = "ws")]
    async fn connect_ws(
        dc_id: i32,
    ) -> Result<transport::Obfuscated2Transport<crate::ws::WsTransport>> {
        crate::ws::connect_obfuscated2_ws(dc_id, transport::TransportProtocol::Intermediate).await
    }

    /// Stub for no-`ws` builds: keeps `connect_one` uniform. Never called
    /// because all WS branches are cfg-gated off.
    #[cfg(not(feature = "ws"))]
    #[allow(dead_code)]
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)] // cfg-mirrors the real `connect_ws` so call sites stay uniform; stub never awaits by design
    async fn connect_ws(dc_id: i32) -> Result<transport::Obfuscated2Transport> {
        let _ = dc_id;
        Err(Error::Transport(
            "WebSocket fallback requested but the `ws` feature is not enabled".into(),
        ))
    }

    async fn reconnect_connection(&self, conn: &PooledConnection) -> Result<()> {
        let mut attempts = 0u32;
        loop {
            if attempts >= self.config.max_reconnect_attempts {
                return Err(Error::Transport(format!(
                    "failed to reconnect after {} attempts",
                    self.config.max_reconnect_attempts
                )));
            }

            let base_delay = self.config.reconnect_base_delay_ms;
            let exp = attempts.min(10); // cap to prevent overflow
            let delay_ms = base_delay.saturating_mul(1u64 << exp);
            let jitter = rand::random::<u64>() % (delay_ms / 4 + 1);
            let delay = std::time::Duration::from_millis((delay_ms + jitter).min(60_000));

            tokio::time::sleep(delay).await;

            let prefer_ws = self.tcp_failover.lock().await.should_prefer_ws();
            let (outcome, res) =
                Self::connect_one(self.dc_id, self.config.transport_policy, prefer_ws).await;
            self.update_failover(outcome).await;
            match res {
                Ok(codec) => {
                    // Guard drops on return — the scoped block IS the
                    // last use (clippy cannot see through the return).
                    #[allow(clippy::significant_drop_tightening)]
                    {
                        let mut locked = conn.codec.lock().await;
                        *locked = codec;
                    }
                    return Ok(());
                }
                Err(e) => {
                    attempts += 1;
                    tracing::warn!(
                        "reconnect to DC {} failed (attempt {}): {}",
                        self.dc_id,
                        attempts,
                        e
                    );
                }
            }
        }
    }

    /// Reconnect the connection at `conn_index`.
    ///
    /// # Errors
    /// Returns a transport error when `conn_index` is out of range or the
    /// reconnection attempts are exhausted.
    pub async fn reconnect(&self, conn_index: usize) -> Result<()> {
        if conn_index >= self.connections.len() {
            return Err(Error::Transport("invalid connection index".into()));
        }
        self.reconnect_connection(&self.connections[conn_index])
            .await
    }

    /// Get the number of active connections.
    #[must_use]
    pub const fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Get the DC ID.
    #[must_use]
    pub const fn dc_id(&self) -> i32 {
        self.dc_id
    }

    /// Scale up by adding an aux connection. Requires `&mut self` so the
    /// new connection is actually inserted into the pool.
    ///
    /// # Errors
    /// Returns a transport error when opening the new connection fails.
    pub async fn scale_up(&mut self) -> Result<()> {
        if self.connections.len() >= self.config.max_connections {
            return Ok(());
        }

        let prefer_ws = self.tcp_failover.lock().await.should_prefer_ws();
        let (outcome, res) =
            Self::connect_one(self.dc_id, self.config.transport_policy, prefer_ws).await;
        self.update_failover(outcome).await;
        let codec = res?;
        self.connections.push(Arc::new(PooledConnection {
            codec: Mutex::new(codec),
            rpc_permit: Mutex::new(()),
        }));
        tracing::info!(
            "scaled up: now {} connection(s) to DC {}",
            self.connections.len(),
            self.dc_id
        );
        Ok(())
    }
}

/// Join a list of task handles, flattening a cancelled task
/// ([`tokio::task::JoinError`]) into a transport error so callers can treat
/// every slot uniformly.
async fn futures_collect<T>(handles: Vec<tokio::task::JoinHandle<Result<T>>>) -> Vec<Result<T>> {
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        out.push(h.await.unwrap_or_else(|e| {
            Err(crate::error::Error::Transport(format!(
                "task join failed: {e}"
            )))
        }));
    }
    out
}

/// Human-readable meaning of a `bad_msg_notification` `error_code`
/// (SPEC §5.2).
///
/// Code 20 (salt invalidated) cannot be auto-recovered here because the
/// notification carries no new salt — the caller re-encrypts with the
/// adopted salt on its next request.
#[must_use]
pub const fn describe_bad_msg_code(code: i32) -> &'static str {
    match code {
        16 => "msg_id too low",
        17 => "msg_id too high",
        18 => "incorrect two lower order msg_id bits",
        20 => "message too old",
        19 => "container msg_id identical to a previously received one",
        32 => "msg_seqno too low",
        33 => "msg_seqno too high",
        34 => "even msg_seqno expected, but odd received",
        35 => "odd msg_seqno expected, but even received",
        48 => "incorrect server salt",
        64 => "invalid container",
        65 => "message not authorised (no auth_key)",
        _ => "unknown bad_msg code",
    }
}

/// Map a `bad_msg_notification` to a typed [`Error`].
fn classify_bad_msg(code: i32, bad_msg_id: u64) -> Error {
    tracing::warn!(
        "bad_msg_notification for msg {bad_msg_id}: code {code} ({})",
        describe_bad_msg_code(code)
    );
    match code {
        // Code 65 means the auth key never reached this server — treat it
        // like an auth failure so callers can re-authenticate.
        65 => Error::NoAuthKey,
        _ => Error::BadMessage {
            code,
            description: describe_bad_msg_code(code).to_string(),
        },
    }
}

#[cfg(test)]
mod bad_msg_tests {
    // Test code: unwrap is the idiomatic failure mode here.
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_describe_bad_msg_codes() {
        assert_eq!(describe_bad_msg_code(16), "msg_id too low");
        assert_eq!(
            describe_bad_msg_code(18),
            "incorrect two lower order msg_id bits"
        );
        assert_eq!(describe_bad_msg_code(20), "message too old");
        assert_eq!(describe_bad_msg_code(48), "incorrect server salt");
        assert_eq!(
            describe_bad_msg_code(65),
            "message not authorised (no auth_key)"
        );
        assert!(describe_bad_msg_code(42).starts_with("unknown"));
    }

    #[test]
    fn test_parse_bad_msg_notification() {
        let mut w = crate::serialize::TLWriter::new();
        w.write_u32(crate::serialize::BAD_MSG_NOTIFICATION);
        w.write_u64(0x1234);
        w.write_i32(5);
        w.write_i32(16);
        let (id, seqno, code) = crate::mtproto::parse_bad_msg_notification(w.as_bytes()).unwrap();
        assert_eq!((id, seqno, code), (0x1234, 5, 16));
    }

    #[test]
    fn test_choose_container_item_prefers_rpc_result() {
        use crate::serialize::TLWriter;
        // Build msgs_ack and rpc_result bodies, wrap in a container.
        let mut ack = TLWriter::new();
        ack.write_u32(crate::serialize::MSGS_ACK);
        ack.write_u32(crate::serialize::VECTOR);
        ack.write_i32(1);
        ack.write_u64(0x1234);
        let ack = ack.into_bytes();

        let mut rpc = TLWriter::new();
        rpc.write_u32(crate::serialize::RPC_RESULT);
        rpc.write_u64(0xdeadbeef);
        rpc.write_raw_bytes(&[0x99, 0x72, 0x75, 0xb5]); // boolTrue body
        let rpc = rpc.into_bytes();

        let mut c = TLWriter::new();
        c.write_u32(crate::serialize::MSG_CONTAINER);
        c.write_i32(2);
        for (id, seq, body) in [(0x11, 1, &ack), (0x22, 3, &rpc)] {
            c.write_u64(id);
            c.write_i32(seq);
            c.write_i32(body.len() as i32);
            let pad = (4 - (body.len() % 4)) % 4;
            c.write_raw_bytes(body);
            if pad > 0 {
                c.write_raw_bytes(&[0u8; 3][..pad]);
            }
        }
        let container = c.into_bytes();

        let chosen = SenderPool::choose_container_item(&container)
            .unwrap()
            .unwrap();
        assert_eq!(
            u32::from_le_bytes(chosen[0..4].try_into().unwrap()),
            crate::serialize::RPC_RESULT
        );
    }

    #[test]
    fn test_classify_bad_msg_code_65_is_no_auth_key() {
        assert!(matches!(classify_bad_msg(65, 1), Error::NoAuthKey));
        let e = classify_bad_msg(16, 1);
        assert!(matches!(e, Error::BadMessage { code: 16, .. }));
        assert!(!e.is_transient());
    }

    #[test]
    fn test_rpc_dropped_is_transient() {
        assert!(Error::RpcDropped { detail: "x".into() }.is_transient());
    }
}

#[cfg(test)]
mod tests {
    // Test code: unwrap is the idiomatic failure mode here.
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_pool_config_defaults() {
        let config = PoolConfig::default();
        assert_eq!(config.min_connections, 4);
        assert_eq!(config.max_connections, 8);
        assert_eq!(config.keepalive_secs, 30);
        assert_eq!(config.reconnect_base_delay_ms, 1000);
    }

    #[test]
    fn test_pool_creation() {
        let session = MtProtoSession::new(vec![0u8; 256], 12345);
        let pool = SenderPool::new(
            2,
            0,
            session,
            PoolConfig::default(),
            ProtocolConfig::default(),
        );
        assert_eq!(pool.dc_id(), 2);
    }

    #[test]
    fn test_pool_without_session() {
        let pool = SenderPool::without_session(2, PoolConfig::default());
        assert_eq!(pool.dc_id(), 2);
    }

    #[test]
    fn test_failover_tracker_threshold_and_reset() {
        let mut t = TcpFailover::default();
        assert!(!t.should_prefer_ws());
        // One failure: below threshold (2 within 5 min).
        t.record_failure();
        assert!(!t.should_prefer_ws());
        // Second failure within window: tripped.
        t.record_failure();
        assert!(t.should_prefer_ws());
        // A TCP success resets — next connects try TCP again (SPEC BS-6).
        t.record_success();
        assert!(!t.should_prefer_ws());
    }

    #[test]
    fn test_transport_policy_default_is_tcp_only() {
        assert_eq!(
            PoolConfig::default().transport_policy,
            TransportPolicy::TcpOnly
        );
        assert_eq!(TransportPolicy::default(), TransportPolicy::TcpOnly);
    }
}

#[cfg(test)]
mod envelope_debug_tests {
    // Test code: unwrap is the idiomatic failure mode here.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    /// Decode the full RPC envelope field-by-field per the layer 223
    /// schema to catch layout drift without hitting the network.
    #[test]
    fn test_full_envelope_layout() {
        let resolve = crate::rpc::build_resolve_username("lebenoa");
        let full = crate::mtproto::build_invoke_with_layer(
            223,
            &crate::mtproto::build_init_connection(
                12345, "mtprsto", "unknown", "0.1.0", "en", &resolve,
            ),
        );
        let mut r = crate::serialize::TLReader::new(&full);
        assert_eq!(r.read_u32().unwrap(), crate::types::INVOKE_WITH_LAYER);
        assert_eq!(r.read_i32().unwrap(), 223);
        assert_eq!(r.read_u32().unwrap(), crate::types::INIT_CONNECTION);
        assert_eq!(r.read_i32().unwrap(), 0); // flags
        assert_eq!(r.read_i32().unwrap(), 12345); // api_id
        assert_eq!(
            String::from_utf8(r.read_bytes().unwrap()).unwrap(),
            "mtprsto"
        );
        assert_eq!(
            String::from_utf8(r.read_bytes().unwrap()).unwrap(),
            "unknown"
        );
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "0.1.0");
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "en"); // system_lang_code
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), ""); // lang_pack
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "en"); // lang_code
        assert_eq!(
            r.read_u32().unwrap(),
            crate::types::CONTACTS_RESOLVE_USERNAME
        );
        assert_eq!(r.read_i32().unwrap(), 0); // flags (no referer)
        assert_eq!(
            String::from_utf8(r.read_bytes().unwrap()).unwrap(),
            "lebenoa"
        );
        assert_eq!(r.position(), full.len(), "trailing bytes in envelope");
    }
}
