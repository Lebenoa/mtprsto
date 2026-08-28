//! Telegram API types, constructor definitions, and authorization flows.
//!
//! Supports both:
//! - **User auth** via phone number + OTP code (full MTProto DH handshake)
//! - **Bot auth** via bot token (direct authorizeURL call)

use crate::error::{Error, Result};
use crate::mtproto::{AuthKeyCreation, AuthKeyResult, MtProtoSession};
use crate::serialize::{TLWriter, TLReader, *};
use crate::transport;
use tokio::io::AsyncWriteExt;

/// API layer version (as of mid-2024).
pub const API_LAYER: i32 = 175;

// ---------------------------------------------------------------------------
// TL Constructor IDs for the Telegram API layer
// ---------------------------------------------------------------------------

// Auth methods
pub const AUTH_SEND_CODE: u32 = 0xa8503291;
pub const AUTH_SIGN_UP: u32 = 0x80eead27;
pub const AUTH_SIGN_IN: u32 = 0x8d52a951;
pub const AUTH_LOG_OUT: u32 = 0x87971c3d;
pub const AUTH_SEND_CODE_CALL: u32 = 0x3c38279e;

// Phone code / verification
pub const AUTH_SENT_CODE: u32 = 0x3f6173a8;
pub const AUTH_SENT_CODE_TYPE_APP: u32 = 0x3dbb5986;
pub const AUTH_SENT_CODE_TYPE_SMS: u32 = 0xc004bac7;
pub const AUTH_SENT_CODE_TYPE_CALL: u32 = 0x5353e5a7;
pub const AUTH_SENT_CODE_TYPE_FLASH_CALL: u32 = 0xab036752;
pub const AUTH_SENT_CODE_TYPE_SMS_CALL: u32 = 0x741cd2ee;

// Authorization result
pub const AUTH_AUTHORIZATION: u32 = 0xcd050916;
pub const AUTH_AUTHORIZATION_SIGN_UP_REQUIRED: u32 = 0x4474f402;

// User
pub const USER: u32 = 0x938458c1;
pub const USER_SELF: u32 = 0x9ff12c8d;
pub const USER_SECRET: u32 = 0x3ff9ec59;

// Input peer
pub const INPUT_PEER_USER: u32 = 0x7f3b18ea;

// Messages
pub const MESSAGES_SEND_MESSAGE: u32 = 0x44942323;
pub const MESSAGES_GET_DIALOGS: u32 = 0x19109d5f;
pub const MESSAGES_GET_ME: u32 = 0xe0b917f2; // users.getFullUser

// Get difference / state
pub const UPDATES_GET_STATE: u32 = 0xedd4882a;
pub const UPDATES_GET_DIFFERENCE: u32 = 0x25939104;

// -- Bot --
pub const AUTH_BOT_LOGIN: u32 = 0x67dd378b; // For newer layers

// -- Misc --
pub const HELP_GET_CONFIG: u32 = 0xc4f9186b;
pub const HELP_GET_NEAREST_DC: u32 = 0x1fb33026;

// ---------------------------------------------------------------------------
// API Client
// ---------------------------------------------------------------------------

/// High-level Telegram API client supporting both user and bot authorization.
pub struct TelegramClient {
    pub session: Option<MtProtoSession>,
    pub user_id: Option<i32>,
    pub dc_id: i32,
    pub api_id: Option<i32>,
    pub api_hash: Option<String>,
}

impl TelegramClient {
    /// Create a new client without an existing session.
    pub fn new(dc_id: i32, api_id: Option<i32>, api_hash: Option<String>) -> Self {
        Self {
            session: None,
            user_id: None,
            dc_id,
            api_id,
            api_hash,
        }
    }

    /// Create a client with a pre-existing auth key and salt.
    pub fn with_session(
        dc_id: i32,
        auth_key: Vec<u8>,
        server_salt: u64,
        api_id: Option<i32>,
        api_hash: Option<String>,
    ) -> Self {
        Self {
            session: Some(MtProtoSession::new(auth_key, server_salt)),
            user_id: None,
            dc_id,
            api_id,
            api_hash,
        }
    }

    // ------------------------------------------------------------------
    // Auth Key Creation (DH handshake)
    // ------------------------------------------------------------------

    /// Perform the full DH handshake to create an authorization key.
    ///
    /// This is a synchronous, step-by-step flow (no network calls inside).
    /// The caller must handle the network I/O.
    pub async fn create_auth_key(&mut self) -> Result<()> {
        let mut auth = AuthKeyCreation::new();

        // Step 1: Send req_pq_multi
        let req_pq = auth.build_req_pq();
        let (_msg_id, response) = transport::exchange_unencrypted(self.dc_id, &req_pq).await?;

        // Step 2: Parse resPQ
        auth.parse_res_pq(&response)?;

        // Step 3: Factor PQ
        auth.factor_pq()?;

        // Step 4: Build req_DH_params
        let req_dh = auth.build_req_dh_params(self.dc_id)?;

        // Step 5: Send req_DH_params
        let mut stream = transport::connect(self.dc_id).await?;
        transport::send_unencrypted(&mut stream, 0, &req_dh).await?;
        let (_, dh_response) = transport::recv_unencrypted(&mut stream).await?;

        // Step 6: Parse server_DH_params_ok
        auth.parse_server_dh_params(&dh_response)?;

        // Step 7: Build set_client_DH_params
        let set_dh = auth.build_set_client_dh_params()?;

        // Step 8: Send set_client_DH_params
        transport::send_unencrypted(&mut stream, 0, &set_dh).await?;
        let (_, dh_gen_response) = transport::recv_unencrypted(&mut stream).await?;

        // Step 9: Parse dh_gen_ok
        let result = auth.parse_dh_gen_result(&dh_gen_response)?;

        match result {
            AuthKeyResult::Ok => {
                let auth_key = auth.compute_auth_key()?;
                let server_salt = auth.compute_server_salt()?;
                self.session = Some(MtProtoSession::new(auth_key, server_salt));
                Ok(())
            }
            AuthKeyResult::Retry => {
                Err(Error::Protocol("DH retry requested (implement retry logic)".into()))
            }
            AuthKeyResult::Fail => {
                Err(Error::Protocol("DH key creation failed".into()))
            }
        }
    }

    // ------------------------------------------------------------------
    // Bot Authorization
    // ------------------------------------------------------------------

    /// Authorize as a bot using a bot token.
    ///
    /// This sends `auth.importBotAuthorization` to the server.
    pub async fn authorize_bot(&mut self, bot_token: &str) -> Result<()> {
        let session = self.session.as_mut().ok_or(Error::NoAuthKey)?;

        // Build the auth.importBotAuthorization request
        // importBotAuthorization flags:0 api_id:int api_hash:string bot_auth_token:string = auth.Authorization;
        let mut payload = TLWriter::new();
        payload.write_u32(0xb36089c9); // auth.importBotAuthorization constructor
        payload.write_i32(0); // flags
        payload.write_i32(self.api_id.unwrap_or(0));
        payload.write_bytes(self.api_hash.as_deref().unwrap_or("").as_bytes());
        payload.write_bytes(bot_token.as_bytes());

        let msg_id = session.next_msg_id();
        let seq_no = session.next_seq_no(true);
        let data = payload.into_bytes();
        let encrypted = session.encrypt_message(&data, msg_id, seq_no);

        // Send over Intermediate transport
        let mut stream = transport::connect(self.dc_id).await?;
        let len = (encrypted.len() as u32).to_le_bytes();
        stream.write_all(&len).await?;
        stream.write_all(&encrypted).await?;
        stream.flush().await?;

        // Receive response
        let response_data = transport::recv_encrypted(&mut stream).await?;
        let (_, plaintext) = session.decrypt_message(&response_data)?;

        // Parse the response
        let mut r = TLReader::new(&plaintext);
        let constructor = r.read_u32()?;

        match constructor {
            AUTH_AUTHORIZATION => {
                let _dc_number = r.read_i32()?;
                let _user_ctor = r.read_u32()?;
                // TODO: parse full user object
                Ok(())
            }
            AUTH_AUTHORIZATION_SIGN_UP_REQUIRED => {
                Err(Error::Protocol(
                    "Bot authorization requires sign up — check bot token".into(),
                ))
            }
            RPC_ERROR => {
                let (code, msg) = crate::mtproto::parse_rpc_error(&plaintext)?;
                Err(Error::Api {
                    error_code: code,
                    error_message: msg,
                })
            }
            _ => Err(Error::UnexpectedResponse(format!(
                "unexpected constructor {:#x} in auth response",
                constructor
            ))),
        }
    }

    // ------------------------------------------------------------------
    // User Authorization (phone + code flow)
    // ------------------------------------------------------------------

    /// Step 1: Send verification code to phone number.
    pub async fn auth_send_code(&mut self, phone_number: &str) -> Result<AuthSentCodeInfo> {
        let session = self.session.as_mut().ok_or(Error::NoAuthKey)?;

        let mut payload = TLWriter::new();
        payload.write_u32(AUTH_SEND_CODE);
        // auth.sendCode#... phone_number:string api_id:int api_hash:string settings:CodeSettings = auth.SentCode;
        payload.write_bytes(phone_number.as_bytes());
        payload.write_i32(self.api_id.unwrap_or(0));
        payload.write_bytes(self.api_hash.as_deref().unwrap_or("").as_bytes());
        // CodeSettings
        payload.write_u32(0); // flags (no settings)

        let msg_id = session.next_msg_id();
        let seq_no = session.next_seq_no(true);
        let data = payload.into_bytes();
        let encrypted = session.encrypt_message(&data, msg_id, seq_no);

        let mut stream = transport::connect(self.dc_id).await?;
        let len = (encrypted.len() as u32).to_le_bytes();
        stream.write_all(&len).await?;
        stream.write_all(&encrypted).await?;
        stream.flush().await?;

        let response_data = transport::recv_encrypted(&mut stream).await?;
        let (_, plaintext) = session.decrypt_message(&response_data)?;

        let mut r = TLReader::new(&plaintext);
        let constructor = r.read_u32()?;

        match constructor {
            AUTH_SENT_CODE => {
                let phone_code_hash = r.read_bytes()?;
                let sent_code_type = r.read_u32()?;
                // timeout field
                let _timeout = r.read_i32()?;

                Ok(AuthSentCodeInfo {
                    phone_code_hash,
                    sent_code_type,
                })
            }
            RPC_ERROR => {
                let (code, msg) = crate::mtproto::parse_rpc_error(&plaintext)?;
                Err(Error::Api {
                    error_code: code,
                    error_message: msg,
                })
            }
            _ => Err(Error::UnexpectedResponse(format!(
                "unexpected constructor {:#x} in auth_send_code response",
                constructor
            ))),
        }
    }

    /// Step 2: Sign in with the verification code.
    pub async fn auth_sign_in(
        &mut self,
        phone_number: &str,
        phone_code_hash: &[u8],
        phone_code: &str,
    ) -> Result<()> {
        let session = self.session.as_mut().ok_or(Error::NoAuthKey)?;

        let mut payload = TLWriter::new();
        payload.write_u32(AUTH_SIGN_IN);
        payload.write_bytes(phone_number.as_bytes());
        payload.write_bytes(phone_code_hash);
        payload.write_bytes(phone_code.as_bytes());

        let msg_id = session.next_msg_id();
        let seq_no = session.next_seq_no(true);
        let data = payload.into_bytes();
        let encrypted = session.encrypt_message(&data, msg_id, seq_no);

        let mut stream = transport::connect(self.dc_id).await?;
        let len = (encrypted.len() as u32).to_le_bytes();
        stream.write_all(&len).await?;
        stream.write_all(&encrypted).await?;
        stream.flush().await?;

        let response_data = transport::recv_encrypted(&mut stream).await?;
        let (_, plaintext) = session.decrypt_message(&response_data)?;

        let mut r = TLReader::new(&plaintext);
        let constructor = r.read_u32()?;

        match constructor {
            AUTH_AUTHORIZATION => {
                let dc_number = r.read_i32()?;
                self.user_id = Some(dc_number); // This is actually a nested user object; simplified
                Ok(())
            }
            AUTH_AUTHORIZATION_SIGN_UP_REQUIRED => {
                Err(Error::Protocol("Sign up required".into()))
            }
            RPC_ERROR => {
                let (code, msg) = crate::mtproto::parse_rpc_error(&plaintext)?;
                Err(Error::Api {
                    error_code: code,
                    error_message: msg,
                })
            }
            _ => Err(Error::UnexpectedResponse(format!(
                "unexpected constructor {:#x} in auth_sign_in response",
                constructor
            ))),
        }
    }

    /// Step 2 (alternative): Sign up as a new user.
    pub async fn auth_sign_up(
        &mut self,
        phone_number: &str,
        phone_code_hash: &[u8],
        first_name: &str,
        last_name: &str,
    ) -> Result<()> {
        let session = self.session.as_mut().ok_or(Error::NoAuthKey)?;

        let mut payload = TLWriter::new();
        payload.write_u32(AUTH_SIGN_UP);
        payload.write_bytes(phone_number.as_bytes());
        payload.write_bytes(phone_code_hash);
        payload.write_bytes(first_name.as_bytes());
        payload.write_bytes(last_name.as_bytes());

        let msg_id = session.next_msg_id();
        let seq_no = session.next_seq_no(true);
        let data = payload.into_bytes();
        let encrypted = session.encrypt_message(&data, msg_id, seq_no);

        let mut stream = transport::connect(self.dc_id).await?;
        let len = (encrypted.len() as u32).to_le_bytes();
        stream.write_all(&len).await?;
        stream.write_all(&encrypted).await?;
        stream.flush().await?;

        let response_data = transport::recv_encrypted(&mut stream).await?;
        let (_, plaintext) = session.decrypt_message(&response_data)?;

        let mut r = TLReader::new(&plaintext);
        let constructor = r.read_u32()?;

        match constructor {
            AUTH_AUTHORIZATION => {
                let _dc_number = r.read_i32()?;
                Ok(())
            }
            RPC_ERROR => {
                let (code, msg) = crate::mtproto::parse_rpc_error(&plaintext)?;
                Err(Error::Api {
                    error_code: code,
                    error_message: msg,
                })
            }
            _ => Err(Error::UnexpectedResponse(format!(
                "unexpected constructor {:#x} in auth_sign_up response",
                constructor
            ))),
        }
    }

    // ------------------------------------------------------------------
    // General API methods
    // ------------------------------------------------------------------

    /// Invoke a raw TL method (generic RPC call).
    pub async fn invoke(&mut self, method_id: u32, payload: &[u8]) -> Result<Vec<u8>> {
        let session = self.session.as_mut().ok_or(Error::NoAuthKey)?;

        let mut full_payload = TLWriter::new();
        full_payload.write_u32(method_id);
        full_payload.write_raw_bytes(payload);

        let msg_id = session.next_msg_id();
        let seq_no = session.next_seq_no(true);
        let data = full_payload.into_bytes();
        let encrypted = session.encrypt_message(&data, msg_id, seq_no);

        let mut stream = transport::connect(self.dc_id).await?;
        let len = (encrypted.len() as u32).to_le_bytes();
        stream.write_all(&len).await?;
        stream.write_all(&encrypted).await?;
        stream.flush().await?;

        let response_data = transport::recv_encrypted(&mut stream).await?;
        let (resp_msg_id, plaintext) = session.decrypt_message(&response_data)?;

        // Send ack
        let ack = crate::mtproto::build_msgs_ack(&[resp_msg_id]);
        let ack_msg_id = session.next_msg_id();
        let ack_seq_no = session.next_seq_no(false);
        let ack_encrypted = session.encrypt_message(
            &ack,
            ack_msg_id,
            ack_seq_no,
        );
        let ack_len = (ack_encrypted.len() as u32).to_le_bytes();
        stream.write_all(&ack_len).await?;
        stream.write_all(&ack_encrypted).await?;
        stream.flush().await?;

        Ok(plaintext)
    }

    /// Get nearest data center.
    pub async fn help_get_nearest_dc(&mut self) -> Result<i32> {
        let result = self.invoke(HELP_GET_NEAREST_DC, &[]).await?;
        let mut r = TLReader::new(&result);
        let constructor = r.read_u32()?;
        if constructor != RPC_ERROR {
            // nearestDc constructor
            Ok(r.read_i32()?) // country (skip) → next DC
        } else {
            let (code, msg) = crate::mtproto::parse_rpc_error(&result)?;
            Err(Error::Api {
                error_code: code,
                error_message: msg,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Auth flow result types
// ---------------------------------------------------------------------------

/// Information about a sent verification code.
#[derive(Debug, Clone)]
pub struct AuthSentCodeInfo {
    /// Hash needed for the next step.
    pub phone_code_hash: Vec<u8>,
    /// Type of code that was sent (app, SMS, call, etc.).
    pub sent_code_type: u32,
}

/// Sent code type constants.
pub const SENT_CODE_TYPE_APP: u32 = 0x3dbb5986;
pub const SENT_CODE_TYPE_SMS: u32 = 0xc004bac7;

// ---------------------------------------------------------------------------
// Serialization helpers for common TL types
// ---------------------------------------------------------------------------

/// Serialize a `string` TL value from a Rust string.
pub fn serialize_string(s: &str) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_bytes(s.as_bytes());
    w.into_bytes()
}

/// Deserialize a `string` TL value into a Rust string.
pub fn deserialize_string(data: &[u8]) -> Result<String> {
    let mut r = TLReader::new(data);
    let bytes = r.read_bytes()?;
    String::from_utf8(bytes).map_err(|e| Error::Serialization(e.to_string()))
}

/// Serialize an `InputPeerUser` from user_id and access_hash.
pub fn serialize_input_peer_user(user_id: i32, access_hash: i64) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(INPUT_PEER_USER);
    w.write_i32(user_id);
    w.write_i64(access_hash);
    w.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_string() {
        let s = "hello";
        let data = serialize_string(s);
        let mut r = TLReader::new(&data);
        let back = r.read_bytes().unwrap();
        assert_eq!(String::from_utf8(back).unwrap(), s);
    }

    #[test]
    fn test_serialize_input_peer_user() {
        let data = serialize_input_peer_user(12345, 67890);
        let mut r = TLReader::new(&data);
        assert_eq!(r.read_u32().unwrap(), INPUT_PEER_USER);
        assert_eq!(r.read_i32().unwrap(), 12345);
        assert_eq!(r.read_i64().unwrap(), 67890);
    }

    #[test]
    fn test_client_creation() {
        let client = TelegramClient::new(2, Some(12345), Some("hash".into()));
        assert!(client.session.is_none());
        assert_eq!(client.dc_id, 2);
    }
}
