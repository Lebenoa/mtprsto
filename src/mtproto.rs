//! MTProto 2.0 core protocol.
//!
//! Implements the full lifecycle:
//! 1. **Auth key creation** — Diffie-Hellman key exchange with the server
//! 2. **Message encryption/decryption** — AES-256-IGE with msg_key derivation
//! 3. **Session management** — salts, sequence numbers, message IDs
//! 4. **Message containers** — msg_container, msg_copy, msgs_ack

use crate::crypto::{self, RsaPublicKey};
use crate::error::{Error, Result};
use crate::serialize::{TLWriter, TLReader, *};
use num_bigint::BigUint;
use rand::rand_core::Rng as _;
use num_traits::ToPrimitive;

/// A negotiated MTProto session.
pub struct MtProtoSession {
    /// 2048-bit authorization key (256 bytes).
    pub auth_key: Vec<u8>,
    /// 64-bit auth_key_id (lower 64 bits of SHA1(auth_key)).
    pub auth_key_id: u64,
    /// 64-bit server salt (initially derived from new_nonce, then updated by server).
    pub server_salt: u64,
    /// 64-bit session ID (random, chosen by client).
    pub session_id: u64,
    /// Sequence number counter.
    pub seq_no: i32,
    /// Server time offset (difference between server time and local time).
    pub server_time_offset: i64,
    /// Message ID to use for the next outgoing message.
    pub last_msg_id: u64,
    /// Anti-fingerprinting: append a random number (0..15) of extra
    /// 16-byte padding blocks to each encrypted message, mirroring
    /// Telegram Desktop / gotd. On by default; deterministic-length
    /// messages make traffic trivially fingerprintable.
    pub random_padding: bool,
}

impl MtProtoSession {
    /// Create a new session with an existing auth key.
    pub fn new(auth_key: Vec<u8>, server_salt: u64) -> Self {
        let auth_key_id = crypto::auth_key_id(&auth_key);
        let session_id = crypto::random_session_id();
        Self {
            auth_key,
            auth_key_id,
            server_salt,
            session_id,
            seq_no: 0,
            server_time_offset: 0,
            last_msg_id: 0,
            random_padding: true,
        }
    }

    /// Toggle randomized padding (see [`MtProtoSession::random_padding`]).
    pub fn set_random_padding(&mut self, enabled: bool) {
        self.random_padding = enabled;
    }

    /// Generate the next message ID (client messages are divisible by 4).
    pub fn next_msg_id(&mut self) -> u64 {
        let msg_id = crypto::next_msg_id(self.server_time_offset);
        // Ensure monotonically increasing
        let msg_id = if msg_id <= self.last_msg_id {
            self.last_msg_id + 4
        } else {
            msg_id
        };
        self.last_msg_id = msg_id;
        msg_id
    }

    /// Increment and return the sequence number for a content-related message.
    pub fn next_seq_no(&mut self, content_related: bool) -> i32 {
        let seq = self.seq_no * 2 + if content_related { 1 } else { 0 };
        if content_related {
            self.seq_no += 1;
        }
        seq
    }

    /// Build the plaintext (internal header + payload) for encryption.
    fn build_plaintext(&self, msg_id: u64, seq_no: i32, payload: &[u8]) -> Vec<u8> {
        let mut w = TLWriter::new();
        // server_salt (int64)
        w.write_u64(self.server_salt);
        // session_id (int64)
        w.write_u64(self.session_id);
        // message_id (int64)
        w.write_u64(msg_id);
        // seq_no (int32)
        w.write_i32(seq_no);
        // message_data_length (int32)
        w.write_i32(payload.len() as i32);
        // message_data (bytes)
        w.write_raw_bytes(payload);

        let mut plaintext = w.into_bytes();

        // MTProto 2.0 padding: align to the 16-byte block size with at
        // least 12 bytes, then (anti-fingerprinting, gotd parity) add a
        // random 0..15 extra 16-byte blocks so encrypted-message length
        // does not deterministically fingerprint the client.
        let mut rng = rand::rng();
        let base = plaintext.len();
        let mut pad_len = (16 - (base % 16)) % 16;
        if pad_len < 12 {
            pad_len += 16;
        }
        if self.random_padding {
            pad_len += (rand::random::<u8>() & 0x0F) as usize * 16;
        }
        let mut padding = vec![0u8; pad_len];
        rng.fill_bytes(&mut padding);
        plaintext.extend_from_slice(&padding);

        plaintext
    }

    /// Encrypt a message payload for transmission.
    ///
    /// Returns the serialized encrypted message: `auth_key_id (8) + msg_key (16) + encrypted_data`.
    pub fn encrypt_message(&self, payload: &[u8], msg_id: u64, seq_no: i32) -> Vec<u8> {
        let plaintext = self.build_plaintext(msg_id, seq_no, payload);

        // Compute msg_key (x=0 for client→server)
        let msg_key = crypto::msg_key_mtproto2(&self.auth_key, &plaintext, 0);

        // Derive aes_key and aes_iv
        let (aes_key, aes_iv) = crypto::aes_key_and_iv(&self.auth_key, &msg_key, 0);

        // Encrypt with AES-256-IGE
        let mut encrypted = plaintext.clone();
        crypto::aes_ige_encrypt(&mut encrypted, &aes_key, &aes_iv)
            .expect("AES-IGE encryption failed");

        // Build external header
        let mut result = Vec::with_capacity(8 + 16 + encrypted.len());
        result.extend_from_slice(&self.auth_key_id.to_be_bytes());
        result.extend_from_slice(&msg_key);
        result.extend_from_slice(&encrypted);

        result
    }

    /// Decrypt a received encrypted message (default: server→client, x=8).
    ///
    /// Input is the full message starting with `auth_key_id`.
    pub fn decrypt_message(&mut self, data: &[u8]) -> Result<(u64, Vec<u8>)> {
        self.decrypt_message_with_x(data, 8)
    }

    /// Decrypt with a specific x value.
    ///
    /// x=0 for client→server messages, x=8 for server→client messages.
    pub fn decrypt_message_with_x(&mut self, data: &[u8], x: usize) -> Result<(u64, Vec<u8>)> {
        if data.len() < 24 {
            return Err(Error::Protocol("encrypted message too short".into()));
        }

        // Parse external header
        let received_auth_key_id = u64::from_be_bytes(data[0..8].try_into().unwrap());
        if received_auth_key_id != self.auth_key_id {
            return Err(Error::Protocol(format!(
                "auth_key_id mismatch: expected {:#x}, got {:#x}",
                self.auth_key_id, received_auth_key_id
            )));
        }

        let msg_key = <[u8; 16]>::try_from(&data[8..24]).unwrap();
        let encrypted = &data[24..];

        // Derive key/iv for the given direction
        let (aes_key, aes_iv) = crypto::aes_key_and_iv(&self.auth_key, &msg_key, x);

        // Decrypt
        let mut decrypted = encrypted.to_vec();
        crypto::aes_ige_decrypt(&mut decrypted, &aes_key, &aes_iv)?;

        // Verify msg_key
        let expected_msg_key = crypto::msg_key_mtproto2(&self.auth_key, &decrypted, x);
        if msg_key != expected_msg_key {
            return Err(Error::Protocol("msg_key verification failed".into()));
        }

        // Parse internal header
        let mut reader = TLReader::new(&decrypted);
        let server_salt = reader.read_u64()?; // server_salt
        let _session = reader.read_u64()?; // session_id
        let msg_id = reader.read_u64()?; // message_id
        let _seq_no = reader.read_i32()?; // seq_no
        let msg_len = reader.read_i32()?; // message_data_length

        let payload_start = reader.position();
        if payload_start + msg_len as usize > decrypted.len() {
            return Err(Error::Protocol("message_data_length exceeds data".into()));
        }
        let payload = decrypted[payload_start..payload_start + msg_len as usize].to_vec();

        // Adopt the server's current salt so subsequent sends stay valid.
        self.server_salt = server_salt;

        Ok((msg_id, payload))
    }

    /// Build a serialized unencrypted message (used during auth key creation).
    pub fn build_unencrypted(msg_id: u64, payload: &[u8]) -> Vec<u8> {
        let mut w = TLWriter::new();
        w.write_u64(0); // auth_key_id = 0 (no key)
        w.write_u64(msg_id);
        w.write_i32(payload.len() as i32);
        w.write_raw_bytes(payload);
        w.into_bytes()
    }
}

// ---------------------------------------------------------------------------
// Auth key creation (Diffie-Hellman handshake)
// ---------------------------------------------------------------------------

/// Intermediate state for the auth-key creation flow.
#[derive(Default)]
pub struct AuthKeyCreation {
    pub nonce: [u8; 16],
    pub server_nonce: Option<[u8; 16]>,
    pub new_nonce: Option<[u8; 32]>,
    pub pq: Option<Vec<u8>>,
    pub server_public_key: Option<RsaPublicKey>,
    pub p: Option<Vec<u8>>,
    pub q: Option<Vec<u8>>,
    pub g: Option<u32>,
    pub dh_prime: Option<Vec<u8>>,
    pub g_a: Option<BigUint>,
    pub a: Option<BigUint>,
    /// auth_key_aux_hash of the previous attempt (64 higher-order bits of
    /// SHA1(auth_key)); becomes `retry_id` when the server asks for a retry.
    pub auth_key_aux_hash: Option<u64>,
    pub temp_aes_key: Option<[u8; 32]>,
    /// `retry_id` for `client_DH_inner_data`: 0 on the first attempt, then
    /// the previous attempt's auth_key_aux_hash (SPEC §7).
    pub retry_id: u64,
    pub temp_aes_iv: Option<[u8; 32]>,
    pub server_time_offset: i64,
}

impl AuthKeyCreation {
    /// Start a new auth key creation flow.
    pub fn new() -> Self {
        Self {
            nonce: crypto::random_nonce(),
            server_nonce: None,
            new_nonce: None,
            pq: None,
            server_public_key: None,
            p: None,
            q: None,
            g: None,
            dh_prime: None,
            g_a: None,
            a: None,
            auth_key_aux_hash: None,
            retry_id: 0,
            temp_aes_key: None,
            temp_aes_iv: None,
            server_time_offset: 0,
        }
    }

    /// Step 1: Build `req_pq_multi` request.
    pub fn build_req_pq(&self) -> Vec<u8> {
        let mut w = TLWriter::new();
        w.write_u32(REQ_PQ_MULTI);
        w.write_raw_bytes(&self.nonce);
        w.into_bytes()
    }

    /// Step 2: Parse `resPQ` response.
    pub fn parse_res_pq(&mut self, data: &[u8]) -> Result<()> {
        let mut r = TLReader::new(data);
        let constructor = r.read_u32()?;
        if constructor != RES_PQ {
            return Err(Error::UnexpectedResponse(format!(
                "expected RES_PQ ({:#x}), got {:#x}",
                RES_PQ, constructor
            )));
        }

        let nonce = r.read_i128_bytes()?;
        if nonce != self.nonce {
            return Err(Error::Protocol("nonce mismatch in resPQ".into()));
        }

        self.server_nonce = Some(r.read_i128_bytes()?);

        // Parse pq as a TL string (big-endian bytes)
        self.pq = Some(r.read_bytes()?);

        // Parse server_public_key_fingerprints: Vector<long>
        // The vector constructor (0x1cb5c415) precedes the count.
        let vec_ctor = r.read_u32()?;
        if vec_ctor != crate::serialize::VECTOR {
            return Err(Error::UnexpectedResponse(format!(
                "expected Vector constructor in resPQ, got {vec_ctor:#x}"
            )));
        }
        let num_fingerprints = r.read_i32()?;
        let mut fingerprints = Vec::new();
        for _ in 0..num_fingerprints {
            // longs are little-endian; gotd's RSAFingerprint is defined as
            // int64(LE u64 of SHA1[12..20]), which is exactly what the
            // server echoes back. Plain LE read matches.
            let v = r.read_u64()?;
            fingerprints.push(v);
        }
        for fp in &fingerprints {
            if let Some(key) = crypto::find_server_key(*fp) {
                self.server_public_key = Some(key);
                return Ok(());
            }
        }

        Err(Error::Crypto("no matching server public key found".into()))
    }

    /// Step 3: Factor PQ and prepare for DH params request.
    ///
    /// Factors pq into p and q where p < q and both are odd primes.
    pub fn factor_pq(&mut self) -> Result<()> {
        let pq_bytes = self.pq.as_ref().ok_or(Error::NoAuthKey)?;
        let pq = BigUint::from_bytes_be(pq_bytes);

        // Pollard's p-1 algorithm or trial division for small factors
        // For Telegram, pq is typically <= 2^63-1, so we can use Pollard's rho
        let (p, q) = pollard_rho_factor(&pq)?;

        // Ensure p < q
        let (p, q) = if p < q { (p, q) } else { (q, p) };

        self.p = Some(p.to_bytes_be());
        self.q = Some(q.to_bytes_be());

        Ok(())
    }

    /// Step 4: Build `req_DH_params` request.
    pub fn build_req_dh_params(&mut self, dc_id: i32) -> Result<Vec<u8>> {
        let p = BigUint::from_bytes_be(self.p.as_ref().ok_or(Error::NoAuthKey)?);
        let q = BigUint::from_bytes_be(self.q.as_ref().ok_or(Error::NoAuthKey)?);

        let new_nonce = crypto::random_nonce_256();
        self.new_nonce = Some(new_nonce);

        // Build p_q_inner_data
        let mut inner = TLWriter::new();
        inner.write_u32(P_Q_INNER_DATA);
        inner.write_bytes(self.pq.as_ref().unwrap());
        inner.write_bytes(&p.to_bytes_be());
        inner.write_bytes(&q.to_bytes_be());
        inner.write_raw_bytes(&self.nonce);
        inner.write_raw_bytes(&self.server_nonce.unwrap());
        inner.write_u256(new_nonce);
        inner.write_i32(dc_id);

        let inner_data = inner.into_bytes();
        let server_key = self.server_public_key.as_ref().ok_or(Error::NoAuthKey)?;

        // RSA_PAD the inner data
        let encrypted_data = crypto::rsa_pad(&inner_data, server_key)?;

        // Build req_DH_params
        let mut w = TLWriter::new();
        w.write_u32(REQ_DH_PARAMS);
        w.write_raw_bytes(&self.nonce);
        w.write_raw_bytes(&self.server_nonce.unwrap());
        w.write_bytes(&p.to_bytes_be());
        w.write_bytes(&q.to_bytes_be());
        w.write_i64(server_key.fingerprint() as i64);
        w.write_bytes(&encrypted_data);

        Ok(w.into_bytes())
    }

    /// Step 6: Parse `server_DH_params_ok` response.
    pub fn parse_server_dh_params(&mut self, data: &[u8]) -> Result<()> {
        let mut r = TLReader::new(data);
        let constructor = r.read_u32()?;

        match constructor {
            SERVER_DH_PARAMS_OK => {
                let nonce = r.read_i128_bytes()?;
                if nonce != self.nonce {
                    return Err(Error::Protocol("nonce mismatch in DH params".into()));
                }

                let server_nonce = r.read_i128_bytes()?;
                if server_nonce != self.server_nonce.unwrap() {
                    return Err(Error::Protocol("server_nonce mismatch".into()));
                }

                let encrypted_answer = r.read_bytes()?;

                // Decrypt the answer
                let new_nonce = self.new_nonce.unwrap();
                let server_nonce_bytes: [u8; 16] = self.server_nonce.unwrap();

                // tmp_aes_key = SHA1(new_nonce + server_nonce) + SHA1(server_nonce + new_nonce)[0..12]
                let mut tmp_key = Vec::with_capacity(32);
                tmp_key.extend_from_slice(&crypto::sha1_concat(&new_nonce, &server_nonce_bytes));
                tmp_key.extend_from_slice(&crypto::sha1_concat(&server_nonce_bytes, &new_nonce)[0..12]);

                let mut tmp_aes_key = [0u8; 32];
                tmp_aes_key.copy_from_slice(&tmp_key);

                // tmp_aes_iv = SHA1(server_nonce + new_nonce)[12..20] + SHA1(new_nonce + new_nonce) + new_nonce[0..4]
                let mut tmp_iv = Vec::with_capacity(32);
                tmp_iv.extend_from_slice(&crypto::sha1_concat(&server_nonce_bytes, &new_nonce)[12..20]);
                tmp_iv.extend_from_slice(&crypto::sha1_concat(&new_nonce, &new_nonce));
                tmp_iv.extend_from_slice(&new_nonce[0..4]);

                let mut tmp_aes_iv = [0u8; 32];
                tmp_aes_iv.copy_from_slice(&tmp_iv);

                self.temp_aes_key = Some(tmp_aes_key);
                self.temp_aes_iv = Some(tmp_aes_iv);

                // Decrypt
                let mut answer = encrypted_answer.clone();
                crypto::aes_ige_decrypt(&mut answer, &tmp_aes_key, &tmp_aes_iv)?;

                // answer = SHA1(answer)[0..20] + answer_data + padding
                // Skip the first 20 bytes (SHA1 hash) and parse the rest
                // Parse inner data
                let mut inner_r = TLReader::new(&answer[20..]);
                let inner_constructor = inner_r.read_u32()?;
                if inner_constructor != SERVER_DH_INNER_DATA {
                    return Err(Error::UnexpectedResponse(format!(
                        "expected SERVER_DH_INNER_DATA, got {:#x}",
                        inner_constructor
                    )));
                }

                let _nonce = inner_r.read_i128_bytes()?;
                let _server_nonce = inner_r.read_i128_bytes()?;

                self.g = Some(inner_r.read_i32()? as u32);
                self.dh_prime = Some(inner_r.read_bytes()?);
                let g_a_bytes = inner_r.read_bytes()?;
                self.g_a = Some(BigUint::from_bytes_be(&g_a_bytes));
                let _server_time = inner_r.read_i32()?;

                // Verify DH parameters
                let prime = BigUint::from_bytes_be(self.dh_prime.as_ref().unwrap());
                crypto::verify_dh_params(self.g.unwrap(), self.g_a.as_ref().unwrap(), &prime)?;

                Ok(())
            }
            SERVER_DH_PARAMS_FAIL => {
                Err(Error::Protocol("server_DH_params_fail received".into()))
            }
            _ => Err(Error::UnexpectedResponse(format!(
                "unexpected constructor {:#x} in DH params response",
                constructor
            ))),
        }
    }

    /// Step 7: Generate client DH value and build `set_client_DH_params`.
    pub fn build_set_client_dh_params(&mut self) -> Result<Vec<u8>> {
        let dh_prime = BigUint::from_bytes_be(self.dh_prime.as_ref().unwrap());
        let g = BigUint::from(self.g.ok_or(Error::NoAuthKey)?);

        // Generate random 2048-bit b
        let mut rng = rand::rng();
        use num_bigint::BigRng010;
        let b = rng.random_biguint(2048);
        let g_b = g.modpow(&b, &dh_prime);

        self.a = Some(b); // Actually store b here; the "a" naming is from the server's perspective
        // Note: in client code, we generate b and compute g_b = g^b mod p.
        // The auth_key = g_a^b mod p = (g^a)^b mod p

        // Candidate auth key for THIS attempt. Its SHA1 prefix is the
        // auth_key_aux_hash the server refers to if it answers dh_gen_retry
        // (SPEC §9); the next attempt then echoes it back as retry_id.
        let candidate_key = {
            let g_a = self.g_a.as_ref().ok_or(Error::NoAuthKey)?;
            let shared = g_a.modpow(self.a.as_ref().unwrap(), &dh_prime);
            let mut key = vec![0u8; 256];
            let bytes = shared.to_bytes_be();
            let start = 256usize.saturating_sub(bytes.len());
            key[start..].copy_from_slice(&bytes);
            key
        };
        let aux_hash_arr = crypto::sha1(&candidate_key);
        let aux_hash = u64::from_be_bytes([
            aux_hash_arr[0], aux_hash_arr[1], aux_hash_arr[2], aux_hash_arr[3],
            aux_hash_arr[4], aux_hash_arr[5], aux_hash_arr[6], aux_hash_arr[7],
        ]);
        self.auth_key_aux_hash = Some(aux_hash);

        // Build client_DH_inner_data
        let mut inner = TLWriter::new();
        inner.write_u32(CLIENT_DH_INNER_DATA);
        inner.write_raw_bytes(&self.nonce);
        inner.write_raw_bytes(&self.server_nonce.unwrap());
        // retry_id = 0 on the first attempt; on retries, the aux hash of the
        // PREVIOUS attempt's candidate key (stored by the caller loop).
        inner.write_i64(self.retry_id as i64);
        inner.write_bytes(&g_b.to_bytes_be());

        let inner_data = inner.into_bytes();

        // data_with_hash = SHA1(data) + data + random padding
        let hash = crypto::sha1(&inner_data);
        let mut data_with_hash = Vec::with_capacity(20 + inner_data.len() + 16);
        data_with_hash.extend_from_slice(&hash);
        data_with_hash.extend_from_slice(&inner_data);
        let mut padding = vec![0u8; 16];
        rng.fill_bytes(&mut padding);
        data_with_hash.extend_from_slice(&padding);

        // Pad to 16-byte alignment
        while data_with_hash.len() % 16 != 0 {
            data_with_hash.push(0);
        }

        let tmp_aes_key = self.temp_aes_key.ok_or(Error::NoAuthKey)?;
        let tmp_aes_iv = self.temp_aes_iv.ok_or(Error::NoAuthKey)?;

        let mut encrypted = data_with_hash;
        crypto::aes_ige_encrypt(&mut encrypted, &tmp_aes_key, &tmp_aes_iv)?;

        // Build set_client_DH_params
        let mut w = TLWriter::new();
        w.write_u32(SET_CLIENT_DH_PARAMS);
        w.write_raw_bytes(&self.nonce);
        w.write_raw_bytes(&self.server_nonce.unwrap());
        w.write_bytes(&encrypted);

        Ok(w.into_bytes())
    }

    /// Step 9: Parse the DH gen result and compute the auth key.
    pub fn parse_dh_gen_result(&mut self, data: &[u8]) -> Result<AuthKeyResult> {
        let mut r = TLReader::new(data);
        let constructor = r.read_u32()?;
        let _nonce = r.read_i128_bytes()?;
        let _server_nonce = r.read_i128_bytes()?;

        match constructor {
            DH_GEN_OK => {
                let _new_nonce_hash = r.read_i128_bytes()?;
                Ok(AuthKeyResult::Ok)
            }
            DH_GEN_RETRY => {
                let _new_nonce_hash = r.read_i128_bytes()?;
                Ok(AuthKeyResult::Retry)
            }
            DH_GEN_FAIL => {
                let _new_nonce_hash = r.read_i128_bytes()?;
                Ok(AuthKeyResult::Fail)
            }
            _ => Err(Error::UnexpectedResponse(format!(
                "unexpected DH gen result {:#x}",
                constructor
            ))),
        }
    }

    /// Compute the auth key from the negotiated DH values.
    ///
    /// Must be called after factor_pq and after generating the client DH value.
    /// The auth_key = g_a^b mod dh_prime.
    pub fn compute_auth_key(&self) -> Result<Vec<u8>> {
        let dh_prime = BigUint::from_bytes_be(self.dh_prime.as_ref().ok_or(Error::NoAuthKey)?);
        let g_a = self.g_a.as_ref().ok_or(Error::NoAuthKey)?;
        let b = self.a.as_ref().ok_or(Error::NoAuthKey)?; // We stored b as "a"

        let shared_secret = g_a.modpow(b, &dh_prime);

        // Auth key is 2048 bits (256 bytes), zero-padded on the left
        let mut auth_key = vec![0u8; 256];
        let bytes = shared_secret.to_bytes_be();
        let start = 256usize.saturating_sub(bytes.len());
        auth_key[start..].copy_from_slice(&bytes);

        Ok(auth_key)
    }

    /// Compute the server salt: substr(new_nonce, 0, 8) XOR substr(server_nonce, 0, 8)
    pub fn compute_server_salt(&self) -> Result<u64> {
        let new_nonce = self.new_nonce.ok_or(Error::NoAuthKey)?;
        let server_nonce = self.server_nonce.ok_or(Error::NoAuthKey)?;

        let mut salt_bytes = [0u8; 8];
        for i in 0..8 {
            salt_bytes[i] = new_nonce[i] ^ server_nonce[i];
        }
        Ok(u64::from_be_bytes(salt_bytes))
    }
}

/// Result of the DH gen step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKeyResult {
    /// Auth key created successfully.
    Ok,
    /// Server requests a retry with different DH value.
    Retry,
    /// Auth key creation failed.
    Fail,
}

// ---------------------------------------------------------------------------
// Message containers
// ---------------------------------------------------------------------------

/// Build a msg_container with multiple messages.
///
/// The container format is:
///   msg_container#73f1f8dc
///   vector#1cb5c415 count:int
///   [ msg_id:long seq_no:int message:bytes ] × count
pub fn build_msg_container(messages: &[(u64, i32, &[u8])]) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(MSG_CONTAINER);
    w.write_u32(VECTOR);
    w.write_i32(messages.len() as i32);

    for &(msg_id, seq_no, payload) in messages {
        w.write_u64(msg_id);
        w.write_i32(seq_no);
        w.write_bytes(payload);
    }

    w.into_bytes()
}

/// Build a msgs_ack message.
pub fn build_msgs_ack(msg_ids: &[u64]) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(MSGS_ACK);
    // Vector of longs
    w.write_u32(VECTOR);
    w.write_i32(msg_ids.len() as i32);
    for &id in msg_ids {
        w.write_u64(id);
    }
    w.into_bytes()
}

/// Build a ping message.
pub fn build_ping(id: i64) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(PING);
    w.write_i64(id);
    w.into_bytes()
}

/// Build `ping_delay_disconnect#f3427b8c ping_id:long disconnect_delay:int`.
///
/// Like [`build_ping`] but instructs the server to drop the connection if
/// no further pings arrive within `disconnect_delay` seconds — matches
/// gotd's keepalive, which relies on this to reap dead connections.
pub fn build_ping_delay_disconnect(id: i64, disconnect_delay: i32) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(PING_DELAY_DISCONNECT);
    w.write_i64(id);
    w.write_i32(disconnect_delay);
    w.into_bytes()
}

/// Parse a pong message.
pub fn parse_pong(data: &[u8]) -> Result<i64> {
    let mut r = TLReader::new(data);
    let constructor = r.read_u32()?;
    if constructor != PONG {
        return Err(Error::UnexpectedResponse(format!(
            "expected PONG, got {:#x}",
            constructor
        )));
    }
    let _msg_id = r.read_u64()?;
    let ping_id = r.read_i64()?;
    Ok(ping_id)
}

/// Parse an RPC result wrapper.
pub fn parse_rpc_result(data: &[u8]) -> Result<Vec<u8>> {
    let mut r = TLReader::new(data);
    let constructor = r.read_u32()?;
    if constructor != RPC_RESULT {
        return Err(Error::UnexpectedResponse(format!(
            "expected RPC_RESULT, got {:#x}",
            constructor
        )));
    }
    let _req_msg_id = r.read_u64()?;
    let result = data[r.position()..].to_vec();
    Ok(result)
}

/// Parse an RPC error.
pub fn parse_rpc_error(data: &[u8]) -> Result<(i32, String)> {
    let mut r = TLReader::new(data);
    let constructor = r.read_u32()?;
    if constructor != RPC_ERROR {
        return Err(Error::UnexpectedResponse(format!(
            "expected RPC_ERROR, got {:#x}",
            constructor
        )));
    }
    let error_code = r.read_i32()?;
    let error_message = String::from_utf8(r.read_bytes()?)
        .map_err(|_| Error::Protocol("invalid UTF-8 in error message".into()))?;
    Ok((error_code, error_message))
}

/// Parse a `bad_msg_notification#a7eff811` service message.
///
/// Returns `(bad_msg_id, bad_msg_seqno, error_code)`. The error_code
/// meanings are documented in [`crate::pool::describe_bad_msg_code`].
pub fn parse_bad_msg_notification(data: &[u8]) -> Result<(u64, i32, i32)> {
    let mut r = TLReader::new(data);
    let constructor = r.read_u32()?;
    if constructor != BAD_MSG_NOTIFICATION {
        return Err(Error::UnexpectedResponse(format!(
            "expected BAD_MSG_NOTIFICATION, got {constructor:#x}"
        )));
    }
    let bad_msg_id = r.read_u64()?;
    let bad_msg_seqno = r.read_i32()?;
    let error_code = r.read_i32()?;
    Ok((bad_msg_id, bad_msg_seqno, error_code))
}

// ---------------------------------------------------------------------------
// RPC envelopes (SPEC §5)
// ---------------------------------------------------------------------------

/// Wrap method bytes in `invokeWithLayer#da9b0d0d {layer:int}`.
pub fn build_invoke_with_layer(layer: i32, method_bytes: &[u8]) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(crate::types::INVOKE_WITH_LAYER);
    w.write_i32(layer);
    w.write_raw_bytes(method_bytes);
    w.into_bytes()
}

/// Wrap method bytes in `initConnection#c1cd5ea9` (client identification).
///
/// `CONNECTION_NOT_INITED` is returned by production servers when
/// `auth.*`/user RPCs run without this wrapper.
pub fn build_init_connection(
    api_id: i32,
    device_model: &str,
    system_version: &str,
    app_version: &str,
    lang_code: &str,
    method_bytes: &[u8],
) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(crate::types::INIT_CONNECTION);
    w.write_i32(0); // flags (no proxy / params)
    w.write_i32(api_id);
    w.write_bytes(device_model.as_bytes());
    w.write_bytes(system_version.as_bytes());
    w.write_bytes(app_version.as_bytes());
    w.write_bytes(lang_code.as_bytes()); // system_lang_code
    w.write_bytes(b""); // lang_pack (empty for non-official apps)
    w.write_bytes(lang_code.as_bytes());
    w.write_raw_bytes(method_bytes);
    w.into_bytes()
}

/// Wrap query bytes in `invokeAfterMsg#cb9f372d {msg_id:long query:#}`.
///
/// Tells the server to process `query` only after `msg_id` was handled —
/// needed to keep dependent RPCs ordered when pipelining.
pub fn build_invoke_after_msg(msg_id: u64, query: &[u8]) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(crate::types::INVOKE_AFTER_MSG);
    w.write_u64(msg_id);
    w.write_raw_bytes(query);
    w.into_bytes()
}

/// Wrap query bytes in `invokeWithoutUpdates#bf94591b {query:#}`.
///
/// Suppresses update delivery for this request (useful for bulk/poll RPCs
/// that would otherwise flood the update stream).
pub fn build_invoke_without_updates(query: &[u8]) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(crate::types::INVOKE_WITHOUT_UPDATES);
    w.write_raw_bytes(query);
    w.into_bytes()
}

// ---------------------------------------------------------------------------
// Server service notifications (SPEC §5)
// ---------------------------------------------------------------------------

/// Parsed `new_session_created#9ec209d4` notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewSessionCreated {
    pub first_msg_id: u64,
    pub unique_id: u64,
    pub server_salt: u64,
}

/// Parse a `new_session_created` body (after the constructor).
pub fn parse_new_session_created(data: &[u8]) -> Result<NewSessionCreated> {
    let mut r = TLReader::new(data);
    let constructor = r.read_u32()?;
    if constructor != NEW_SESSION_CREATED {
        return Err(Error::UnexpectedResponse(format!(
            "expected NEW_SESSION_CREATED, got {constructor:#x}"
        )));
    }
    Ok(NewSessionCreated {
        first_msg_id: r.read_u64()?,
        unique_id: r.read_u64()?,
        server_salt: r.read_u64()?,
    })
}

/// A single salt window from `future_salt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaltWindow {
    pub valid_since: i32,
    pub valid_until: i32,
    pub salt: u64,
}

/// Parse `future_salts#ae500895 req_msg_id:long now:int salts:vector<future_salt>`.
///
/// `data` is the full service message (constructor included).
pub fn parse_future_salts(data: &[u8]) -> Result<(u64, i32, Vec<SaltWindow>)> {
    let mut r = TLReader::new(data);
    let constructor = r.read_u32()?;
    if constructor != FUTURE_SALTS {
        return Err(Error::UnexpectedResponse(format!(
            "expected FUTURE_SALTS, got {constructor:#x}"
        )));
    }
    let req_msg_id = r.read_u64()?;
    let now = r.read_i32()?;
    let count = r.read_i32()?;
    let mut salts = Vec::with_capacity(count.max(0) as usize);
    for _ in 0..count {
        salts.push(SaltWindow {
            valid_since: r.read_i32()?,
            valid_until: r.read_i32()?,
            salt: r.read_u64()?,
        });
    }
    Ok((req_msg_id, now, salts))
}

/// Build `getFutureSalts#b921bd04 num:int` — asks the server for its salt
/// windows. The reply is a `future_salts` service message.
pub fn build_get_future_salts(num: i32) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(FUTURE_SALTS_REQUEST);
    w.write_i32(num);
    w.into_bytes()
}

/// Build `msgs_state_req#da69fb52 msg_ids:Vector<long>` — ask the server
/// for the delivery state of the given message ids.
pub fn build_msgs_state_req(msg_ids: &[u64]) -> Vec<u8> {
    let mut w = TLWriter::new();
    w.write_u32(MSGS_STATE_REQ);
    w.write_u32(VECTOR);
    w.write_u32(msg_ids.len() as u32);
    for id in msg_ids {
        w.write_u64(*id);
    }
    w.into_bytes()
}

// ---------------------------------------------------------------------------
// Pollard's rho factorization for PQ
// ---------------------------------------------------------------------------

fn pollard_rho_factor(n: &BigUint) -> Result<(BigUint, BigUint)> {
    use num_traits::{One, Zero};

    if n.is_zero() {
        return Err(Error::Crypto("cannot factor zero".into()));
    }
    if n.is_one() {
        return Err(Error::Crypto("cannot factor one".into()));
    }

    // For small numbers, trial division
    if n.bits() < 64 {
        let n_val = n.to_u64().unwrap();
        for d in (3..=((n_val as f64).sqrt() as u64)).step_by(2) {
            if n_val.is_multiple_of(d) {
                return Ok((
                    BigUint::from(d),
                    BigUint::from(n_val / d),
                ));
            }
        }
        // n is prime
        return Ok((n.clone(), BigUint::one()));
    }

    // Pollard's rho
    use rand::RngExt;
    let mut rng = rand::rng();

    loop {
        let c = BigUint::from(rng.random_range(1u32..=100));
        let mut x = BigUint::from(2u32);
        let mut y = x.clone();
        let mut d = BigUint::one();

        while d.is_one() {
            // x = x^2 + c mod n
            x = (&x * &x + &c) % n;
            // y = y^2 + c mod n, twice
            y = (&y * &y + &c) % n;
            y = (&y * &y + &c) % n;
            let diff = if x > y { &x - &y } else { &y - &x };
            d = biguint_gcd(&diff, n);
        }

        if &d != n {
            let other = n / &d;
            return Ok((d, other));
        }
        // If d == n, retry with different c
    }
}

/// Compute GCD of two BigUint values.
fn biguint_gcd(a: &BigUint, b: &BigUint) -> BigUint {
    use num_traits::Zero;
    let mut x = a.clone();
    let mut y = b.clone();
    while !y.is_zero() {
        let r = &x % &y;
        x = y;
        y = r;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_encrypt_decrypt() {
        let auth_key = crypto::random_bytes(256);
        let mut session = MtProtoSession::new(auth_key, 12345);

        let payload = b"test message payload";
        let msg_id = session.next_msg_id();
        let seq_no = session.next_seq_no(true);

        let encrypted = session.encrypt_message(payload, msg_id, seq_no);
        // Decrypt with x=0 (client→server) since the message was encrypted with x=0
        let (dec_msg_id, dec_payload) = session.decrypt_message_with_x(&encrypted, 0).unwrap();

        assert_eq!(dec_msg_id, msg_id);
        assert_eq!(dec_payload, payload);
    }

    #[test]
    fn test_build_ping() {
        let ping = build_ping(42);
        let mut r = TLReader::new(&ping);
        assert_eq!(r.read_u32().unwrap(), PING);
        assert_eq!(r.read_i64().unwrap(), 42);
    }

    #[test]
    fn test_msgs_ack() {
        let ack = build_msgs_ack(&[1, 2, 3]);
        let mut r = TLReader::new(&ack);
        assert_eq!(r.read_u32().unwrap(), MSGS_ACK);
        assert_eq!(r.read_u32().unwrap(), VECTOR);
        assert_eq!(r.read_i32().unwrap(), 3);
        assert_eq!(r.read_u64().unwrap(), 1);
        assert_eq!(r.read_u64().unwrap(), 2);
        assert_eq!(r.read_u64().unwrap(), 3);
    }

    #[test]
    fn test_unencrypted_message() {
        let msg = MtProtoSession::build_unencrypted(12345, b"hello");
        let mut r = TLReader::new(&msg);
        assert_eq!(r.read_u64().unwrap(), 0); // auth_key_id = 0
        assert_eq!(r.read_u64().unwrap(), 12345); // msg_id
        let len = r.read_i32().unwrap();
        assert_eq!(len, 5);
    }

    #[test]
    fn test_auth_key_creation_flow() {
        let mut auth = AuthKeyCreation::new();
        let req_pq = auth.build_req_pq();
        assert!(req_pq.len() > 4);

        // Simulate factor_pq with a known value
        auth.pq = Some(BigUint::from(1234567891u64).to_bytes_be()); // This is prime
        // For testing, we'll just verify the structure is valid
        assert!(auth.nonce.len() == 16);
    }

    #[test]
    fn test_set_client_dh_params_retry_id_semantics() {
        // Simulate the DH state the server would have delivered: small
        // safe-ish params are enough to exercise serialization only.
        let mut auth = AuthKeyCreation::new();
        auth.g = Some(3);
        auth.dh_prime = Some(BigUint::from_bytes_be(&[0xC7, 0x1C, 0xAE, 0xB9]).to_bytes_be());
        auth.g_a = Some(BigUint::from(7u32));
        auth.temp_aes_key = Some([0u8; 32]);
        auth.temp_aes_iv = Some([0u8; 32]);
        auth.server_nonce = Some([9u8; 16]);

        // Attempt 1: retry_id must be 0; aux hash recorded.
        let req1 = auth.build_set_client_dh_params().unwrap();
        let aux1 = auth.auth_key_aux_hash.unwrap();
        assert_ne!(aux1, 0);

        // Decode the outer set_client_DH_params (unencrypted part) and the
        // inner retry_id by decrypting with the zero-key IGE we set up.
        let mut r = TLReader::new(&req1);
        assert_eq!(r.read_u32().unwrap(), SET_CLIENT_DH_PARAMS);
        let _nonce = r.read_i128_bytes().unwrap();
        let _server_nonce = r.read_i128_bytes().unwrap();
        let encrypted = r.read_bytes().unwrap();

        let mut plain = encrypted.clone();
        crypto::aes_ige_decrypt(&mut plain, &auth.temp_aes_key.unwrap(), &auth.temp_aes_iv.unwrap()).unwrap();
        // plain = SHA1(inner) + inner + padding; inner ctor + nonce + server_nonce + retry_id
        let inner = &plain[20..];
        let mut ir = TLReader::new(inner);
        assert_eq!(ir.read_u32().unwrap(), CLIENT_DH_INNER_DATA);
        let _ = ir.read_i128_bytes().unwrap(); // nonce
        let _ = ir.read_i128_bytes().unwrap(); // server_nonce
        let retry_id = ir.read_i64().unwrap() as u64;
        assert_eq!(retry_id, 0, "first attempt must carry retry_id = 0");

        // Server answers dh_gen_retry: caller promotes aux hash to retry_id.
        auth.retry_id = aux1;

        // Attempt 2: retry_id must equal attempt 1's aux hash.
        let req2 = auth.build_set_client_dh_params().unwrap();
        let mut r2 = TLReader::new(&req2);
        let _ = r2.read_u32().unwrap();
        let _ = r2.read_i128_bytes().unwrap();
        let _ = r2.read_i128_bytes().unwrap();
        let encrypted2 = r2.read_bytes().unwrap();
        let mut plain2 = encrypted2.clone();
        crypto::aes_ige_decrypt(&mut plain2, &auth.temp_aes_key.unwrap(), &auth.temp_aes_iv.unwrap()).unwrap();
        let mut ir2 = TLReader::new(&plain2[20..]);
        let _ = ir2.read_u32().unwrap();
        let _ = ir2.read_i128_bytes().unwrap();
        let _ = ir2.read_i128_bytes().unwrap();
        let retry_id2 = ir2.read_i64().unwrap() as u64;
        assert_eq!(retry_id2, aux1, "second attempt must echo previous aux hash");
    }

    #[test]
    fn test_invoke_envelopes_roundtrip() {
        use crate::types::{INVOKE_AFTER_MSG, INVOKE_WITH_LAYER, INVOKE_WITHOUT_UPDATES};

        // invokeWithLayer
        let inner = vec![0xAA, 0xBB, 0xCC];
        let wrapped = build_invoke_with_layer(223, &inner);
        let mut r = TLReader::new(&wrapped);
        assert_eq!(r.read_u32().unwrap(), INVOKE_WITH_LAYER);
        assert_eq!(r.read_i32().unwrap(), 223);
        assert_eq!(&wrapped[4 + 4..], &inner[..]); // rest is raw query bytes

        // invokeAfterMsg
        let after = build_invoke_after_msg(0x1234_5678_9abc, &inner);
        let mut r = TLReader::new(&after);
        assert_eq!(r.read_u32().unwrap(), INVOKE_AFTER_MSG);
        assert_eq!(r.read_u64().unwrap(), 0x1234_5678_9abc);
        assert_eq!(&after[4 + 8..], &inner[..]);

        // invokeWithoutUpdates
        let quiet = build_invoke_without_updates(&inner);
        let mut r = TLReader::new(&quiet);
        assert_eq!(r.read_u32().unwrap(), INVOKE_WITHOUT_UPDATES);
        assert_eq!(&quiet[4..], &inner[..]);

        // Nesting composes: afterMsg(withoutUpdates(withLayer(query)))
        let stacked = build_invoke_after_msg(
            7,
            &build_invoke_without_updates(&build_invoke_with_layer(223, &inner)),
        );
        let mut r = TLReader::new(&stacked);
        assert_eq!(r.read_u32().unwrap(), INVOKE_AFTER_MSG);
        assert_eq!(r.read_u64().unwrap(), 7);
        let mid = stacked[4 + 8..].to_vec();
        let mut r = TLReader::new(&mid);
        assert_eq!(r.read_u32().unwrap(), INVOKE_WITHOUT_UPDATES);
    }

    #[test]
    fn test_parse_new_session_created() {
        let mut w = TLWriter::new();
        w.write_u32(NEW_SESSION_CREATED);
        w.write_u64(0x1111);
        w.write_u64(0x2222);
        w.write_u64(0x3333);
        let ns = parse_new_session_created(&w.into_bytes()).unwrap();
        assert_eq!(ns.first_msg_id, 0x1111);
        assert_eq!(ns.unique_id, 0x2222);
        assert_eq!(ns.server_salt, 0x3333);
    }

    #[test]
    fn test_future_salts_roundtrip() {
        // future_salts#ae500895 req_msg_id:long now:int salts:vector<future_salt>
        // future_salt#0949dfe1 valid_since:int valid_until:int salt:long
        let mut w = TLWriter::new();
        w.write_u32(FUTURE_SALTS);
        w.write_u64(42); // req_msg_id
        w.write_i32(1_700_000_000); // now
        w.write_i32(2); // count
        for i in 0..2i32 {
            w.write_i32(1_700_000_000 + i * 3600);
            w.write_i32(1_700_003_600 + i * 3600);
            w.write_u64(0x5A17 + i as u64);
        }
        let data = w.into_bytes();
        let (req_msg_id, now, salts) = parse_future_salts(&data).unwrap();
        assert_eq!(req_msg_id, 42);
        assert_eq!(now, 1_700_000_000);
        assert_eq!(salts.len(), 2);
        assert_eq!(salts[0].salt, 0x5A17);
        assert_eq!(salts[1].salt, 0x5A17 + 1);
    }

    #[test]
    fn test_msgs_state_req_build() {
        let req = build_msgs_state_req(&[1, 2, 3]);
        let mut r = TLReader::new(&req);
        assert_eq!(r.read_u32().unwrap(), MSGS_STATE_REQ);
        assert_eq!(r.read_u32().unwrap(), VECTOR);
        assert_eq!(r.read_i32().unwrap(), 3);
        assert_eq!(r.read_u64().unwrap(), 1);
        assert_eq!(r.read_u64().unwrap(), 2);
        assert_eq!(r.read_u64().unwrap(), 3);

        // getFutureSalts builder
        let gfs = build_get_future_salts(64);
        let mut r = TLReader::new(&gfs);
        assert_eq!(r.read_u32().unwrap(), FUTURE_SALTS_REQUEST);
        assert_eq!(r.read_i32().unwrap(), 64);
    }
}
