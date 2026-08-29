//! Connection pool for MTProto sessions.
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

use crate::error::{Error, Result};
use crate::mtproto::MtProtoSession;
use crate::transport;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// Configuration for the connection pool.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Minimum number of connections (main + aux).
    pub min_connections: usize,
    /// Maximum number of connections.
    pub max_connections: usize,
    /// Threshold for scaling up: if inflight > 2 * aux_count for > 10s.
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
        }
    }
}

/// A single connection to a Telegram DC: an Obfuscated2 codec (CTR streams
/// plus `TcpStream`) behind its own mutex so I/O on one connection does not
/// block the others.
struct PooledConnection {
    codec: Mutex<transport::Obfuscated2Transport>,
}

/// A pool of connections to a single DC.
pub struct SenderPool {
    /// DC ID this pool connects to.
    dc_id: i32,
    /// Shared session (auth key, salt, etc.).
    session: Arc<RwLock<MtProtoSession>>,
    /// Active connections, each with its own mutex.
    connections: Vec<Arc<PooledConnection>>,
    /// Pool configuration.
    config: PoolConfig,
    /// Next connection index for round-robin.
    next_index: Mutex<usize>,
}

impl SenderPool {
    /// Create a new pool for the given DC with an existing session.
    pub fn new(dc_id: i32, session: MtProtoSession, config: PoolConfig) -> Self {
        Self {
            dc_id,
            session: Arc::new(RwLock::new(session)),
            connections: Vec::new(),
            config,
            next_index: Mutex::new(0),
        }
    }

    /// Create a pool with just a DC ID and config (no session yet).
    pub fn without_session(dc_id: i32, config: PoolConfig) -> Self {
        let session = MtProtoSession::new(vec![0u8; 256], 0);
        Self::new(dc_id, session, config)
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
    pub async fn connect(&mut self) -> Result<()> {
        if !self.connections.is_empty() {
            return Ok(());
        }

        let codec = transport::connect_obfuscated2(
            self.dc_id, transport::TransportProtocol::Intermediate,
        ).await?;
        self.connections.push(Arc::new(PooledConnection {
            codec: Mutex::new(codec),
        }));

        // Open additional aux connections in parallel — log failures but
        // don't abort. tokio::join! runs them concurrently; the first
        // (main) connection above is already in the pool.
        let aux_count = (self.config.min_connections.min(self.config.max_connections))
            .saturating_sub(1);
        let mut joins = Vec::with_capacity(aux_count);
        for _ in 0..aux_count {
            joins.push(transport::connect_obfuscated2(
                self.dc_id, transport::TransportProtocol::Intermediate,
            ));
        }
        // Spawn all aux connects concurrently and await them together
        // (tokio::spawn + JoinHandle::await — no futures crate needed).
        let handles: Vec<tokio::task::JoinHandle<_>> = joins
            .into_iter()
            .map(tokio::spawn)
            .collect();
        let results = futures_collect(handles).await;
        for (i, res) in results.into_iter().enumerate() {
            match res {
                Ok(codec) => {
                    self.connections.push(Arc::new(PooledConnection {
                        codec: Mutex::new(codec),
                    }));
                }
                Err(e) => {
                    tracing::warn!("aux connection {} to DC {} failed: {}", i + 1, self.dc_id, e);
                }
            }
        }

        Ok(())
    }

    /// Round-robin over connections. Each request/response pair is confined
    /// to a single connection (held under its mutex for the whole exchange),
    /// so a response can never be drained by a different request. NOTE: any
    /// server-initiated frame that arrives before the RPC result will be
    /// returned as if it were the result; updates are not yet routed to a
    /// separate queue, and the write-only ack is not msg_id-correlated.
    ///
    /// On I/O error the dead connection is reconnected transparently and the
    /// SAME encrypted payload is retried once. MTProto dedupes by msg_id
    /// (identical bytes ⇒ identical msg_id), so the server treats the retry
    /// as a retransmit, not a new request — safe even for non-idempotent
    /// methods.
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
                    idx, self.dc_id, e
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

    /// Send an encrypted message, allocate msg_id/seq_no atomically,
    /// and receive the response.
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

    /// High-level RPC: wraps method bytes in invokeWithLayer, encrypts,
    /// sends on one connection, decrypts the response, sends a write-only
    /// ack, unwraps gzip/rpc_result, classifies rpc_error, and returns the
    /// inner result bytes.
    pub async fn send_rpc(&self, method_bytes: &[u8]) -> Result<Vec<u8>> {
        use crate::serialize::{BAD_SERVER_SALT, RPC_ERROR, RPC_RESULT, TLReader, TLWriter};
        use crate::types::INVOKE_WITH_LAYER;

        // Build invokeWithLayer
        let mut w = TLWriter::new();
        w.write_u32(INVOKE_WITH_LAYER);
        w.write_i32(crate::api::API_LAYER);
        w.write_raw_bytes(method_bytes);
        let full_payload = w.into_bytes();

        // A bad_server_salt notification is retried transparently once with
        // the fresh salt (already adopted by decrypt_message).
        for attempt in 0..2u32 {
            let (_msg_id, response) = self.send_encrypted(&full_payload).await?;

            // Decrypt the response (also adopts the server's current salt)
            let (resp_msg_id, plaintext) = {
                let mut session = self.session.write().await;
                session.decrypt_message(&response)?
            };

            // Ack the response on the wire (write-only — no reply expected).
            self.send_write_only_ack(resp_msg_id).await;

            // Unwrap gzip_packed / rpc_result into the inner result bytes.
            let payload = Self::unwrap_gzip(&plaintext)?;
            let mut r = TLReader::new(&payload);
            let ctor = r.read_u32()?;

            if ctor == BAD_SERVER_SALT {
                // bad_server_salt#edab447b bad_msg_id:long bad_msg_seqno:int
                // error_code:int new_server_salt:long
                let _bad_msg_id = r.read_u64()?;
                let _bad_msg_seqno = r.read_i32()?;
                let _error_code = r.read_i32()?;
                let new_salt = r.read_u64()?;
                {
                    let mut session = self.session.write().await;
                    session.server_salt = new_salt;
                }
                if attempt == 0 {
                    continue; // re-send with the fresh salt
                }
                return Err(Error::Protocol("server salt out of sync after retry".into()));
            }

            if ctor == RPC_RESULT {
                let _req_msg_id = r.read_u64()?;
                let inner = payload[r.position()..].to_vec();
                // rpc_result body may itself be gzipped
                let inner = Self::unwrap_gzip(&inner)?;
                // rpc_error is delivered INSIDE rpc_result
                if inner.len() < 4 {
                    return Err(Error::Protocol(format!(
                        "rpc_result body too short: {} bytes", inner.len()
                    )));
                }
                let inner_ctor = u32::from_le_bytes(inner[..4].try_into().unwrap());
                if inner_ctor == RPC_ERROR {
                    let (code, msg) = crate::mtproto::parse_rpc_error(&inner)?;
                    return Err(crate::error::classify_rpc_error(code, &msg));
                }
                return Ok(inner);
            }

            return Ok(payload);
        }
        unreachable!("retry loop returns on every path")
    }

    /// Detect gzip_packed (possibly nested) and return decompressed bytes.
    fn unwrap_gzip(data: &[u8]) -> Result<Vec<u8>> {
        use crate::serialize::{TLReader, GZIP_PACKED};

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

    /// Fire-and-forget ack: encrypt and write to the round-robin connection
    /// without reading a reply. Best-effort — errors are logged, not fatal.
    async fn send_write_only_ack(&self, resp_msg_id: u64) {
        let ack = crate::mtproto::build_msgs_ack(&[resp_msg_id]);
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

    /// Reconnect a single connection with jittered exponential backoff.
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
            let delay =
                std::time::Duration::from_millis((delay_ms + jitter).min(60_000));

            tokio::time::sleep(delay).await;

            match transport::connect_obfuscated2(
                self.dc_id, transport::TransportProtocol::Intermediate,
            ).await {
                Ok(codec) => {
                    let mut locked = conn.codec.lock().await;
                    *locked = codec;
                    return Ok(());
                }
                Err(e) => {
                    attempts += 1;
                    tracing::warn!(
                        "reconnect to DC {} failed (attempt {}): {}",
                        self.dc_id, attempts, e
                    );
                }
            }
        }
    }

    /// Reconnect a connection at a given index.
    pub async fn reconnect(&self, conn_index: usize) -> Result<()> {
        if conn_index >= self.connections.len() {
            return Err(Error::Transport("invalid connection index".into()));
        }
        self.reconnect_connection(&self.connections[conn_index]).await
    }

    /// Get the number of active connections.
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Get the DC ID.
    pub fn dc_id(&self) -> i32 {
        self.dc_id
    }

    /// Scale up by adding an aux connection. Requires `&mut self` so the
    /// new connection is actually inserted into the pool.
    pub async fn scale_up(&mut self) -> Result<()> {
        if self.connections.len() >= self.config.max_connections {
            return Ok(());
        }

        let codec = transport::connect_obfuscated2(
            self.dc_id, transport::TransportProtocol::Intermediate,
        ).await?;
        self.connections.push(Arc::new(PooledConnection {
            codec: Mutex::new(codec),
        }));
        tracing::info!(
            "scaled up: now {} connection(s) to DC {}",
            self.connections.len(), self.dc_id
        );
        Ok(())
    }
}

/// Join a list of task handles, flattening a cancelled task (JoinError)
/// into a transport error so callers can treat every slot uniformly.
async fn futures_collect<T>(
    handles: Vec<tokio::task::JoinHandle<Result<T>>>,
) -> Vec<Result<T>> {
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        out.push(h.await.unwrap_or_else(|e| {
            Err(crate::error::Error::Transport(format!("task join failed: {e}")))
        }));
    }
    out
}

#[cfg(test)]
mod tests {
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
        let pool = SenderPool::new(2, session, PoolConfig::default());
        assert_eq!(pool.dc_id(), 2);
    }

    #[test]
    fn test_pool_without_session() {
        let pool = SenderPool::without_session(2, PoolConfig::default());
        assert_eq!(pool.dc_id(), 2);
    }
}
