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
pub const API_LAYER: i32 = 225;

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
                    let mut sess = MtProtoSession::new(auth_key, server_salt);

                    // Activate the key: production servers only keep a
                    // freshly negotiated auth key if an encrypted message
                    // follows the DH exchange promptly (gotd/tdlib send
                    // their first RPC on the same connection). Send a ping
                    // here and absorb the reply (pong or bad_server_salt).
                    let mut w = TLWriter::new();
                    w.write_u32(0x7abe77ec); // ping#7abe77ec ping_id:long
                    w.write_i64(rand::random::<i64>());
                    let payload = w.into_bytes();
                    let msg_id = sess.next_msg_id();
                    let seq_no = sess.next_seq_no(true);
                    let encrypted = sess.encrypt_message(&payload, msg_id, seq_no);
                    let len = (encrypted.len() as u32).to_le_bytes();
                    stream.write_all(&len).await?;
                    stream.write_all(&encrypted).await?;
                    stream.flush().await?;
                    let resp = transport::recv_encrypted(&mut stream).await?;
                    let (_, plaintext) = sess.decrypt_message(&resp)?;
                    // If the server corrected our initial salt, adopt it.
                    // bad_server_salt#edab447b bad_msg_id:long(8)
                    // bad_msg_seqno:int(4) error_code:int(4) new_salt:long(8)
                    if plaintext.len() >= 28
                        && u32::from_le_bytes(plaintext[0..4].try_into().unwrap())
                            == crate::serialize::BAD_SERVER_SALT
                    {
                        let new_salt =
                            u64::from_le_bytes(plaintext[20..28].try_into().unwrap());
                        tracing::debug!("adopting corrected server salt after handshake");
                        sess.server_salt = new_salt;
                    }
                    tracing::info!("auth key activated via same-connection ping");

                    self.session = Some(sess);
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

    async fn exchange_encrypted(&mut self, payload: &[u8]) -> Result<Vec<u8>> {
        for outer in 0..2u32 {
            match self.exchange_once(payload).await {
                Err(Error::Transport(_)) if outer == 0 => {
                    tracing::warn!("encrypted exchange failed (transport); retrying");
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                other => return other,
            }
        }
        unreachable!("retry loop returns on every path")
    }

    /// One full exchange on a single connection: send the encrypted
    /// request, then service the session/salt dance (bad_server_salt,
    /// new_session_created) by adopting the server state and re-sending
    /// on the SAME connection — a new connection would just earn a fresh
    /// session handshake every time.
    async fn exchange_once(&mut self, payload: &[u8]) -> Result<Vec<u8>> {
        let session = self.session.as_mut().ok_or(Error::NoAuthKey)?;
        let mut stream = transport::connect(self.dc_id).await?;
        // invokeWithLayer(initConnection(query)) — production servers
        // answer CONNECTION_NOT_INITED without the initConnection wrapper.
        let full_payload = crate::mtproto::build_invoke_with_layer(
            API_LAYER,
            &crate::mtproto::build_init_connection(
                self.api_id.unwrap_or(0),
                "mtprsto",
                "unknown",
                env!("CARGO_PKG_VERSION"),
                "en",
                payload,
            ),
        );
        for _ in 0..4u32 {
            let msg_id = session.next_msg_id();
            let seq_no = session.next_seq_no(true);
            let encrypted = session.encrypt_message(&full_payload, msg_id, seq_no);
            let len = (encrypted.len() as u32).to_le_bytes();
            stream.write_all(&len).await?;
            stream.write_all(&encrypted).await?;
            stream.flush().await?;

            // Read frames until OUR answer arrives. The server re-delivers
            // unacknowledged messages from earlier exchanges on reconnect,
            // so correlate rpc_result by req_msg_id and ack everything —
            // reading the first result blindly answers a request with the
            // previous one's response (they can be byte-shape identical,
            // e.g. auth.sendCode vs auth.signIn).
            let mut answer: Option<Vec<u8>> = None;
            let mut resend = false;
            for _ in 0..16usize {
                let response = transport::recv_encrypted(&mut stream).await?;
                let (_, plaintext) = session.decrypt_message(&response)?;
                let mut ack_ids: Vec<i64> = Vec::new();

                // Iterate messages in the frame (single body or container).
                let mut items: Vec<&[u8]> = Vec::new();
                if plaintext.len() >= 8
                    && u32::from_le_bytes(plaintext[0..4].try_into().unwrap())
                        == crate::serialize::MSG_CONTAINER
                {
                    let count = i32::from_le_bytes(plaintext[4..8].try_into().unwrap());
                    let mut off = 8usize;
                    for _ in 0..count.max(0) {
                        if off + 12 > plaintext.len() {
                            break;
                        }
                        off += 12; // msg_id:long seq_no:int
                        if off + 4 > plaintext.len() {
                            break;
                        }
                        let len =
                            i32::from_le_bytes(plaintext[off..off + 4].try_into().unwrap()) as usize;
                        off += 4;
                        if off + len > plaintext.len() {
                            break;
                        }
                        items.push(&plaintext[off..off + len]);
                        off += (len + 3) & !3;
                    }
                } else {
                    items.push(&plaintext[..]);
                }

                for item in items {
                    if item.len() < 4 {
                        continue;
                    }
                    let ctor = u32::from_le_bytes(item[0..4].try_into().unwrap());
                    match ctor {
                        crate::serialize::BAD_SERVER_SALT => {
                            // The query was NOT processed — adopt the fresh
                            // salt (at [20..28]) and re-send it.
                            if item.len() >= 28 {
                                session.server_salt =
                                    u64::from_le_bytes(item[20..28].try_into().unwrap());
                            }
                            resend = true;
                        }
                        crate::serialize::NEW_SESSION_CREATED => {
                            // The server re-earned the session but DOES
                            // process the triggering message — adopt the
                            // salt and keep waiting for the answer.
                            // Re-sending would execute the query twice
                            // (double sendCode, self-inflicted flood on
                            // the second copy).
                            if item.len() >= 28 {
                                session.server_salt =
                                    u64::from_le_bytes(item[20..28].try_into().unwrap());
                            }
                        }
                        crate::serialize::NEW_SERVER_SALT => {
                            if item.len() >= 12 {
                                session.server_salt =
                                    u64::from_le_bytes(item[4..12].try_into().unwrap());
                            }
                        }
                        crate::serialize::MSGS_ACK | crate::serialize::PONG => {}
                        crate::serialize::RPC_RESULT if item.len() >= 12 => {
                            let req = i64::from_le_bytes(item[4..12].try_into().unwrap());
                            ack_ids.push(req);
                            if req == msg_id as i64 && answer.is_none() {
                                answer = Some(item[12..].to_vec());
                            }
                        }
                        _ => {} // updates / pong-like payloads: ignored
                    }
                }

                // Ack the results we consumed so the server stops
                // re-delivering them on the next exchange.
                if !ack_ids.is_empty() {
                    let mut ack = TLWriter::new();
                    ack.write_u32(crate::serialize::MSGS_ACK);
                    ack.write_u32(crate::serialize::VECTOR);
                    ack.write_i32(ack_ids.len() as i32);
                    for id in ack_ids {
                        ack.write_i64(id);
                    }
                    let aid = session.next_msg_id();
                    let asn = session.next_seq_no(false);
                    let enc = session.encrypt_message(&ack.into_bytes(), aid, asn);
                    let alen = (enc.len() as u32).to_le_bytes();
                    let _ = stream.write_all(&alen).await;
                    let _ = stream.write_all(&enc).await;
                    let _ = stream.flush().await;
                }

                if answer.is_some() || resend {
                    break;
                }
            }

            if let Some(a) = answer {
                return Ok(a);
            }
            if resend {
                tracing::debug!("session/salt correction — re-sending query");
                continue;
            }
        }
        Err(Error::Protocol(
            "exchange did not settle after session/salt corrections".into(),
        ))
    }

    /// Authorize as a bot using a bot token.
    ///
    /// This sends `auth.importBotAuthorization` to the server.
    pub async fn authorize_bot(&mut self, bot_token: &str) -> Result<i64> {
        // importBotAuthorization flags:0 api_id:int api_hash:string bot_auth_token:string = auth.Authorization;
        let mut payload = TLWriter::new();
        payload.write_u32(types::IMPORT_BOT_AUTH); // auth.importBotAuthorization#67a3ff2c
        payload.write_i32(0); // flags
        payload.write_i32(self.api_id.unwrap_or(0));
        payload.write_bytes(self.api_hash.as_deref().unwrap_or("").as_bytes());
        payload.write_bytes(bot_token.as_bytes());

        let plaintext = self.exchange_encrypted(payload.as_bytes()).await?;

        // Parse the response
        let mut r = TLReader::new(&plaintext);
        let constructor = r.read_u32()?;

        match constructor {
            types::AUTH_AUTHORIZATION => {
                // auth.authorization#2ea2c0d4 flags:# tmp_sessions:flags.0?int
                //   user:User — capture the bot's user id for the session.
                let flags = r.read_i32()?;
                if flags & (1 << 0) != 0 {
                    let _ = r.read_i32()?; // tmp_sessions
                }
                let user = crate::types::User::read_from(&mut r)?;
                tracing::info!("bot authorization succeeded");
                Ok(user.id().0)
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


        let mut payload = TLWriter::new();
        payload.write_u32(types::AUTH_SEND_CODE);
        // auth.sendCode#... phone_number:string api_id:int api_hash:string settings:CodeSettings = auth.SentCode;
        payload.write_bytes(phone_number.as_bytes());
        payload.write_i32(self.api_id.unwrap_or(0));
        payload.write_bytes(self.api_hash.as_deref().unwrap_or("").as_bytes());
        // CodeSettings is a full TL object in modern layers:
        // codeSettings#ad253d78 flags:# ... (no optional fields when flags=0)
        payload.write_u32(types::CODE_SETTINGS);
        payload.write_i32(0);

        let plaintext = self.exchange_encrypted(payload.as_bytes()).await?;

        let mut r = TLReader::new(&plaintext);
        let constructor = r.read_u32()?;

        match constructor {
            types::AUTH_SENT_CODE => parse_sent_code_response(&plaintext),
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


        let mut payload = TLWriter::new();
        payload.write_u32(types::AUTH_SIGN_IN);
        // auth.signIn#8d52a951 flags:# phone_number:string phone_code_hash:string
        //   phone_code:flags.0?string email_verification:flags.1?EmailVerification
        payload.write_i32(1 << 0); // phone_code provided
        payload.write_bytes(phone_number.as_bytes());
        payload.write_bytes(phone_code_hash);
        payload.write_bytes(phone_code.as_bytes());

        let plaintext = self.exchange_encrypted(payload.as_bytes()).await?;

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
            types::AUTH_SENT_CODE => {
                // The code session expired while the user was typing — the
                // server sent a fresh code. Retry sign-in with the new hash.
                let sent = parse_sent_code_response(&plaintext)?;
                Err(Error::CodeResent { phone_code_hash: sent.phone_code_hash })
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


        let mut payload = TLWriter::new();
        payload.write_u32(types::AUTH_SIGN_UP);
        payload.write_i32(0); // flags# (no_joined_notifications off)
        payload.write_bytes(phone_number.as_bytes());
        payload.write_bytes(phone_code_hash);
        payload.write_bytes(first_name.as_bytes());
        payload.write_bytes(last_name.as_bytes());

        let plaintext = self.exchange_encrypted(payload.as_bytes()).await?;

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
            types::AUTH_SENT_CODE => {
                let sent = parse_sent_code_response(&plaintext)?;
                Err(Error::CodeResent { phone_code_hash: sent.phone_code_hash })
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
        let mut full_payload = TLWriter::new();
        full_payload.write_u32(method_id);
        full_payload.write_raw_bytes(payload);

        // The ack for this response is intentionally skipped: the one-shot
        // exchange connection closes right after the reply, and Telegram
        // re-delivers unacked service frames if it ever matters.
        self.exchange_encrypted(full_payload.as_bytes()).await
    }

    /// Get nearest data center.
    ///
    /// Returns `(this_dc, nearest_dc)`. Requires an auth key (any key,
    /// even unauthenticated) — usable right after the DH handshake.
    pub async fn help_get_nearest_dc(&mut self) -> Result<(i32, i32)> {
        let result = self.invoke(types::HELP_GET_NEAREST_DC, &[]).await?;
        let mut r = TLReader::new(&result);
        let constructor = r.read_u32()?;
        if constructor == types::NEAREST_DC {
            // nearestDc#8e1a1775 country:string this_dc:int nearest_dc:int
            let _country = r.read_bytes()?;
            let this_dc = r.read_i32()?;
            let nearest_dc = r.read_i32()?;
            Ok((this_dc, nearest_dc))
        } else if constructor == RPC_ERROR {
            let (code, msg) = crate::mtproto::parse_rpc_error(&result)?;
            Err(Error::Rpc {
                error_code: code,
                error_message: msg,
            })
        } else {
            Err(Error::UnexpectedResponse(format!(
                "unexpected constructor {constructor:#x} in getNearestDc response"
            )))
        }
    }
}

/// Parse an `auth.sentCode#5e002502` response into [`AuthSentCodeInfo`].
pub(crate) fn parse_sent_code_response(plaintext: &[u8]) -> Result<AuthSentCodeInfo> {
    let mut r = TLReader::new(plaintext);
    let _ctor = r.read_u32()?;
    // flags:# type:auth.SentCodeType phone_code_hash:string
    //   next_type:flags.1?auth.CodeType timeout:flags.2?int
    let flags = r.read_i32()?;
    let sent_code_type = read_sent_code_type(&mut r)?;
    let phone_code_hash = r.read_bytes()?;
    if flags & (1 << 1) != 0 { skip_code_type(&mut r)?; }
    if flags & (1 << 2) != 0 { let _ = r.read_i32()?; }
    Ok(AuthSentCodeInfo { phone_code_hash, sent_code_type })
}

/// Parse/skip an `auth.SentCodeType` object, returning its constructor ID.
/// The caller can match it against the `SENT_CODE_TYPE_*` constants.
pub(crate) fn read_sent_code_type(r: &mut TLReader) -> Result<u32> {
    let ctor = r.read_u32()?;
    match ctor {
        // sentCodeTypeApp#3dbb5986 length:int | sentCodeTypeSms#c000bba2 length:int
        // | sentCodeTypeCall#5353e5a7 length:int
        types::AUTH_SENT_CODE_TYPE_APP | types::AUTH_SENT_CODE_TYPE_SMS | SENT_CODE_TYPE_CALL => {
            let _length = r.read_i32()?;
        }
        // sentCodeTypeFlashCall#ab03c6d9 pattern:string
        SENT_CODE_TYPE_FLASH_CALL => {
            let _pattern = r.read_bytes()?;
        }
        // sentCodeTypeMissedCall#82006484 prefix:string length:int
        SENT_CODE_TYPE_MISSED_CALL => {
            let _prefix = r.read_bytes()?;
            let _length = r.read_i32()?;
        }
        // sentCodeTypeEmailCode#f450f59b flags:# apple_signin_allowed:flags.0?true
        //   google_signin_allowed:flags.1?true email_pattern:string length:int
        //   reset_available_period:flags.3?int reset_pending_date:flags.4?int
        SENT_CODE_TYPE_EMAIL_CODE => {
            let flags = r.read_i32()?;
            let _email_pattern = r.read_bytes()?;
            let _length = r.read_i32()?;
            if flags & (1 << 3) != 0 { let _ = r.read_i32()?; }
            if flags & (1 << 4) != 0 { let _ = r.read_i32()?; }
        }
        // sentCodeTypeSetUpEmailRequired#a5491dea flags:# apple_signin_allowed:flags.0?true
        //   google_signin_allowed:flags.1?true
        SENT_CODE_TYPE_SET_UP_EMAIL_REQUIRED => {
            let _flags = r.read_i32()?;
        }
        // sentCodeTypeFragmentSms#d9565c39 url:string length:int
        SENT_CODE_TYPE_FRAGMENT_SMS => {
            let _url = r.read_bytes()?;
            let _length = r.read_i32()?;
        }
        // sentCodeTypeFirebaseSms#9fd736 flags:# nonce:flags.0?bytes
        //   play_integrity_project_id:flags.2?long play_integrity_nonce:flags.2?bytes
        //   receipt:flags.1?string push_timeout:flags.1?int length:int
        SENT_CODE_TYPE_FIREBASE_SMS => {
            let flags = r.read_i32()?;
            if flags & (1 << 0) != 0 { let _ = r.read_bytes()?; }
            if flags & (1 << 1) != 0 { let _ = r.read_bytes()?; let _ = r.read_i32()?; }
            if flags & (1 << 2) != 0 { let _ = r.read_i64()?; let _ = r.read_bytes()?; }
            let _length = r.read_i32()?;
        }
        // sentCodeTypeSmsWord#a416ac81 flags:# beginning:flags.0?string
        // sentCodeTypeSmsPhrase#b37794af flags:# beginning:flags.0?string
        SENT_CODE_TYPE_SMS_WORD | SENT_CODE_TYPE_SMS_PHRASE => {
            let flags = r.read_i32()?;
            if flags & (1 << 0) != 0 { let _ = r.read_bytes()?; }
        }
        other => {
            return Err(Error::Serialization(format!(
                "unknown auth.SentCodeType constructor {other:#x}"
            )))
        }
    }
    Ok(ctor)
}

/// Skip an `auth.CodeType` object (bare constructor, no fields).
fn skip_code_type(r: &mut TLReader) -> Result<()> {
    r.read_u32()?;
    Ok(())
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
pub const SENT_CODE_TYPE_SMS: u32 = 0xc000bba2;
pub const SENT_CODE_TYPE_CALL: u32 = 0x5353e5a7;
pub const SENT_CODE_TYPE_FLASH_CALL: u32 = 0xab03c6d9;
pub const SENT_CODE_TYPE_MISSED_CALL: u32 = 0x82006484;
pub const SENT_CODE_TYPE_EMAIL_CODE: u32 = 0xf450f59b;
pub const SENT_CODE_TYPE_SET_UP_EMAIL_REQUIRED: u32 = 0xa5491dea;
pub const SENT_CODE_TYPE_FRAGMENT_SMS: u32 = 0xd9565c39;
pub const SENT_CODE_TYPE_FIREBASE_SMS: u32 = 0x9fd736;
pub const SENT_CODE_TYPE_SMS_WORD: u32 = 0xa416ac81;
pub const SENT_CODE_TYPE_SMS_PHRASE: u32 = 0xb37794af;


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
pub(crate) fn parse_login_token_response(plaintext: &[u8]) -> Result<AuthLoginToken> {
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
    // new_algo:PasswordKdfAlgo — consume the full object
    let new_algo_ctor = r.read_u32()?;
    if new_algo_ctor == types::PASSWORD_KDF_ALGO_SHA256_SHA256_PBKDF2_HMACSHA512_100K_MODPOW {
        let _salt1 = r.read_bytes()?;
        let _salt2 = r.read_bytes()?;
        let _g = r.read_i32()?;
        let _p = r.read_bytes()?;
    }
    // new_secure_algo:SecurePasswordKdfAlgo — consume per ctor
    let secure_ctor = r.read_u32()?;
    match secure_ctor {
        types::SECURE_PASSWORD_KDF_ALGO_PBKDF2 | types::SECURE_PASSWORD_KDF_ALGO_SHA512 => {
            let _salt = r.read_bytes()?;
        }
        _ => {}
    }
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
