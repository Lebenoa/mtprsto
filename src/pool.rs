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
//! Each Connection owns a tokio task. Requests are load-balanced
//! across available connections inside the pool.
//! ```

use crate::error::{Error, Result};
use crate::mtproto::MtProtoSession;
use crate::transport;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
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
            min_connections: 1,
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

/// A single connection to a Telegram DC.
struct Connection {
    stream: TcpStream,
    /// Whether this connection is the main (non-aux) connection.
    is_main: bool,
}

/// A pool of connections to a single DC.
pub struct SenderPool {
    /// DC ID this pool connects to.
    dc_id: i32,
    /// Shared session (auth key, salt, etc.).
    session: Arc<RwLock<MtProtoSession>>,
    /// Active connections.
    connections: Arc<Mutex<Vec<Connection>>>,
    /// Pool configuration.
    config: PoolConfig,
    /// Next connection index for round-robin.
    next_index: Arc<Mutex<usize>>,
}

impl SenderPool {
    /// Create a new pool for the given DC with an existing session.
    pub fn new(dc_id: i32, session: MtProtoSession, config: PoolConfig) -> Self {
        Self {
            dc_id,
            session: Arc::new(RwLock::new(session)),
            connections: Arc::new(Mutex::new(Vec::new())),
            config,
            next_index: Arc::new(Mutex::new(0)),
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

    /// Open the initial connection to the DC.
    pub async fn connect(&self) -> Result<()> {
        let mut conns = self.connections.lock().await;
        if !conns.is_empty() {
            return Ok(());
        }

        let stream = transport::connect(self.dc_id).await?;
        conns.push(Connection {
            stream,
            is_main: true,
        });

        // Spawn additional aux connections
        for _ in 1..self.config.min_connections.min(self.config.max_connections) {
            match transport::connect(self.dc_id).await {
                Ok(stream) => {
                    conns.push(Connection {
                        stream,
                        is_main: false,
                    });
                }
                Err(_) => break,
            }
        }

        Ok(())
    }

    /// Send raw bytes and receive a response using the next available connection (round-robin).
    pub async fn send_raw(&self, data: &[u8]) -> Result<Vec<u8>> {
        let conn_count = {
            let conns = self.connections.lock().await;
            conns.len()
        };

        if conn_count == 0 {
            self.connect().await?;
        }

        let idx = {
            let mut ni = self.next_index.lock().await;
            let idx = *ni;
            *ni = (*ni + 1) % conn_count.max(1);
            idx
        };

        let mut conns = self.connections.lock().await;
        if idx >= conns.len() {
            return Err(Error::Transport("no available connections in pool".into()));
        }

        let conn = &mut conns[idx];
        let len = (data.len() as u32).to_le_bytes();
        conn.stream.write_all(&len).await?;
        conn.stream.write_all(data).await?;
        conn.stream.flush().await?;

        // Receive response
        let mut len_buf = [0u8; 4];
        conn.stream.read_exact(&mut len_buf).await?;
        let resp_len = u32::from_le_bytes(len_buf) as usize;
        let mut resp = vec![0u8; resp_len];
        conn.stream.read_exact(&mut resp).await?;

        Ok(resp)
    }

    /// Send an encrypted message and receive the response.
    pub async fn send_encrypted(&self, payload: &[u8], msg_id: u64, seq_no: i32) -> Result<Vec<u8>> {
        let encrypted = {
            let session = self.session.read().await;
            session.encrypt_message(payload, msg_id, seq_no)
        };
        self.send_raw(&encrypted).await
    }

    /// Reconnect a failed connection with exponential backoff.
    pub async fn reconnect(&self, conn_index: usize) -> Result<()> {
        let mut attempts = 0u32;
        loop {
            if attempts >= self.config.max_reconnect_attempts {
                return Err(Error::Transport(format!(
                    "failed to reconnect after {} attempts",
                    self.config.max_reconnect_attempts
                )));
            }

            let delay = std::time::Duration::from_millis(
                self.config.reconnect_base_delay_ms * 2u64.pow(attempts),
            )
            .min(std::time::Duration::from_secs(60));

            tokio::time::sleep(delay).await;

            match transport::connect(self.dc_id).await {
                Ok(stream) => {
                    let mut conns = self.connections.lock().await;
                    if conn_index < conns.len() {
                        conns[conn_index].stream = stream;
                        return Ok(());
                    }
                    return Err(Error::Transport("invalid connection index".into()));
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

    /// Get the number of active connections.
    pub async fn connection_count(&self) -> usize {
        self.connections.lock().await.len()
    }

    /// Get the DC ID.
    pub fn dc_id(&self) -> i32 {
        self.dc_id
    }

    /// Scale up by adding an aux connection.
    pub async fn scale_up(&self) -> Result<()> {
        let mut conns = self.connections.lock().await;
        if conns.len() >= self.config.max_connections {
            return Ok(());
        }

        let stream = transport::connect(self.dc_id).await?;
        conns.push(Connection {
            stream,
            is_main: false,
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_config_defaults() {
        let config = PoolConfig::default();
        assert_eq!(config.min_connections, 1);
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
