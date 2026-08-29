//! Telegram API types, constructor definitions, and authorization flows.
//!
//! Supports both:
//! - **User auth** via phone number + OTP code (full MTProto DH handshake)
//! - **Bot auth** via bot token (direct authorizeURL call)

use crate::error::{Error, Result};
use crate::mtproto::{AuthKeyCreation, AuthKeyResult, MtProtoSession};
use crate::serialize::{TLWriter, TLReader, *};
use crate::types;
use crate::transport;
use tokio::io::AsyncWriteExt;

/// API layer version (Layer 223).
pub const API_LAYER: i32 = 223;

// ---------------------------------------------------------------------------
// API Client
// ---------------------------------------------------------------------------

/// High-level Telegram API client supporting both user and bot authorization.
pub struct TelegramClient {
    pub session: Option<MtProtoSession>,
    pub user_id: Option<i64>,
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
        if std::env::var("MTPRSTO_DEBUG_DH").is_ok() {
            tracing::warn!(
                "req_DH_params ({} bytes): {}",
                req_dh.len(),
                req_dh.iter().map(|b| format!("{b:02x}")).collect::<String>()
            );
        }

        // Step 5: Send req_DH_params
        let mut stream = transport::connect(self.dc_id).await?;
        transport::send_unencrypted(&mut stream, 0, &req_dh).await?;
        let (_, dh_response) = transport::recv_unencrypted(&mut stream).await?;

        // Step 6: Parse server_DH_params_ok
        auth.parse_server_dh_params(&dh_response)?;

        // Steps 7-9: set_client_DH_params, parse answer; on dh_gen_retry,
        // regenerate b and retry with retry_id = previous attempt's
        // auth_key_aux_hash (SPEC §7/§9).
        const MAX_DH_ATTEMPTS: u32 = 4;
        for attempt in 0..MAX_DH_ATTEMPTS {
            // Step 7: Build set_client_DH_params (fresh b every attempt;
            // retry_id inside is 0 first, then the previous aux hash).
            let set_dh = auth.build_set_client_dh_params()?;

            // Step 8: Send set_client_DH_params
            transport::send_unencrypted(&mut stream, 0, &set_dh).await?;
            let (_, dh_gen_response) = transport::recv_unencrypted(&mut stream).await?;

            // Step 9: Parse the answer
            match auth.parse_dh_gen_result(&dh_gen_response)? {
                AuthKeyResult::Ok => {
                    let auth_key = auth.compute_auth_key()?;
                    let server_salt = auth.compute_server_salt()?;
                    self.session = Some(MtProtoSession::new(auth_key, server_salt));
                    return Ok(());
                }
                AuthKeyResult::Retry => {
                    // Server rejected this candidate key: promote this
                    // attempt's aux hash to the next attempt's retry_id and
                    // go around again with a fresh b.
                    auth.retry_id = auth.auth_key_aux_hash.unwrap_or(0);
                    tracing::warn!(
                        "dh_gen_retry from server (attempt {}), retrying with retry_id={:#x}",
                        attempt + 1, auth.retry_id
                    );
                }
                AuthKeyResult::Fail => {
                    return Err(Error::Protocol("DH key creation failed".into()));
                }
            }
        }
        Err(Error::Protocol(format!(
            "DH key creation still retrying after {MAX_DH_ATTEMPTS} attempts"
        )))
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
        payload.write_u32(types::IMPORT_BOT_AUTH); // auth.importBotAuthorization#67a3ff2c
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
            types::AUTH_AUTHORIZATION => {
                // Bot authorization successful
                tracing::info!("bot authorization succeeded");
                Ok(())
            }
            types::AUTH_AUTHORIZATION_SIGN_UP_REQUIRED => {
                Err(Error::SignUpRequired)
            }
            RPC_ERROR => {
                let (code, msg) = crate::mtproto::parse_rpc_error(&plaintext)?;
                Err(crate::error::classify_rpc_error(code, &msg))
            }
            _ => Err(Error::UnexpectedResponse(format!(
                "unexpected constructor {:#x} in bot auth response",
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
        payload.write_u32(types::AUTH_SEND_CODE);
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
            types::AUTH_SENT_CODE => {
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
                Err(crate::error::classify_rpc_error(code, &msg))
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
        payload.write_u32(types::AUTH_SIGN_IN);
        payload.write_i32(0); // flags# (no phone_code / email_verification flags)
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
            types::AUTH_AUTHORIZATION => {
                // auth.authorization#2ea2c0d4 flags:# setup_password_required:flags.1?true
                // otherwise_relogin_days:flags.1?int tmp_sessions:flags.0?int
                // future_auth_token:flags.2?bytes user:User
                let flags = r.read_i32()?;
                if flags & (1 << 0) != 0 { let _ = r.read_i32()?; } // tmp_sessions
                if flags & (1 << 1) != 0 { let _ = r.read_i32()?; } // otherwise_relogin_days
                if flags & (1 << 2) != 0 { let _ = r.read_bytes()?; } // future_auth_token
                let user = types::User::read_from(&mut r)?;
                self.user_id = Some(user.id().0);
                Ok(())
            }
            types::AUTH_AUTHORIZATION_SIGN_UP_REQUIRED => {
                Err(Error::SignUpRequired)
            }
            RPC_ERROR => {
                let (code, msg) = crate::mtproto::parse_rpc_error(&plaintext)?;
                Err(crate::error::classify_rpc_error(code, &msg))
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
        payload.write_u32(types::AUTH_SIGN_UP);
        payload.write_i32(0); // flags# (no_joined_notifications off)
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
            types::AUTH_AUTHORIZATION => {
                // auth.authorization#2ea2c0d4 — same layout as auth_sign_in
                let flags = r.read_i32()?;
                if flags & (1 << 0) != 0 { let _ = r.read_i32()?; } // tmp_sessions
                if flags & (1 << 1) != 0 { let _ = r.read_i32()?; } // otherwise_relogin_days
                if flags & (1 << 2) != 0 { let _ = r.read_bytes()?; } // future_auth_token
                let user = types::User::read_from(&mut r)?;
                self.user_id = Some(user.id().0);
                Ok(())
            }
            RPC_ERROR => {
                let (code, msg) = crate::mtproto::parse_rpc_error(&plaintext)?;
                Err(crate::error::classify_rpc_error(code, &msg))
            }
            _ => Err(Error::UnexpectedResponse(format!(
                "unexpected constructor {:#x} in auth_sign_up response",
                constructor
            ))),
        }
    }

    /// Log out and invalidate the current authorization.
    ///
    /// `auth.logOut#3e72ba19 = auth.LoggedOut;` — server may return a
    /// `future_auth_token` for resync; we surface it to the caller.
    pub async fn auth_log_out(&mut self) -> Result<Option<Vec<u8>>> {
        let plaintext = self.invoke(types::AUTH_LOG_OUT, &[]).await?;
        let mut r = TLReader::new(&plaintext);
        let constructor = r.read_u32()?;

        match constructor {
            types::AUTH_LOGGED_OUT => {
                let flags = r.read_i32()?;
                let token = if flags & (1 << 0) != 0 {
                    Some(r.read_bytes()?)
                } else {
                    None
                };
                // Our session is dead server-side.
                self.user_id = None;
                Ok(token)
            }
            RPC_ERROR => {
                let (code, msg) = crate::mtproto::parse_rpc_error(&plaintext)?;
                Err(crate::error::classify_rpc_error(code, &msg))
            }
            _ => Err(Error::UnexpectedResponse(format!(
                "unexpected constructor {:#x} in auth_log_out response",
                constructor
            ))),
        }
    }

    /// Verify a 2FA cloud password and finish sign-in.
    ///
    /// `auth.checkPassword#d18b4d16 password:InputCheckPasswordSRP`.
    /// Fetches `account.getPassword` for the server challenge, runs the
    /// client side of SRP with the password, and sends the proof.
    pub async fn auth_check_password(&mut self, password: &str) -> Result<()> {
        // account.getPassword → the SRP challenge (algo + srp_B + srp_id).
        let raw = self.invoke(types::ACCOUNT_GET_PASSWORD, &[]).await?;
        let challenge = parse_account_password(&raw)?;
        let algo = challenge.algo.as_ref().ok_or_else(|| {
            Error::Protocol("account.password has no current_algo (no password set?)".into())
        })?;

        let params = crate::crypto::SrpParams {
            salt1: algo.salt1.clone(),
            salt2: algo.salt2.clone(),
            g: algo.g,
            p: algo.p.clone(),
            b: challenge.srp_b.clone(),
            srp_id: challenge.srp_id,
        };
        let answer = crate::crypto::srp_check_password(password.as_bytes(), &params);

        // inputCheckPasswordSRP#d27ff082 srp_id:long A:bytes M1:bytes
        let mut srp_payload = TLWriter::new();
        srp_payload.write_u32(types::INPUT_CHECK_PASSWORD_SRP);
        srp_payload.write_i64(answer.srp_id);
        srp_payload.write_bytes(&answer.a);
        srp_payload.write_bytes(&answer.m1);

        let plaintext = self.invoke(types::AUTH_CHECK_PASSWORD, &srp_payload.into_bytes()).await?;
        let mut r = TLReader::new(&plaintext);
        let constructor = r.read_u32()?;

        match constructor {
            types::AUTH_AUTHORIZATION => {
                let user = parse_authorization_user(&mut r)?;
                self.user_id = Some(user);
                Ok(())
            }
            types::AUTH_AUTHORIZATION_SIGN_UP_REQUIRED => Err(Error::SignUpRequired),
            RPC_ERROR => {
                let (code, msg) = crate::mtproto::parse_rpc_error(&plaintext)?;
                Err(crate::error::classify_rpc_error(code, &msg))
            }
            _ => Err(Error::UnexpectedResponse(format!(
                "unexpected constructor {:#x} in auth_check_password response",
                constructor
            ))),
        }
    }

    /// QR-code login, step 1: request a login token to render as
    /// `tg://login?token=...`.
    ///
    /// `auth.exportLoginToken#b7e085fe api_id:int api_hash:string except_ids:Vector<long>`.
    pub async fn auth_export_login_token(
        &mut self,
        except_ids: &[i64],
    ) -> Result<AuthLoginToken> {
        let mut payload = TLWriter::new();
        payload.write_u32(types::AUTH_EXPORT_LOGIN_TOKEN);
        payload.write_i32(self.api_id.unwrap_or(0));
        payload.write_bytes(self.api_hash.as_deref().unwrap_or("").as_bytes());
        payload.write_i32(except_ids.len() as i32);
        for id in except_ids {
            payload.write_i64(*id);
        }

        let plaintext = self.invoke(types::AUTH_EXPORT_LOGIN_TOKEN, &payload.into_bytes()).await?;
        parse_login_token_response(&plaintext)
    }

    /// QR-code login, step 2 (caller side): import a token scanned from
    /// another device and poll until it is approved.
    ///
    /// `auth.importLoginToken#95ac5ce4 token:bytes = auth.LoginToken;`
    pub async fn auth_import_login_token(&mut self, token: &[u8]) -> Result<AuthLoginToken> {
        let mut payload = TLWriter::new();
        payload.write_u32(types::AUTH_IMPORT_LOGIN_TOKEN);
        payload.write_bytes(token);

        let plaintext = self.invoke(types::AUTH_IMPORT_LOGIN_TOKEN, &payload.into_bytes()).await?;
        parse_login_token_response(&plaintext)
    }

    /// QR-code login, other side: accept a token scanned from a QR code.
    ///
    /// `auth.acceptLoginToken#e894ad4d token:bytes = Authorization;`
    /// Returns the authorized user id, if the token was valid.
    pub async fn auth_accept_login_token(&mut self, token: &[u8]) -> Result<i64> {
        let mut payload = TLWriter::new();
        payload.write_u32(types::AUTH_ACCEPT_LOGIN_TOKEN);
        payload.write_bytes(token);

        let plaintext = self.invoke(types::AUTH_ACCEPT_LOGIN_TOKEN, &payload.into_bytes()).await?;
        let mut r = TLReader::new(&plaintext);
        let constructor = r.read_u32()?;

        match constructor {
            types::AUTH_AUTHORIZATION => parse_authorization_user(&mut r),
            RPC_ERROR => {
                let (code, msg) = crate::mtproto::parse_rpc_error(&plaintext)?;
                Err(crate::error::classify_rpc_error(code, &msg))
            }
            _ => Err(Error::UnexpectedResponse(format!(
                "unexpected constructor {:#x} in auth_accept_login_token response",
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
        let result = self.invoke(types::HELP_GET_NEAREST_DC, &[]).await?;
        let mut r = TLReader::new(&result);
        let constructor = r.read_u32()?;
        if constructor != RPC_ERROR {
            // nearestDc constructor
            Ok(r.read_i32()?) // country (skip) → next DC
        } else {
            let (code, msg) = crate::mtproto::parse_rpc_error(&result)?;
            Err(Error::Rpc {
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


/// Result of an `auth.exportLoginToken` / `auth.importLoginToken` call.
#[derive(Debug, Clone)]
pub enum AuthLoginToken {
    /// Token active — poll again or render the QR code.
    Token {
        expires: i32,
        token: Vec<u8>,
    },
    /// Login must continue on another DC.
    MigrateTo {
        dc_id: i32,
        token: Vec<u8>,
    },
    /// The other device approved the login.
    Success,
}

/// Parse a `auth.LoginToken` response.
fn parse_login_token_response(plaintext: &[u8]) -> Result<AuthLoginToken> {
    let mut r = TLReader::new(plaintext);
    let constructor = r.read_u32()?;

    match constructor {
        types::AUTH_LOGIN_TOKEN => {
            let expires = r.read_i32()?;
            let token = r.read_bytes()?;
            Ok(AuthLoginToken::Token { expires, token })
        }
        types::AUTH_LOGIN_TOKEN_MIGRATE_TO => {
            let dc_id = r.read_i32()?;
            let token = r.read_bytes()?;
            Ok(AuthLoginToken::MigrateTo { dc_id, token })
        }
        types::AUTH_LOGIN_TOKEN_SUCCESS => Ok(AuthLoginToken::Success),
        RPC_ERROR => {
            let (code, msg) = crate::mtproto::parse_rpc_error(plaintext)?;
            Err(crate::error::classify_rpc_error(code, &msg))
        }
        _ => Err(Error::UnexpectedResponse(format!(
            "unexpected constructor {:#x} in login token response",
            constructor
        ))),
    }
}

/// Parse an `auth.Authorization` body (after the constructor) and return
/// the authorized user's id.
fn parse_authorization_user(r: &mut TLReader) -> Result<i64> {
    // auth.authorization#2ea2c0d4 flags:# setup_password_required:flags.1?true
    // otherwise_relogin_days:flags.1?int tmp_sessions:flags.0?int
    // future_auth_token:flags.2?bytes user:User
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
    let user = types::User::read_from(r)?;
    Ok(user.id().0)
}

/// The parts of `account.password` that `auth.checkPassword` needs.
#[derive(Debug, Clone)]
pub struct AccountPasswordChallenge {
    /// `current_algo` payload — None when no password is set.
    pub algo: Option<PasswordKdfAlgo>,
    /// `srp_B`.
    pub srp_b: Vec<u8>,
    /// `srp_id`.
    pub srp_id: i64,
}

/// `passwordKdfAlgoSHA256SHA256PBKDF2HMACSHA512iter100000SHA256ModPow` body.
#[derive(Debug, Clone)]
pub struct PasswordKdfAlgo {
    pub salt1: Vec<u8>,
    pub salt2: Vec<u8>,
    pub g: u32,
    pub p: Vec<u8>,
}

/// Parse an `account.password` response.
fn parse_account_password(plaintext: &[u8]) -> Result<AccountPasswordChallenge> {
    // account.password#5188ee1b flags:# has_recovery:flags.0?true
    // has_secure_values:flags.1?true has_password:flags.2?true
    // current_algo:flags.2?PasswordKdfAlgo srp_B:flags.2?bytes
    // srp_id:flags.2?long hint:flags.3?string
    // email_unconfirmed_pattern:flags.4?string new_algo:PasswordKdfAlgo
    // new_secure_algo:SecurePasswordKdfAlgo secure_random:bytes
    // pending_reset_date:flags.5?int login_email_pattern:flags.6?string
    let mut r = TLReader::new(plaintext);
    let constructor = r.read_u32()?;
    if constructor != types::ACCOUNT_GET_PASSWORD_RESPONSE {
        return Err(Error::UnexpectedResponse(format!(
            "expected account.password, got {constructor:#x}"
        )));
    }

    let flags = r.read_i32()?;
    let has_password = flags & (1 << 2) != 0;
    let algo = if has_password {
        let algo_ctor = r.read_u32()?;
        if algo_ctor != types::PASSWORD_KDF_ALGO_SHA256_SHA256_PBKDF2_HMACSHA512_100K_MODPOW {
            return Err(Error::UnexpectedResponse(format!(
                "unsupported passwordKdfAlgo {algo_ctor:#x}"
            )));
        }
        Some(PasswordKdfAlgo {
            salt1: r.read_bytes()?,
            salt2: r.read_bytes()?,
            g: r.read_i32()? as u32,
            p: r.read_bytes()?,
        })
    } else {
        None
    };
    let srp_b = if has_password { r.read_bytes()? } else { Vec::new() };
    let srp_id = if has_password { r.read_i64()? } else { 0 };
    let hint = if flags & (1 << 3) != 0 { r.read_bytes()? } else { Vec::new() };
    let _ = hint;
    let email = if flags & (1 << 4) != 0 { r.read_bytes()? } else { Vec::new() };
    let _ = email;
    let _new_algo_ctor = r.read_u32()?; // skip new_algo (only ctor id read)
    let _new_secure_algo_ctor = r.read_u32()?; // skip new_secure_algo
    let _secure_random = r.read_bytes()?;
    if flags & (1 << 5) != 0 {
        let _ = r.read_i32()?; // pending_reset_date
    }
    if flags & (1 << 6) != 0 {
        let _ = r.read_bytes()?; // login_email_pattern
    }

    Ok(AccountPasswordChallenge {
        algo,
        srp_b,
        srp_id,
    })
}

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
pub fn serialize_input_peer_user(user_id: i64, access_hash: i64) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(types::INPUT_PEER_USER);
    w.write_i64(user_id);
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
        assert_eq!(r.read_u32().unwrap(), types::INPUT_PEER_USER);
        assert_eq!(r.read_i64().unwrap(), 12345);
        assert_eq!(r.read_i64().unwrap(), 67890);
    }

    #[test]
    fn test_client_creation() {
        let client = TelegramClient::new(2, Some(12345), Some("hash".into()));
        assert!(client.session.is_none());
        assert_eq!(client.dc_id, 2);
    }

    #[test]
    fn test_parse_login_token_response_variants() {
        // auth.loginToken#629f1980 expires:int token:bytes
        let mut w = TLWriter::new();
        w.write_u32(types::AUTH_LOGIN_TOKEN);
        w.write_i32(3600);
        w.write_bytes(&[1, 2, 3, 4]);
        match parse_login_token_response(&w.into_bytes()).unwrap() {
            AuthLoginToken::Token { expires, token } => {
                assert_eq!(expires, 3600);
                assert_eq!(token, vec![1, 2, 3, 4]);
            }
            _ => panic!("expected Token"),
        }

        // auth.loginTokenMigrateTo#68e9916 dc_id:int token:bytes
        let mut w = TLWriter::new();
        w.write_u32(types::AUTH_LOGIN_TOKEN_MIGRATE_TO);
        w.write_i32(2);
        w.write_bytes(&[9]);
        match parse_login_token_response(&w.into_bytes()).unwrap() {
            AuthLoginToken::MigrateTo { dc_id, token } => {
                assert_eq!(dc_id, 2);
                assert_eq!(token, vec![9]);
            }
            _ => panic!("expected MigrateTo"),
        }

        // auth.loginTokenSuccess#390d5c5e
        let mut w = TLWriter::new();
        w.write_u32(types::AUTH_LOGIN_TOKEN_SUCCESS);
        assert!(matches!(
            parse_login_token_response(&w.into_bytes()).unwrap(),
            AuthLoginToken::Success
        ));
    }

    #[test]
    fn test_parse_account_password_roundtrip() {
        // Build account.password#5188ee1b with has_password + no optional extras.
        let mut w = TLWriter::new();
        w.write_u32(types::ACCOUNT_GET_PASSWORD_RESPONSE);
        let flags: i32 = 1 << 2; // has_password
        w.write_i32(flags);
        w.write_u32(types::PASSWORD_KDF_ALGO_SHA256_SHA256_PBKDF2_HMACSHA512_100K_MODPOW);
        w.write_bytes(&[0xAA, 0xBB]); // salt1
        w.write_bytes(&[0xCC]); // salt2
        w.write_i32(3); // g
        w.write_bytes(&[0x11; 256]); // p
        w.write_bytes(&[0x22; 128]); // srp_B
        w.write_i64(0x1234_5678_9abc); // srp_id
        // new_algo / new_secure_algo ctor ids + secure_random
        w.write_u32(0); // passwordKdfAlgoUnknown
        w.write_u32(0); // securePasswordKdfAlgoUnknown
        w.write_bytes(&[0; 32]);

        let challenge = parse_account_password(&w.into_bytes()).unwrap();
        let algo = challenge.algo.expect("has_password set");
        assert_eq!(algo.salt1, vec![0xAA, 0xBB]);
        assert_eq!(algo.salt2, vec![0xCC]);
        assert_eq!(algo.g, 3);
        assert_eq!(algo.p, vec![0x11_u8; 256]);
        assert_eq!(challenge.srp_b, vec![0x22_u8; 128]);
        assert_eq!(challenge.srp_id, 0x1234_5678_9abc);
    }

    #[test]
    fn test_srp_check_password_is_deterministic_shape() {
        // SRP mixes a random a, so M1 differs run to run — but the answer
        // must always be well-formed: A padded to 255 bytes, M1 = 32 bytes.
        let params = crate::crypto::SrpParams {
            salt1: vec![1, 2, 3],
            salt2: vec![4, 5, 6],
            g: 3,
            p: {
                // A 2048-bit odd number is enough for a shape test (ModPow
                // works for any modulus; correctness vs server is covered
                // by interoperability, not unit tests).
                let mut p = vec![0x7F; 256];
                p[255] |= 1;
                p
            },
            b: vec![0x42; 255],
            srp_id: 77,
        };
        let ans = crate::crypto::srp_check_password(b"hunter2", &params);
        assert_eq!(ans.srp_id, 77);
        assert_eq!(ans.a.len(), 255);
        assert_eq!(ans.m1.len(), 32);
    }
}
