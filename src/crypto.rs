//! Cryptographic primitives for `MTProto` 2.0.
//!
//! Provides `AES-256-IGE` encryption/decryption, `SHA-1`/`SHA-256`/`SHA-512`/`MD5`
//! hashing, `CRC32` checksums, Diffie-Hellman key exchange, and RSA padding.
//!
//! # Panics
//!
//! Functions documented with a `# Panics` section panic on broken
//! invariants only (e.g. a pre-1970 wall clock); fallible paths return
//! [`Error::Crypto`].

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
use aes::cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use num_bigint::{BigRng010, BigUint};
use num_traits::One;
use rand::{Rng, rng};
use sha1::Digest as Sha1Digest;
use sha2::Sha256;

// ---------------------------------------------------------------------------
// AES-256-IGE — Telegram's Infinite Garble Extension mode
// ---------------------------------------------------------------------------

type Aes256EcbEnc = aes::Aes256Enc;
type Aes256EcbDec = aes::Aes256Dec;

/// Encrypt `data` in-place using AES-256-IGE.
///
/// IGE processes pairs of blocks (x, y):
///
/// ```text
/// c = E(x XOR y)
/// new_y = c
/// new_x = y
/// ```
///
/// `iv` must be exactly 32 bytes (two 16-byte blocks: x₀ || y₀).
/// `data` length must be a multiple of 16.
///
/// # Errors
///
/// Returns an error when `data` is empty-padded incorrectly, i.e. its
/// length is not a multiple of the 16-byte AES block size.
///
/// # Panics
///
/// Does not panic: the length check above and `chunks_mut(16)` guarantee
/// every chunk is a full 16-byte block before any indexed access.
pub fn aes_ige_encrypt(data: &mut [u8], key: &[u8; 32], iv: &[u8; 32]) -> Result<()> {
    if !data.len().is_multiple_of(16) {
        return Err(Error::Crypto(format!(
            "AES-IGE data length {} is not a multiple of 16",
            data.len()
        )));
    }
    if data.is_empty() {
        return Ok(());
    }

    let mut x = [0u8; 16];
    let mut y = [0u8; 16];
    x.copy_from_slice(&iv[0..16]);
    y.copy_from_slice(&iv[16..32]);

    let cipher = Aes256EcbEnc::new(key.into());

    for chunk in data.chunks_mut(16) {
        // MTProto IGE encrypt (gotd ige convention):
        //   c_i = E(p_i XOR c_prev) XOR p_prev
        // where c_prev starts as iv[0..16] and p_prev as iv[16..32].
        let mut block_arr: aes::Block = {
            let mut b = [0u8; 16];
            b.copy_from_slice(chunk);
            b.into()
        };
        for i in 0..16 {
            block_arr[i] ^= x[i]; // prev ciphertext
        }
        cipher.encrypt_block(&mut block_arr);
        for i in 0..16 {
            block_arr[i] ^= y[i]; // prev plaintext
        }

        let p: [u8; 16] = {
            // chunks_mut(16) on a length-checked buffer yields full blocks
            // only; `unwrap` is unreachable for other chunk shapes.
            #[allow(clippy::unwrap_used)]
            chunk.try_into().unwrap()
        };
        let c: [u8; 16] = block_arr.into();

        x.copy_from_slice(&c); // chain tracks ciphertext for pre-XOR
        y.copy_from_slice(&p); // and plaintext for post-XOR

        chunk.copy_from_slice(&c);
    }

    // Write back the updated IV for chaining (optional, not used externally here).
    Ok(())
}

/// Decrypt `data` in-place using AES-256-IGE.
///
/// # Errors
///
/// Returns an error when `data`'s length is not a multiple of the
/// 16-byte AES block size.
///
/// # Panics
///
/// Does not panic: the length check above and `chunks_mut(16)` guarantee
/// every chunk is a full 16-byte block before any indexed access.
pub fn aes_ige_decrypt(data: &mut [u8], key: &[u8; 32], iv: &[u8; 32]) -> Result<()> {
    if !data.len().is_multiple_of(16) {
        return Err(Error::Crypto(format!(
            "AES-IGE data length {} is not a multiple of 16",
            data.len()
        )));
    }
    if data.is_empty() {
        return Ok(());
    }

    let mut x = [0u8; 16];
    let mut y = [0u8; 16];
    x.copy_from_slice(&iv[0..16]);
    y.copy_from_slice(&iv[16..32]);

    let cipher = Aes256EcbDec::new(key.into());
    for chunk in data.chunks_mut(16) {
        // MTProto IGE decrypt (gotd ige convention):
        //   p_i = D(c_i XOR p_prev) XOR c_prev
        // where c_prev starts as iv[0..16] and p_prev as iv[16..32].
        let mut block_arr: aes::Block = {
            let mut b = [0u8; 16];
            b.copy_from_slice(chunk);
            b.into()
        };
        for i in 0..16 {
            block_arr[i] ^= y[i]; // prev plaintext
        }
        cipher.decrypt_block(&mut block_arr);
        for i in 0..16 {
            block_arr[i] ^= x[i]; // prev ciphertext
        }

        let c: [u8; 16] = {
            // same invariant as above: full 16-byte chunks only
            #[allow(clippy::unwrap_used)]
            chunk.try_into().unwrap()
        };
        let p: [u8; 16] = block_arr.into();

        x.copy_from_slice(&c); // chain tracks ciphertext for post-XOR
        y.copy_from_slice(&p); // and plaintext for pre-XOR

        chunk.copy_from_slice(&p);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Hashing wrappers
// ---------------------------------------------------------------------------

/// SHA-256 hash, returning 32 bytes.
#[must_use]
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// SHA-256 of two concatenated inputs.
#[must_use]
pub fn sha256_concat(a: &[u8], b: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(a);
    hasher.update(b);
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// SHA-1 hash, returning 20 bytes.
#[must_use]
pub fn sha1(data: &[u8]) -> [u8; 20] {
    use sha1::Digest;
    let mut hasher = sha1::Sha1::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut out = [0u8; 20];
    out.copy_from_slice(&result);
    out
}

/// SHA-1 of two concatenated inputs.
#[must_use]
pub fn sha1_concat(a: &[u8], b: &[u8]) -> [u8; 20] {
    use sha1::Digest;
    let mut hasher = sha1::Sha1::new();
    hasher.update(a);
    hasher.update(b);
    let result = hasher.finalize();
    let mut out = [0u8; 20];
    out.copy_from_slice(&result);
    out
}

/// MD5 hash, returning 16 bytes.
#[must_use]
pub fn md5(data: &[u8]) -> [u8; 16] {
    use md5::Digest;
    let mut hasher = md5::Md5::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&result);
    out
}

/// CRC32 of data — used for TL combinator IDs.
#[must_use]
pub fn crc32(data: &[u8]) -> u32 {
    // Standard CRC32 (same as used in TL schema)
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc ^ 0xFFFF_FFFF
}

// ---------------------------------------------------------------------------
// Diffie-Hellman with Telegram's 2048-bit prime
// ---------------------------------------------------------------------------

/// Telegram's standard 2048-bit DH prime.
/// Source: <https://core.telegram.org/mtproto/security_guidelines>
pub const DH_PRIME: [u8; 256] = [
    0xC7, 0x1C, 0xAE, 0xB9, 0xC6, 0xB1, 0xC9, 0x04, 0x8E, 0x6C, 0x52, 0x2F, 0x70, 0xF1, 0x3F, 0x73,
    0x98, 0x0D, 0x40, 0x23, 0x8E, 0x3E, 0x21, 0xC1, 0x49, 0x34, 0xD0, 0x37, 0x56, 0x3D, 0x93, 0x0F,
    0x48, 0x19, 0x8A, 0x0A, 0xA7, 0xC1, 0x40, 0x58, 0x22, 0x94, 0x93, 0xD2, 0x25, 0x30, 0xF4, 0xDB,
    0xFA, 0x33, 0x6F, 0x6E, 0x0A, 0xC9, 0x25, 0x13, 0x95, 0x43, 0xAE, 0xD4, 0x4C, 0xCE, 0x7C, 0x37,
    0x20, 0xFD, 0x51, 0xF6, 0x94, 0x58, 0x70, 0x5A, 0xC6, 0x8C, 0xD4, 0xFE, 0x6B, 0x6B, 0x13, 0xAB,
    0xDC, 0x97, 0x46, 0x51, 0x29, 0x69, 0x32, 0x84, 0x54, 0xF1, 0x8F, 0xAF, 0x8C, 0x59, 0x5F, 0x64,
    0x24, 0x77, 0xFE, 0x96, 0xBB, 0x2A, 0x94, 0x1D, 0x5B, 0xCD, 0x1D, 0x4A, 0xC8, 0xCC, 0x49, 0x88,
    0x07, 0x08, 0xFA, 0x9B, 0x37, 0x8E, 0x3C, 0x4F, 0x3A, 0x90, 0x60, 0xBE, 0xE6, 0x7C, 0xF9, 0xA4,
    0xA4, 0xA6, 0x95, 0x81, 0x10, 0x51, 0x90, 0x7E, 0x16, 0x27, 0x53, 0xB5, 0x6B, 0x0F, 0x6B, 0x41,
    0x0D, 0xBA, 0x74, 0xD8, 0xA8, 0x4B, 0x2A, 0x14, 0xB3, 0x14, 0x4E, 0x0E, 0xF1, 0x28, 0x47, 0x54,
    0xFD, 0x17, 0xED, 0x95, 0x0D, 0x59, 0x65, 0xB4, 0xB9, 0xDD, 0x46, 0x58, 0x2D, 0xB1, 0x17, 0x8D,
    0x16, 0x9C, 0x6B, 0xC4, 0x65, 0xB0, 0xD6, 0xFF, 0x9C, 0xA3, 0x92, 0x8F, 0xEF, 0x5B, 0x9A, 0xE4,
    0xE4, 0x18, 0xFC, 0x15, 0xE8, 0x3E, 0xBE, 0xA0, 0xF8, 0x7F, 0xA9, 0xFF, 0x5E, 0xED, 0x70, 0x05,
    0x0D, 0xED, 0x28, 0x49, 0xF4, 0x7B, 0xF9, 0x59, 0xD9, 0x56, 0x85, 0x0C, 0xE9, 0x29, 0x85, 0x1F,
    0x0D, 0x81, 0x15, 0xF6, 0x35, 0xB1, 0x05, 0xEE, 0x2E, 0x4E, 0x15, 0xD0, 0x4B, 0x24, 0x54, 0xBF,
    0x6F, 0x4F, 0xAD, 0xF0, 0x34, 0xB1, 0x04, 0x03, 0x11, 0x9C, 0xD8, 0xE3, 0xB9, 0x2F, 0xCC, 0x5B,
];
// ---------------------------------------------------------------------------
// Diffie-Hellman — RFC 3526 2048-bit MODP group 14 (SPEC §2)
// ---------------------------------------------------------------------------

/// DH generator. SPEC §2: g = 3.
pub const DH_GENERATOR: u32 = 3;

/// Result of a DH key exchange.
pub struct DhResult {
    /// The client's public value `g_a` or `g_b`.
    pub g_value: BigUint,
    /// The shared auth key.
    pub auth_key: Vec<u8>,
}

/// Get the DH prime as a `BigUint`.
#[must_use]
pub fn dh_prime() -> BigUint {
    BigUint::from_bytes_be(&DH_PRIME)
}

/// Perform a client-side DH step: generate random `a`, compute `g_a = g^a mod p`.
/// Returns the pair (`g_a`, `a`): the value to hand the server, and the
/// secret to keep for [`dh_client_complete`].
#[must_use]
pub fn dh_client_generate() -> (BigUint, BigUint) {
    let p = dh_prime();
    let g = BigUint::from(DH_GENERATOR);

    // Generate random 2048-bit number a
    let mut rng = rng();
    let a = rng.random_biguint(2048);

    // g_a = g^a mod p
    let g_a = g.modpow(&a, &p);

    (g_a, a)
}

/// Compute the shared auth key from server's `g_b` and client's secret `a`.
#[must_use]
#[allow(clippy::needless_pass_by_value)] // mirrors dh_server_complete's symmetric signature; both BigUints are read-only
pub fn dh_client_complete(g_b: BigUint, a: BigUint) -> Vec<u8> {
    let p = dh_prime();
    let shared = g_b.modpow(&a, &p);
    biguint_to_256_bytes(&shared)
}

/// Compute the shared auth key from client's `g_a` and server's secret `b`.
#[must_use]
#[allow(clippy::needless_pass_by_value)] // mirrors dh_client_complete's symmetric signature; both BigUints are read-only
pub fn dh_server_complete(g_a: BigUint, b: BigUint) -> Vec<u8> {
    let p = dh_prime();
    let shared = g_a.modpow(&b, &p);
    biguint_to_256_bytes(&shared)
}

/// Generate a random 2048-bit number for the server side of DH.
#[must_use]
pub fn dh_server_generate() -> (BigUint, BigUint) {
    let p = dh_prime();
    let g = BigUint::from(DH_GENERATOR);
    let mut rng = rng();
    let b = rng.random_biguint(2048);
    let g_b = g.modpow(&b, &p);

    (g_b, b)
}

/// Verify DH parameters received from the server.
///
/// # Errors
///
/// Returns [`Error::DhVerification`] when `g` is outside 2..=7, `g_a`
/// falls outside (1, p-1), or the prime is not 2048 bits.
pub fn verify_dh_params(g: u32, g_a: &BigUint, dh_prime_val: &BigUint) -> Result<()> {
    let one = BigUint::one();
    let prime_minus_one = dh_prime_val - &one;

    // g must be in range [2, 7]
    if !(2..=7).contains(&g) {
        return Err(Error::DhVerification(format!("g={g} is not in range 2..7")));
    }

    // g_a must be > 1 and < dh_prime - 1
    if g_a <= &one || g_a >= &prime_minus_one {
        return Err(Error::DhVerification(
            "g_a is out of range (must be > 1 and < dh_prime - 1)".into(),
        ));
    }

    // Check p is a 2048-bit prime (2^2047 < p < 2^2048)
    let bit_len = dh_prime_val.bits();
    if bit_len != 2048 {
        return Err(Error::DhVerification(format!(
            "DH prime has {bit_len} bits, expected 2048"
        )));
    }

    Ok(())
}

/// Convert a `BigUint` to exactly 256 bytes (big-endian), zero-padded on the left.
#[must_use]
fn biguint_to_256_bytes(n: &BigUint) -> Vec<u8> {
    let bytes = n.to_bytes_be();
    let mut result = vec![0u8; 256];
    let start = 256usize.saturating_sub(bytes.len());
    result[start..].copy_from_slice(&bytes);
    result
}

// ---------------------------------------------------------------------------
// Auth key ID — 64 lower bits of `SHA-1(auth_key)`
// ---------------------------------------------------------------------------

/// Compute the 64-bit `auth_key_id` from a 256-byte auth key.
#[must_use]
pub fn auth_key_id(auth_key: &[u8]) -> u64 {
    let hash = sha1(auth_key);
    // Take the last 8 bytes of the 20-byte SHA-1
    let mut id_bytes = [0u8; 8];
    id_bytes.copy_from_slice(&hash[12..20]);
    u64::from_be_bytes(id_bytes)
}

/// Compute `msg_key` for `MTProto` 2.0 encryption.
///
/// `msg_key_large` is `SHA-256(substr(auth_key, 88+x, 32) || plaintext_padded)`
/// and the returned key is `msg_key_large` bytes 8..24, per the official
/// `MTProto` 2.0 description and gotd/td `crypto/keys.go` (reference
/// implementation). Note: this is *not* the `MTProto` 1.0 `SHA-1` rule.
#[must_use]
pub fn msg_key_mtproto2(auth_key: &[u8], plaintext_padded: &[u8], x: usize) -> [u8; 16] {
    let mut hasher = Sha256::new();
    hasher.update(&auth_key[88 + x..88 + x + 32]);
    hasher.update(plaintext_padded);
    let large = hasher.finalize();

    let mut msg_key = [0u8; 16];
    msg_key.copy_from_slice(&large[8..24]);
    msg_key
}

/// Derive AES key and IV from `auth_key` and `msg_key` (`MTProto` 2.0).
///
/// ```text
/// sha256_a = SHA-256(msg_key || substr(auth_key, x, 36))
/// sha256_b = SHA-256(substr(auth_key, 40+x, 36) || msg_key)
/// aes_key  = sha256_a[0..8]  || sha256_b[8..24] || sha256_a[24..32]
/// aes_iv   = sha256_b[0..8]  || sha256_a[8..24] || sha256_b[24..32]
/// ```
///
/// Returns (`aes_key`, `aes_iv`) — both 32 bytes.
#[must_use]
pub fn aes_key_and_iv(auth_key: &[u8], msg_key: &[u8; 16], x: usize) -> ([u8; 32], [u8; 32]) {
    let sha256_a = {
        let mut h = Sha256::new();
        h.update(msg_key);
        h.update(&auth_key[x..x + 36]);
        h.finalize()
    };
    let sha256_b = {
        let mut h = Sha256::new();
        h.update(&auth_key[40 + x..40 + x + 36]);
        h.update(msg_key);
        h.finalize()
    };

    let mut aes_key = [0u8; 32];
    aes_key[0..8].copy_from_slice(&sha256_a[0..8]);
    aes_key[8..24].copy_from_slice(&sha256_b[8..24]);
    aes_key[24..32].copy_from_slice(&sha256_a[24..32]);

    let mut aes_iv = [0u8; 32];
    aes_iv[0..8].copy_from_slice(&sha256_b[0..8]);
    aes_iv[8..24].copy_from_slice(&sha256_a[8..24]);
    aes_iv[24..32].copy_from_slice(&sha256_b[24..32]);

    (aes_key, aes_iv)
}

// ---------------------------------------------------------------------------
// SRP password check (Cloud Password, auth.checkPassword)
// https://core.telegram.org/api/srp
// ---------------------------------------------------------------------------

/// Parameters for the SRP check as delivered by `account.getPassword`.
#[derive(Debug, Clone)]
pub struct SrpParams {
    /// `salt1` from `passwordKdfAlgo...`.
    pub salt1: Vec<u8>,
    /// `salt2` from `passwordKdfAlgo...`.
    pub salt2: Vec<u8>,
    /// `g` from `passwordKdfAlgo...`.
    pub g: u32,
    /// `p` from `passwordKdfAlgo...`.
    pub p: Vec<u8>,
    /// `srp_B` from `account.password`.
    pub b: Vec<u8>,
    /// `srp_id` from `account.password`.
    pub srp_id: i64,
}

/// The result of a successful client SRP run: everything
/// `inputCheckPasswordSRP` needs.
#[derive(Debug, Clone)]
pub struct SrpAnswer {
    /// Server-provided SRP session id.
    pub srp_id: i64,
    /// Client public value A (256 bytes big-endian).
    pub a: Vec<u8>,
    /// Proof M1 (SHA-256, 32 bytes).
    pub m1: [u8; 32],
}

/// SHA-256 over a growable input, helper local to the SRP block.
fn srp_sha256(parts: &[&[u8]]) -> [u8; 32] {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

/// Derive the SRP secret exponent `x` per the Telegram SRP spec
/// (<https://core.telegram.org/api/srp>, mirrored by tdlib's
/// `PasswordManager::calc_password_hash`):
///
/// ```text
/// SH(data, salt) = SHA256(salt || data || salt)
/// PH1 = SH(SH(password, salt1), salt2)
/// x   = PH2 = SH(PBKDF2-HMAC-SHA512(PH1, salt1, 100000), salt2)
/// ```
fn srp_derive_x(password: &[u8], salt1: &[u8], salt2: &[u8]) -> [u8; 32] {
    // PH1 = H(salt2 || H(salt1 || password || salt1) || salt2)
    let inner = srp_sha256(&[salt1, password, salt1]);
    let ph1 = srp_sha256(&[salt2, &inner, salt2]);

    // x = H(salt2 || PBKDF2-HMAC-SHA512(PH1, salt1, 100000) || salt2)
    let mut pbkdf2_out = [0u8; 64];
    let _ = pbkdf2::pbkdf2::<hmac::Hmac<sha2::Sha512>>(&ph1, salt1, 100_000, &mut pbkdf2_out);
    srp_sha256(&[salt2, &pbkdf2_out, salt2])
}

/// Run the client side of Telegram's SRP: derive the password hash, run the
/// SRP handshake with the server's `B`, and compute the proof `M1`.
///
/// Follows <https://core.telegram.org/api/srp> exactly (mirrored by tdlib's
/// `PasswordManager::get_input_check_password`): numbers hash in big-endian
/// form padded to 2048 bits; all math is modulo `p`.
///
/// The local bindings (`x`, `p`, `g`, `k`, `u`, `v`, `e`, `s`, `a`, `b`)
/// mirror the variable names in Telegram's SRP documentation; renaming
/// them would make the two sources harder to diff.
#[allow(clippy::many_single_char_names)]
#[must_use]
pub fn srp_check_password(password: &[u8], params: &SrpParams) -> SrpAnswer {
    let x_bytes = srp_derive_x(password, &params.salt1, &params.salt2);
    let x_bn = BigUint::from_bytes_be(&x_bytes);

    let p = BigUint::from_bytes_be(&params.p);
    let g_bn = BigUint::from(params.g);
    // Spec: "numbers must be used in big-endian form, padded to 2048 bits".
    let g_pad = biguint_to_256_bytes(&g_bn);

    // k = H(p | g), both padded to 2048 bits.
    let k = BigUint::from_bytes_be(&srp_sha256(&[&params.p, &g_pad]));

    // a = random 2048-bit number; A = g^a mod p
    let mut rng = rng();
    let a_pad;
    let a;
    loop {
        let mut a_bytes = vec![0u8; 256];
        rng.fill_bytes(&mut a_bytes);
        let a_try = BigUint::from_bytes_be(&a_bytes);
        if a_try == BigUint::ZERO || a_try >= p {
            continue;
        }
        a = a_try;
        a_pad = biguint_to_256_bytes(&g_bn.modpow(&a, &p));
        break;
    }

    // B arrives as its natural-length serialization; pad to 256 bytes for
    // hashing (tdlib: B_pad = 256 - B.len() zero bytes prepended).
    let b_bn = BigUint::from_bytes_be(&params.b);
    let b_pad = biguint_to_256_bytes(&b_bn);

    // u = H(A | B)
    let u = BigUint::from_bytes_be(&srp_sha256(&[&a_pad, &b_pad]));

    // v = g^x mod p; k_v = (k*v) mod p
    let v = g_bn.modpow(&x_bn, &p);
    let kv = (&k * &v) % &p;

    // t = (B - k_v) mod p (positive modulo, increment by p if negative)
    let t = (b_bn + &p - kv) % &p;

    // s_a = t^(a + u*x) mod p
    let e = &a + (&u * &x_bn);
    let s = t.modpow(&e, &p);

    // k_a = H(s_a)
    let s_pad = biguint_to_256_bytes(&s);
    let k_final = srp_sha256(&[&s_pad]);

    // M1 = H( H(p)^H(g) | H(salt1) | H(salt2) | A | B | k_a )
    let hp = srp_sha256(&[&params.p]);
    let hg = srp_sha256(&[&g_pad]);
    let mut xor = [0u8; 32];
    for (i, (p_i, g_i)) in hp.iter().zip(hg.iter()).enumerate() {
        xor[i] = p_i ^ g_i;
    }
    let h_salt1 = srp_sha256(&[&params.salt1]);
    let h_salt2 = srp_sha256(&[&params.salt2]);
    let m1 = srp_sha256(&[
        &xor,
        &h_salt1,
        &h_salt2,
        &a_pad,
        &b_pad,
        &k_final,
    ]);

    SrpAnswer {
        srp_id: params.srp_id,
        a: a_pad,
        m1,
    }
}

// ---------------------------------------------------------------------------
// RSA Padding (`RSA_PAD`) for DH key exchange
// ---------------------------------------------------------------------------

/// Perform `RSA_PAD` as specified by Telegram for the DH handshake.
///
/// Pads data with random bytes to 192 bytes, then encrypts with the
/// server's `RSA` public key using `OAEP`-like padding specific to `MTProto`.
///
/// # Errors
///
/// Returns [`Error::Crypto`] when `data` exceeds 144 bytes or the server
/// key is not a 2048-bit modulus.
pub fn rsa_pad(data: &[u8], server_public_key: &RsaPublicKey) -> Result<Vec<u8>> {
    if data.len() > 144 {
        return Err(Error::Crypto(
            "RSA_PAD: data too long (max 144 bytes)".into(),
        ));
    }

    let modulus_size = server_public_key.modulus_len_bytes();
    if modulus_size != 256 {
        return Err(Error::Crypto(format!(
            "RSA_PAD: expected 2048-bit key, got {modulus_size} bytes"
        )));
    }

    let mut rng = rng();

    // data_with_padding = data + random bytes to make exactly 192 bytes
    let padding_len = 192 - data.len();
    let mut data_with_padding = Vec::with_capacity(192);
    data_with_padding.extend_from_slice(data);
    let mut pad = vec![0u8; padding_len];
    rng.fill_bytes(&mut pad);
    data_with_padding.extend_from_slice(&pad);

    // data_pad_reversed = reverse byte order
    let mut data_pad_reversed = data_with_padding.clone();
    data_pad_reversed.reverse();

    // Generate random 32-byte temp_key
    let mut temp_key = [0u8; 32];
    rng.fill_bytes(&mut temp_key);

    // data_with_hash = data_pad_reversed + SHA256(temp_key + data_with_padding)
    let temp_hash = sha256_concat(&temp_key, &data_with_padding);
    let mut data_with_hash = Vec::with_capacity(224);
    data_with_hash.extend_from_slice(&data_pad_reversed);
    data_with_hash.extend_from_slice(&temp_hash);

    // aes_encrypted = AES-256-IGE(data_with_hash, temp_key, zero_iv)
    let mut aes_encrypted = data_with_hash.clone();
    let zero_iv = [0u8; 32];
    aes_ige_encrypt(&mut aes_encrypted, &temp_key, &zero_iv)?;

    // temp_key_xor = temp_key XOR SHA256(aes_encrypted)
    let aes_hash = sha256(&aes_encrypted);
    let mut temp_key_xor = [0u8; 32];
    for i in 0..32 {
        temp_key_xor[i] = temp_key[i] ^ aes_hash[i];
    }

    // key_aes_encrypted = temp_key_xor + aes_encrypted (256 bytes total)
    let mut key_aes_encrypted = Vec::with_capacity(256);
    key_aes_encrypted.extend_from_slice(&temp_key_xor);
    key_aes_encrypted.extend_from_slice(&aes_encrypted);

    // Check against RSA modulus
    let key_bn = BigUint::from_bytes_be(&key_aes_encrypted);
    let modulus_bn = BigUint::from_bytes_be(&server_public_key.n);
    if key_bn >= modulus_bn {
        // Retry with new random bytes
        return rsa_pad(data, server_public_key);
    }

    // RSA encryption: result = key_aes_encrypted^e mod n
    let e_bn = BigUint::from_bytes_be(&server_public_key.e);
    let result_bn = key_bn.modpow(&e_bn, &modulus_bn);
    let result_bytes = biguint_to_256_bytes(&result_bn);

    Ok(result_bytes)
}

/// RSA public key for Telegram servers.
#[derive(Debug, Clone)]
pub struct RsaPublicKey {
    /// modulus (big-endian bytes)
    pub n: Vec<u8>,
    /// exponent (big-endian bytes)
    pub e: Vec<u8>,
}

impl RsaPublicKey {
    /// Create from modulus and exponent bytes.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // struct update of Vec fields is not const
    pub fn new(n: Vec<u8>, e: Vec<u8>) -> Self {
        Self { n, e }
    }

    /// Modulus length in bytes.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // Vec::len is not const-stable
    pub fn modulus_len_bytes(&self) -> usize {
        self.n.len()
    }

    /// Compute the 64-bit fingerprint of this RSA public key.
    ///
    /// `MTProto` definition (gotd `RSAFingerprint`): `int64(LE u64 of
    /// SHA1(rsa_public_key(n:string e:string))[12..20])` — the SHA-1
    /// runs over the TL-serialized combinator.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let buf = self.tl_serialized();
        let hash = sha1(&buf);
        let mut id_bytes = [0u8; 8];
        id_bytes.copy_from_slice(&hash[12..20]);
        u64::from_le_bytes(id_bytes)
    }

    /// TL bytes of the bare `rsa_public_key n:string e:string` combinator.
    fn tl_serialized(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&tl_string_bytes(&self.n));
        buf.extend_from_slice(&tl_string_bytes(&self.e));
        buf
    }
}

/// Compute the TL-serialized bytes of a `string` (bare).
fn tl_string_bytes(data: &[u8]) -> Vec<u8> {
    let len = data.len();
    let mut buf = Vec::new();
    if len <= 252 {
        buf.push(len as u8);
    } else {
        buf.push(254);
        buf.push((len & 0xFF) as u8);
        buf.push(((len >> 8) & 0xFF) as u8);
        buf.push(((len >> 16) & 0xFF) as u8);
    }
    buf.extend_from_slice(data);
    // Pad to 4-byte alignment
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
    buf
}

// ---------------------------------------------------------------------------
// Known Telegram server public keys
// ---------------------------------------------------------------------------

/// Telegram's production RSA public keys (main DCs).
///
/// Source: gotd/td `mtproto/_data/public_keys.pem` (canonical, well-known).
///
/// # Panics
///
/// Panics if the embedded PEM-derived hex constants fail to decode;
/// they are compile-time-known-good values.
#[must_use]
#[allow(clippy::expect_used)] // hex constants are fixed and verified by test
pub fn known_server_keys() -> Vec<RsaPublicKey> {
    // Key 1: fingerprint 0x03268d20df9858b2
    let n1 = hex_decode(
        "C8C11D635691FAC091DD9489AEDCED2932AA8A0BCEFEF05FA800892D9B52ED03200865C9E97211CB2EE6C7AE96D3FB0E15AEFFD66019B44A08A240CFDD2868A85E1F54D6FA5DEAA041F6941DDF302690D61DC476385C2FA655142353CB4E4B59F6E5B6584DB76FE8B1370263246C010C93D011014113EBDF987D093F9D37C2BE48352D69A1683F8F6E6C2167983C761E3AB169FDE5DAAA12123FA1BEAB621E4DA5935E9C198F82F35EAE583A99386D8110EA6BD1ABB0F568759F62694419EA5F69847C43462ABEF858B4CB5EDC84E7B9226CD7BD7E183AA974A712C079DDE85B9DC063B8A5C08E8F859C0EE5DCD824C7807F20153361A7F63CFD2A433A1BE7F5",
    )
    .expect("valid hex");
    let e1 = hex_decode("010001").expect("valid hex");
    // Key 2: fingerprint 0x85fd64de851d9dd0
    let n2 = hex_decode(
        "E8BB3305C0B52C6CF2AFDF7637313489E63E05268E5BADB601AF417786472E5F93B85438968E20E6729A301C0AFC121BF7151F834436F7FDA680847A66BF64ACCEC78EE21C0B316F0EDAFE2F41908DA7BD1F4A5107638EEB67040ACE472A14F90D9F7C2B7DEF99688BA3073ADB5750BB02964902A359FE745D8170E36876D4FD8A5D41B2A76CBFF9A13267EB9580B2D06D10357448D20D9DA2191CB5D8C93982961CDFDEDA629E37F1FB09A0722027696032FE61ED663DB7A37F6F263D370F69DB53A0DC0A1748BDAAFF6209D5645485E6E001D1953255757E4B8E42813347B11DA6AB500FD0ACE7E6DFA3736199CCAF9397ED0745A427DCFA6CD67BCB1ACFF3",
    )
    .expect("valid hex");
    let e2 = hex_decode("010001").expect("valid hex");

    vec![RsaPublicKey::new(n1, e1), RsaPublicKey::new(n2, e2)]
}

/// Find a server key by fingerprint.
#[must_use]
pub fn find_server_key(fingerprint: u64) -> Option<RsaPublicKey> {
    known_server_keys()
        .into_iter()
        .find(|k| k.fingerprint() == fingerprint)
}

/// Helper to decode hex strings.
#[must_use]
fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    let hex = hex
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()
}

// ---------------------------------------------------------------------------
// Random byte generation
// ---------------------------------------------------------------------------

/// Generate `len` random bytes.
#[must_use]
pub fn random_bytes(len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    rng().fill_bytes(&mut buf);
    buf
}

#[must_use]
pub fn random_nonce() -> [u8; 16] {
    let mut nonce = [0u8; 16];
    rng().fill_bytes(&mut nonce);
    nonce
}

#[must_use]
pub fn random_nonce_256() -> [u8; 32] {
    let mut nonce = [0u8; 32];
    rng().fill_bytes(&mut nonce);
    nonce
}

#[must_use]
pub fn random_session_id() -> u64 {
    rng().next_u64()
}

/// Compute a message ID based on current time (divisible by 4 for client messages).
///
/// # Panics
///
/// Panics only if the wall clock reads before 1970 — a broken system
/// clock that would invalidate every other timestamp anyway.
#[must_use]
#[allow(clippy::unwrap_used)] // pre-1970 wall clock would break everything else first
pub fn next_msg_id(server_time_offset: i64) -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let time = now + server_time_offset;
    ((time as u64) << 32) | 4 // Client msg_ids are divisible by 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc32() {
        // TL "vector t:Type # [ t ] = Vector t" should produce 0x1cb5c415
        let s = "vector t:Type # [ t ] = Vector t";
        let c = crc32(s.as_bytes());
        assert_eq!(c, 0x1cb5c415);
    }

    #[test]
    fn test_sha256() {
        let hash = sha256(b"hello");
        assert_eq!(hash.len(), 32);
        // SHA-256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        let mut hex = String::with_capacity(hash.len() * 2);
        for b in hash {
            use std::fmt::Write as _;
            let _ = write!(hex, "{b:02x}");
        }
        assert_eq!(
            hex,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    // Test code: unwrap is the idiomatic failure mode here.
    #[allow(clippy::unwrap_used)]
    fn test_aes_ige_roundtrip() {
        let key = [0x42u8; 32];
        let iv = [0x24u8; 32];
        let original = b"Hello, MTProto world!!Extra padding for 16 bytes!!";
        // Pad to multiple of 16
        let mut data = original.to_vec();
        let pad = 16 - (data.len() % 16);
        if pad != 16 {
            data.resize(data.len() + pad, 0);
        }
        let mut encrypted = data.clone();
        aes_ige_encrypt(&mut encrypted, &key, &iv).unwrap();

        // IV must be reset for decryption
        aes_ige_decrypt(&mut encrypted, &key, &iv).unwrap();
        assert_eq!(encrypted, data);
    }

    #[test]
    fn test_fingerprint() {
        let key = known_server_keys();
        // Just verify we get valid fingerprints
        for k in &key {
            let fp = k.fingerprint();
            assert!(fp != 0);
        }
    }

    /// SPEC §4/§4.1 regression guards: pins the exact derivation formulas.
    /// Any change to these functions must be a SPEC-verified change.
    #[test]
    fn test_msg_key_is_sha256_slice() {
        let auth_key = [0x42u8; 256];
        let plaintext = b"payload";
        let key = msg_key_mtproto2(&auth_key, plaintext, 0);
        // msg_key = SHA-256(auth_key[88..120] || plaintext)[8..24]
        let mut hasher = Sha256::new();
        hasher.update(&auth_key[88..120]);
        hasher.update(plaintext);
        let large = hasher.finalize();
        assert_eq!(&key, &large[8..24]);
    }

    #[test]
    fn test_aes_key_and_iv_assembly() {
        let auth_key = [0x11u8; 256];
        let msg_key = [0x22u8; 16];
        let (aes_key, aes_iv) = aes_key_and_iv(&auth_key, &msg_key, 0);

        // Recompute per the MTProto 2.0 description independently.
        let mut ha = Sha256::new();
        ha.update(msg_key);
        ha.update(&auth_key[0..36]);
        let sha_a = ha.finalize();
        let mut hb = Sha256::new();
        hb.update(&auth_key[40..76]);
        hb.update(msg_key);
        let sha_b = hb.finalize();

        let mut want_key = Vec::new();
        want_key.extend_from_slice(&sha_a[0..8]);
        want_key.extend_from_slice(&sha_b[8..24]);
        want_key.extend_from_slice(&sha_a[24..32]);
        let mut want_iv = Vec::new();
        want_iv.extend_from_slice(&sha_b[0..8]);
        want_iv.extend_from_slice(&sha_a[8..24]);
        want_iv.extend_from_slice(&sha_b[24..32]);

        assert_eq!(aes_key.as_slice(), want_key.as_slice());
        assert_eq!(aes_iv.as_slice(), want_iv.as_slice());
    }

    /// C1 regression guard: pins the PH1/PH2/x pipeline to the Telegram
    /// SRP spec formulas (<https://core.telegram.org/api/srp>) and the
    /// 256-byte A pad, recomputed independently of the production code.
    #[test]
    fn test_srp_derivation_spec_formulas() {
        // Independent recomputation per tdlib PasswordManager::calc_password_hash:
        //   SH(data, salt) = H(salt | data | salt)
        //   PH1 = SH(SH(password, salt1), salt2)
        //   x   = SH(PBKDF2-HMAC-SHA512(PH1, salt1, 100000), salt2)
        use sha2::Digest as _;
        let h = |parts: &[&[u8]]| -> [u8; 32] {
            let mut hasher = sha2::Sha256::new();
            for p in parts {
                hasher.update(p);
            }
            hasher.finalize().into()
        };
        let inner = h(&[b"s1", b"pw", b"s1"]);
        let ph1 = h(&[b"s2", &inner, b"s2"]);
        let mut pbkdf_out = [0u8; 64];
        let _ = pbkdf2::pbkdf2::<hmac::Hmac<sha2::Sha512>>(&ph1, b"s1", 100_000, &mut pbkdf_out);
        let x = h(&[b"s2", &pbkdf_out, b"s2"]);

        // Production helper must match byte-for-byte.
        assert_eq!(srp_derive_x(b"pw", b"s1", b"s2"), x);
        assert_ne!(x, ph1); // sanity: the PBKDF2 stage changes the result

        // g pads to 256 bytes (2048 bits) for k and H(g), not minimal bytes.
        let g_pad = biguint_to_256_bytes(&BigUint::from(3u32));
        assert_eq!(g_pad.len(), 256);
        assert_eq!(&g_pad[254..], &[0, 3]);

        // Structural contract: A padded to 256 bytes, M1 32 bytes.
        let params = SrpParams {
            salt1: b"s1".to_vec(),
            salt2: b"s2".to_vec(),
            g: 3,
            p: DH_PRIME.to_vec(),
            b: vec![0xFF; 256],
            srp_id: 7,
        };
        let ans = srp_check_password(b"pw", &params);
        assert_eq!(ans.srp_id, 7);
        assert_eq!(ans.a.len(), 256);
        assert_eq!(ans.m1.len(), 32);
    }

    #[test]
    fn test_dh_generator_is_three() {
        // SPEC §2: g = 3.
        assert_eq!(DH_GENERATOR, 3);
    }
}
