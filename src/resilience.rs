//! Behind-the-scenes resilience (SPEC §12.2 BS-2/BS-3/BS-6):
//!
//! - [`FloodLimiter`]: per-method flood-wait bucket tracking with
//!   retry-after scheduling (BS-2).
//! - [`FileRefCache`]: bounded in-memory cache of `file_reference` blobs
//!   with expiry-driven invalidation (BS-3).
//! - [`DcRotator`]: background `help.getConfig` refresh and DC migration
//!   bookkeeping (BS-6).

use crate::error::Error;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

// ===========================================================================
// BS-2: flood-wait buckets
// ===========================================================================

/// Whether and how aggressively the client self-throttles.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Self-throttle enabled. Off by default (SPEC BS-2 is opt-in).
    pub enabled: bool,
    /// Default backoff applied after a FloodWait when the server did not
    /// give a retry duration.
    pub default_backoff: Duration,
    /// Maximum backoff the limiter will schedule.
    pub max_backoff: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
        }
    }
}

/// Per-method flood-wait bucket tracker.
///
/// Keyed by TL method id (the finest granularity the server's error
/// message exposes). Thread-safe: shared across pool workers.
#[derive(Debug, Default)]
pub struct FloodLimiter {
    inner: Mutex<HashMap<u32, Instant>>,
    config: RateLimitConfig,
}

impl FloodLimiter {
    /// Build a limiter from a rate-limit config.
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            config,
        }
    }

    /// True when self-throttling is enabled.
    pub fn enabled(&self) -> bool {
        self.config.enabled
    }

    /// Wait until `method` is allowed to run again (no-op when disabled or
    /// no flood recorded).
    pub async fn wait_for(&self, method: u32) {
        if !self.config.enabled {
            return;
        }
        let ready_at = {
            let map = self.inner.lock().await;
            map.get(&method).copied()
        };
        if let Some(ready_at) = ready_at {
            let now = Instant::now();
            if ready_at > now {
                tracing::debug!(
                    method = format!("{method:#x}"),
                    wait_ms = (ready_at - now).as_millis() as u64,
                    "flood-wait backoff"
                );
                tokio::time::sleep(ready_at - now).await;
            }
            let mut map = self.inner.lock().await;
            map.remove(&method);
        }
    }

    /// Record a FloodWait of `seconds` for `method`.
    pub async fn record_flood(&self, method: u32, seconds: i32) {
        let secs = seconds.clamp(0, 3600) as u64;
        let backoff = if secs == 0 {
            self.config.default_backoff
        } else {
            Duration::from_secs(secs)
        }
        .min(self.config.max_backoff);
        let mut map = self.inner.lock().await;
        map.insert(method, Instant::now() + backoff);
    }
}

// ===========================================================================
// BS-3: file-reference cache
// ===========================================================================

/// Cached `file_reference` blob with its origin message.
#[derive(Debug, Clone)]
pub struct FileRefEntry {
    pub reference: Vec<u8>,
    /// Message that carried the document/photo (for refresh).
    pub source: FileRefSource,
    /// Insertion time — file references go stale; entries older than
    /// `max_age` are treated as absent.
    pub inserted: Instant,
}

/// Where a file reference came from (needed to re-fetch it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileRefKey {
    /// Document or photo id.
    pub file_id: i64,
}

/// Origin descriptor for a file reference.
#[derive(Debug, Clone)]
pub enum FileRefSource {
    /// From a message in a chat (msg_id + peer id for re-fetch).
    Message { peer_id: i64, msg_id: i64 },
}

/// Bounded LRU-ish cache of file references (BS-3).
///
/// "LRU" in the sense of insertion-order eviction at capacity; the
/// freshness rule is `max_age` (server references typically live 1h+).
pub struct FileRefCache {
    entries: Mutex<HashMap<FileRefKey, FileRefEntry>>,
    capacity: usize,
    max_age: Duration,
}

impl FileRefCache {
    /// Cache with the default capacity (1024) and 1 h max age.
    pub fn new() -> Self {
        Self::with_limits(1024, Duration::from_secs(3600))
    }

    /// Cache with explicit capacity and max age.
    pub fn with_limits(capacity: usize, max_age: Duration) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            capacity,
            max_age,
        }
    }

    /// Look up a fresh reference for `key`.
    pub async fn get(&self, key: FileRefKey) -> Option<Vec<u8>> {
        let map = self.entries.lock().await;
        let entry = map.get(&key)?;
        if entry.inserted.elapsed() > self.max_age {
            return None; // stale
        }
        Some(entry.reference.clone())
    }

    /// Insert (or replace) a reference.
    pub async fn put(&self, key: FileRefKey, entry: FileRefEntry) {
        let mut map = self.entries.lock().await;
        if map.len() >= self.capacity {
            // Evict the oldest entry (linear scan is fine at this scale).
            if let Some(oldest_key) = map
                .iter()
                .min_by_key(|(_, e)| e.inserted)
                .map(|(k, _)| *k)
            {
                map.remove(&oldest_key);
            }
        }
        map.insert(key, entry);
    }

    /// Drop a stale entry (called after a `FILE_REFERENCE_EXPIRED` retry
    /// succeeded with a fresh blob).
    pub async fn invalidate(&self, key: FileRefKey) {
        self.entries.lock().await.remove(&key);
    }

    /// Number of live entries.
    pub async fn len(&self) -> usize {
        self.entries.lock().await.len()
    }

    /// True when no entries are cached.
    pub async fn is_empty(&self) -> bool {
        self.entries.lock().await.is_empty()
    }
}

impl Default for FileRefCache {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// BS-6: DC rotation bookkeeping
// ===========================================================================

/// DC migration + config refresh bookkeeping (BS-6).
///
/// The background refresh task itself lives in `Client::spawn_dc_refresher`;
/// this struct is the shared state it mutates.
#[derive(Debug)]
pub struct DcRotator {
    inner: Mutex<DcRotatorInner>,
}

#[derive(Debug, Default)]
struct DcRotatorInner {
    /// DC option table from the last `help.getConfig` (dc_id -> ip:port).
    dc_options: HashMap<i32, String>,
    /// When the config was last refreshed (None = never).
    last_refresh: Option<Instant>,
}

impl DcRotator {
    /// New rotator with an empty option table.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(DcRotatorInner::default()),
        }
    }

    /// Store the DC option table from a fresh `help.getConfig`.
    pub async fn update_config(&self, options: HashMap<i32, String>) {
        let mut inner = self.inner.lock().await;
        inner.dc_options = options;
        inner.last_refresh = Some(Instant::now());
    }

    /// Endpoint (ip:port) for `dc_id`, if known from the config table.
    pub async fn endpoint(&self, dc_id: i32) -> Option<String> {
        self.inner.lock().await.dc_options.get(&dc_id).cloned()
    }

    /// Age of the current config (None = never refreshed).
    pub async fn age(&self) -> Option<Duration> {
        self.inner.lock().await.last_refresh.map(|t| t.elapsed())
    }
}

impl Default for DcRotator {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience alias for the shared handle used by `Client`.
pub type SharedFloodLimiter = Arc<FloodLimiter>;
pub type SharedFileRefCache = Arc<FileRefCache>;
pub type SharedDcRotator = Arc<DcRotator>;

/// True when the error is a file-reference expiry (the BS-3 trigger).
pub fn is_file_ref_expired(e: &Error) -> bool {
    matches!(e, Error::FileReferenceExpired { .. })
}
