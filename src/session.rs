//! Session persistence for MTProto connections.
//!
//! Saves and loads `auth_key`, `server_salt`, `session_id`, and
//! `server_time_offset` to/from disk. Format is a simple JSON file
//! that can be extended to match `grammers_session 0.10` schema
//! for one-shot migration from existing grammers sessions.
//!
//! # Persistence strategy
//!
//! ```text
//! ┌─────────────────────────────────────┐
//! │ {                                   │
//! │   "auth_key": "<base64>",           │
//! │   "server_salt": 123456789,         │
//! │   "session_id": 987654321,          │
//! │   "server_time_offset": 0,          │
//! │   "dc_id": 2,                       │
//! │   "user_id": 12345678,              │
//! │   "api_layer": 175,                 │
//! │   "version": 1                      │
//! │ }                                   │
//! └─────────────────────────────────────┘
//! ```

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Persisted session data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionData {
    /// The 256-byte authorization key, base64-encoded.
    pub auth_key: String,
    /// 64-bit server salt.
    pub server_salt: u64,
    /// 64-bit session ID.
    pub session_id: u64,
    /// Server time offset (seconds).
    pub server_time_offset: i64,
    /// DC ID this session is connected to.
    pub dc_id: i32,
    /// User ID (0 for bots or unknown).
    #[serde(default)]
    pub user_id: i64,
    /// API layer version at time of creation.
    #[serde(default)]
    pub api_layer: i32,
    /// Format version (for forward compat).
    #[serde(default = "default_version")]
    pub version: i32,
}

fn default_version() -> i32 {
    1
}

/// Session store that manages persistence to disk.
pub struct SessionStore {
    path: PathBuf,
    data: Option<SessionData>,
}

impl SessionStore {
    /// Create a new session store. Does not load from disk yet.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            data: None,
        }
    }

    /// Load session from disk. Returns `Ok(None)` if the file doesn't exist.
    pub fn load(&mut self) -> Result<Option<SessionData>> {
        if !self.path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&self.path)
            .map_err(|e| Error::Network(std::io::Error::new(e.kind(), format!(
                "failed to read session file {}: {e}", self.path.display()
            ))))?;

        let data: SessionData = serde_json::from_str(&content)
            .map_err(|e| Error::Serialization(format!(
                "failed to parse session file {}: {e}", self.path.display()
            )))?;

        let result = data.clone();
        self.data = Some(data);
        Ok(Some(result))
    }

    /// Save session to disk.
    pub fn save(&mut self, data: &SessionData) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| Error::Network(std::io::Error::new(e.kind(), format!(
                        "failed to create session directory {}: {e}", parent.display()
                    ))))?;
            }
        }

        let content = serde_json::to_string_pretty(data)
            .map_err(|e| Error::Serialization(format!("failed to serialize session: {e}")))?;

        std::fs::write(&self.path, content)
            .map_err(|e| Error::Network(std::io::Error::new(e.kind(), format!(
                "failed to write session file {}: {e}", self.path.display()
            ))))?;

        self.data = Some(data.clone());
        Ok(())
    }

    /// Get the current session data, if loaded.
    pub fn data(&self) -> Option<&SessionData> {
        self.data.as_ref()
    }

    /// Check if a session file exists on disk.
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// Get the path to the session file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Delete the session file from disk.
    pub fn delete(&mut self) -> Result<()> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)
                .map_err(|e| Error::Network(std::io::Error::new(e.kind(), format!(
                    "failed to delete session file {}: {e}", self.path.display()
                ))))?;
        }
        self.data = None;
        Ok(())
    }
}

impl SessionData {
    /// Create a new session from raw auth key bytes.
    pub fn from_auth_key(auth_key: &[u8], server_salt: u64, dc_id: i32) -> Self {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(auth_key);
        Self {
            auth_key: encoded,
            server_salt,
            session_id: rand::random::<u64>(),
            server_time_offset: 0,
            dc_id,
            user_id: 0,
            api_layer: super::api::API_LAYER,
            version: 1,
        }
    }

    /// Decode the auth key back to raw bytes.
    pub fn decode_auth_key(&self) -> Result<Vec<u8>> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(&self.auth_key)
            .map_err(|e| Error::Serialization(format!("invalid auth_key base64: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_path() -> PathBuf {
        let dir = std::env::temp_dir().join("mtprsto_test");
        fs::create_dir_all(&dir).ok();
        dir.join(format!("test_session_{}.json", rand::random::<u64>()))
    }

    #[test]
    fn test_save_and_load() {
        let path = temp_path();
        let mut store = SessionStore::new(&path);

        let data = SessionData::from_auth_key(&vec![0u8; 256], 12345, 2);
        store.save(&data).unwrap();
        assert!(store.exists());

        // Load in a new store
        let mut store2 = SessionStore::new(&path);
        let loaded = store2.load().unwrap().expect("should load session");
        assert_eq!(loaded.auth_key, data.auth_key);
        assert_eq!(loaded.server_salt, 12345);
        assert_eq!(loaded.dc_id, 2);
        assert_eq!(loaded.version, 1);

        // Cleanup
        store.delete().unwrap();
    }

    #[test]
    fn test_load_nonexistent() {
        let path = std::env::temp_dir().join("nonexistent_session.json");
        let mut store = SessionStore::new(&path);
        let result = store.load().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_decode_auth_key() {
        let auth_key = vec![0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x56, 0x78, 0x90];
        let data = SessionData::from_auth_key(&auth_key, 0, 2);
        let decoded = data.decode_auth_key().unwrap();
        assert_eq!(decoded, auth_key);
    }

    #[test]
    fn test_session_file_content_format() {
        let path = temp_path();
        let mut store = SessionStore::new(&path);
        let data = SessionData::from_auth_key(&vec![1u8; 256], 99999, 3);
        store.save(&data).unwrap();

        // Read the raw file and verify it's valid JSON
        let content = fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["server_salt"], 99999);
        assert_eq!(parsed["dc_id"], 3);
        assert_eq!(parsed["version"], 1);

        store.delete().unwrap();
    }
}
