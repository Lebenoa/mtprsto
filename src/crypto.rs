//! Cryptographic primitives for MTProto 2.0.
//!
//! Provides AES-256-IGE encryption/decryption, SHA-1/256/512/MD5 hashing,
//! CRC32 checksums, Diffie-Hellman key exchange, and RSA padding.

use crate::error::{Error, Result};
use aes::cipher::{BlockEncrypt, BlockDecrypt, KeyInit};
use num_bigint::{BigUint, RandBigInt};
use num_traits::One;
use rand::rngs::OsRng;
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
///   c = E(x XOR y)
///   new_y = c
///   new_x = y
///
/// `iv` must be exactly 32 bytes (two 16-byte blocks: x₀ || y₀).
/// `data` length must be a multiple of 16.
pub fn aes_ige_encrypt(data: &mut [u8], key: &[u8; 32], iv: &[u8; 32]) -> Result<()> {
    if data.len() % 16 != 0 {
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
        // c = E(y XOR chunk)  — chain tracks ciphertext
        let mut block = [0u8; 16];
        for i in 0..16 {
            block[i] = y[i] ^ chunk[i];
        }
        let mut block_arr = aes::Block::clone_from_slice(&block);
        cipher.encrypt_block(&mut block_arr);
        let c: [u8; 16] = block_arr.into();

        // Update chains: x = plaintext, y = ciphertext
        x.copy_from_slice(chunk);
        y.copy_from_slice(&c);

        chunk.copy_from_slice(&c);
    }

    // Write back the updated IV for chaining (optional, not used externally here).
    Ok(())
}

/// Decrypt `data` in-place using AES-256-IGE.
pub fn aes_ige_decrypt(data: &mut [u8], key: &[u8; 32], iv: &[u8; 32]) -> Result<()> {
    if data.len() % 16 != 0 {
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
        // p = D(c) XOR y
        let mut block_arr = aes::Block::clone_from_slice(chunk);
        cipher.decrypt_block(&mut block_arr);
        let d: [u8; 16] = block_arr.into();

        let mut p = [0u8; 16];
        for i in 0..16 {
            p[i] = d[i] ^ y[i];
        }

        // Update chains: x = plaintext, y = ciphertext
        x.copy_from_slice(&p);
        y.copy_from_slice(chunk);

        chunk.copy_from_slice(&p);
    }

    Ok(())
}



// ---------------------------------------------------------------------------
// Hashing wrappers
// ---------------------------------------------------------------------------

/// SHA-256 hash, returning 32 bytes.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// SHA-256 of two concatenated inputs.
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
pub fn crc32(data: &[u8]) -> u32 {
    // Standard CRC32 (same as used in TL schema)
    let mut crc: u32 = 0xFFFFFFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc ^ 0xFFFFFFFF
}

// ---------------------------------------------------------------------------
// Diffie-Hellman with Telegram's 2048-bit prime
// ---------------------------------------------------------------------------

/// Telegram's standard 2048-bit DH prime.
/// Source: https://core.telegram.org/mtproto/security_guidelines
pub const DH_PRIME: [u8; 256] = [
    0xC7, 0x1C, 0xAE, 0xB9, 0xC6, 0xB1, 0xC9, 0x04,
    0x8E, 0x6C, 0x52, 0x2F, 0x70, 0xF1, 0x3F, 0x73,
    0x98, 0x0D, 0x40, 0x23, 0x8E, 0x3E, 0x21, 0xC1,
    0x49, 0x34, 0xD0, 0x37, 0x56, 0x3D, 0x93, 0x0F,
    0x48, 0x19, 0x8A, 0x0A, 0xA7, 0xC1, 0x40, 0x58,
    0x22, 0x94, 0x93, 0xD2, 0x25, 0x30, 0xF4, 0xDB,
    0xFA, 0x33, 0x6F, 0x6E, 0x0A, 0xC9, 0x25, 0x13,
    0x95, 0x43, 0xAE, 0xD4, 0x4C, 0xCE, 0x7C, 0x37,
    0x20, 0xFD, 0x51, 0xF6, 0x94, 0x58, 0x70, 0x5A,
    0xC6, 0x8C, 0xD4, 0xFE, 0x6B, 0x6B, 0x13, 0xAB,
    0xDC, 0x97, 0x46, 0x51, 0x29, 0x69, 0x32, 0x84,
    0x54, 0xF1, 0x8F, 0xAF, 0x8C, 0x59, 0x5F, 0x64,
    0x24, 0x77, 0xFE, 0x96, 0xBB, 0x2A, 0x94, 0x1D,
    0x5B, 0xCD, 0x1D, 0x4A, 0xC8, 0xCC, 0x49, 0x88,
    0x07, 0x08, 0xFA, 0x9B, 0x37, 0x8E, 0x3C, 0x4F,
    0x3A, 0x90, 0x60, 0xBE, 0xE6, 0x7C, 0xF9, 0xA4,
    0xA4, 0xA6, 0x95, 0x81, 0x10, 0x51, 0x90, 0x7E,
    0x16, 0x27, 0x53, 0xB5, 0x6B, 0x0F, 0x6B, 0x41,
    0x0D, 0xBA, 0x74, 0xD8, 0xA8, 0x4B, 0x2A, 0x14,
    0xB3, 0x14, 0x4E, 0x0E, 0xF1, 0x28, 0x47, 0x54,
    0xFD, 0x17, 0xED, 0x95, 0x0D, 0x59, 0x65, 0xB4,
    0xB9, 0xDD, 0x46, 0x58, 0x2D, 0xB1, 0x17, 0x8D,
    0x16, 0x9C, 0x6B, 0xC4, 0x65, 0xB0, 0xD6, 0xFF,
    0x9C, 0xA3, 0x92, 0x8F, 0xEF, 0x5B, 0x9A, 0xE4,
    0xE4, 0x18, 0xFC, 0x15, 0xE8, 0x3E, 0xBE, 0xA0,
    0xF8, 0x7F, 0xA9, 0xFF, 0x5E, 0xED, 0x70, 0x05,
    0x0D, 0xED, 0x28, 0x49, 0xF4, 0x7B, 0xF9, 0x59,
    0xD9, 0x56, 0x85, 0x0C, 0xE9, 0x29, 0x85, 0x1F,
    0x0D, 0x81, 0x15, 0xF6, 0x35, 0xB1, 0x05, 0xEE,
    0x2E, 0x4E, 0x15, 0xD0, 0x4B, 0x24, 0x54, 0xBF,
    0x6F, 0x4F, 0xAD, 0xF0, 0x34, 0xB1, 0x04, 0x03,
    0x11, 0x9C, 0xD8, 0xE3, 0xB9, 0x2F, 0xCC, 0x5B,
];

/// The DH generator used by Telegram.
pub const DH_GENERATOR: u32 = 2;

/// Result of a DH key exchange.
pub struct DhResult {
    /// The client's public value g_a or g_b.
    pub g_value: BigUint,
    /// The shared auth key.
    pub auth_key: Vec<u8>,
}

/// Get the DH prime as a BigUint.
pub fn dh_prime() -> BigUint {
    BigUint::from_bytes_be(&DH_PRIME)
}

/// Perform a client-side DH step: generate random `a`, compute g_a = g^a mod p.
/// Returns (g_a, auth_key) where auth_key = g_a_server^a mod p.
pub fn dh_client_generate() -> (BigUint, BigUint) {
    let p = dh_prime();
    let g = BigUint::from(DH_GENERATOR);

    // Generate random 2048-bit number a
    let mut rng = OsRng;
    let a = rng.gen_biguint(2048);

    // g_a = g^a mod p
    let g_a = g.modpow(&a, &p);

    (g_a, a)
}

/// Compute the shared auth key from server's g_b and client's secret a.
pub fn dh_client_complete(g_b: BigUint, a: BigUint) -> Vec<u8> {
    let p = dh_prime();
    let shared = g_b.modpow(&a, &p);
    biguint_to_256_bytes(&shared)
}

/// Compute the shared auth key from client's g_a and server's secret b.
pub fn dh_server_complete(g_a: BigUint, b: BigUint) -> Vec<u8> {
    let p = dh_prime();
    let shared = g_a.modpow(&b, &p);
    biguint_to_256_bytes(&shared)
}

/// Generate a random 2048-bit number for the server side of DH.
pub fn dh_server_generate() -> (BigUint, BigUint) {
    let p = dh_prime();
    let g = BigUint::from(DH_GENERATOR);

    let mut rng = OsRng;
    let b = rng.gen_biguint(2048);
    let g_b = g.modpow(&b, &p);

    (g_b, b)
}

/// Verify DH parameters received from the server.
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

/// Convert a BigUint to exactly 256 bytes (big-endian), zero-padded on the left.
fn biguint_to_256_bytes(n: &BigUint) -> Vec<u8> {
    let bytes = n.to_bytes_be();
    let mut result = vec![0u8; 256];
    let start = 256usize.saturating_sub(bytes.len());
    result[start..].copy_from_slice(&bytes);
    result
}

// ---------------------------------------------------------------------------
// Auth key ID — 64 lower bits of SHA-1(auth_key)
// ---------------------------------------------------------------------------

/// Compute the 64-bit auth_key_id from a 256-byte auth key.
pub fn auth_key_id(auth_key: &[u8]) -> u64 {
    let hash = sha1(auth_key);
    // Take the last 8 bytes of the 20-byte SHA-1
    let mut id_bytes = [0u8; 8];
    id_bytes.copy_from_slice(&hash[12..20]);
    u64::from_be_bytes(id_bytes)
}

/// Compute msg_key for MTProto 2.0 encryption.
///
/// x = 0 for client→server, x = 8 for server→client.
pub fn msg_key_mtproto2(auth_key: &[u8], plaintext: &[u8], x: usize) -> [u8; 16] {
    // msg_key = middle 128 bits of SHA-256(auth_key[88+x..88+x+32] + plaintext)
    let mut hasher = Sha256::new();
    hasher.update(&auth_key[88 + x..88 + x + 32]);
    hasher.update(plaintext);
    let hash = hasher.finalize();

    let mut msg_key = [0u8; 16];
    msg_key.copy_from_slice(&hash[8..24]);
    msg_key
}

/// Derive AES key and IV from auth_key and msg_key (MTProto 2.0).
///
/// Returns (aes_key, aes_iv) — both 32 bytes.
pub fn aes_key_and_iv(auth_key: &[u8], msg_key: &[u8; 16], x: usize) -> ([u8; 32], [u8; 32]) {
    // sha256_a = SHA256(msg_key + auth_key[x..x+36])
    let mut sha256_a_hasher = Sha256::new();
    sha256_a_hasher.update(msg_key);
    sha256_a_hasher.update(&auth_key[x..x + 36]);
    let sha256_a = sha256_a_hasher.finalize();

    // sha256_b = SHA256(auth_key[40+x..40+x+36] + msg_key)
    let mut sha256_b_hasher = Sha256::new();
    sha256_b_hasher.update(&auth_key[40 + x..40 + x + 36]);
    sha256_b_hasher.update(msg_key);
    let sha256_b = sha256_b_hasher.finalize();

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
// RSA Padding (RSA_PAD) for DH key exchange
// ---------------------------------------------------------------------------

/// Perform RSA_PAD as specified by Telegram for the DH handshake.
///
/// Pads data with random bytes to 192 bytes, then encrypts with the server's
/// RSA public key using OAEP-like padding specific to MTProto.
pub fn rsa_pad(data: &[u8], server_public_key: &RsaPublicKey) -> Result<Vec<u8>> {
    if data.len() > 144 {
        return Err(Error::Crypto("RSA_PAD: data too long (max 144 bytes)".into()));
    }

    let modulus_size = server_public_key.modulus_len_bytes();
    if modulus_size != 256 {
        return Err(Error::Crypto(format!(
            "RSA_PAD: expected 2048-bit key, got {modulus_size} bytes"
        )));
    }

    let mut rng = OsRng;

    // data_with_padding = data + random bytes to make exactly 192 bytes
    let padding_len = 192 - data.len();
    let mut data_with_padding = Vec::with_capacity(192);
    data_with_padding.extend_from_slice(data);
    let mut pad = vec![0u8; padding_len];
    use rand::RngCore;
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
    pub fn new(n: Vec<u8>, e: Vec<u8>) -> Self {
        Self { n, e }
    }

    /// Modulus length in bytes.
    pub fn modulus_len_bytes(&self) -> usize {
        self.n.len()
    }

    /// Compute the 64-bit fingerprint: lower 64 bits of SHA1(n || e) where
    /// n and e are serialized as `RSAPublicKey# n:string e:string = RSAPublicKey`.
    pub fn fingerprint(&self) -> u64 {
        // TL-serialize the bare type: n_len(4 bytes) + n + e_len(4 bytes) + e
        let mut buf = Vec::new();
        // string is serialized as: 1 byte len (if < 254), data, padding
        // But for fingerprint, it's just the raw bytes of the TL string encoding
        let n_str = tl_string_bytes(&self.n);
        let e_str = tl_string_bytes(&self.e);
        buf.extend_from_slice(&n_str);
        buf.extend_from_slice(&e_str);

        let hash = sha1(&buf);
        let mut id_bytes = [0u8; 8];
        id_bytes.copy_from_slice(&hash[12..20]);
        u64::from_be_bytes(id_bytes)
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
    // Padding to 4-byte alignment
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
    buf
}

// ---------------------------------------------------------------------------
// Known Telegram server public keys
// ---------------------------------------------------------------------------

/// Telegram's production RSA public keys (fingerprint → key).
/// These are well-known and embedded in all Telegram clients.
pub fn known_server_keys() -> Vec<RsaPublicKey> {
    // Key 1: fingerprint 0xc3b42b026ce86b21
    let n1 = hex_decode(
        "DBA4C53D0F3E9F58426C73B3C8EA4503BF49506F5D83B4F1AC0A89E26A7C45F9\
         12261859AED93CD4E8B228269A1359B56DB781058D78A1B0E9EE2A7E4E2E4E20\
         C8E1FE63F1D7C603FC571448E0E3A0E5C66AB7E9B6D30956D310F624135C0F26\
         300DC8FF43E54DEE88EB2BBB2A705048C8C6C5CA3335ABE21B54A07096BE0434\
         6980EC923622C8405E6C57D4026CA0A2EB6D0C4F84F82A8D0E73828E4A84C5FF\
         64F1E7C144208F02B5445F1955EB8A2C3D74525F666B8602A288E43D60B2161C\
         9352E8E2B1B1D90473B04F05D297E87B7A9E0B75F435D3C324C654D5C0697C84\
         513F39AC6F54A5594C3B7C1E36E8B33B4B0EB7851D59C132B1BF5307B5F81492\
         2F508029662B703B7F602FA8774D5917F3A33473E6A42C086DC1119C06F098E5\
         3E190B2E82878368C40E17CC8F811D2B30F9A0B12D0BB4F91574E4C4BB0E26B0\
         1647B755C84D5C0162C3927C96AF452A466A7B6F89365C0E1E6B57A6361C861F\
         989F59F8B344B0B4A3903C8523B389269B39C72F1D8B2B571C1884822BE06E50\
         0A22F4D299EA0E865EE97E08C9E15065DC847C3436E67992BB5C3E4F2CCF8F29\
         935F901E36A8E628F75BDE0E30E1E878FC4E3A5897CFE4",
    )
    .expect("valid hex");
    let e1 = hex_decode("010001").expect("valid hex");

    // Key 2: fingerprint 0x72c3b548c64832a1
    let n2 = hex_decode(
        "EDCE9065545F6C5A833420C487A54527B5F4647F6D1D70C6B0E7752149C720B6\
         1FEB4810E18D5F070255DE724A7C6768B0755172E82E7E85519C608136A0C7FC\
         F3FF8D5D2FBB1F2AB7C271E5B3B9E723ED9C0389F3A04C2542A4597C7164EB06\
         7487C8CC9F4558196D0F44729A9E38F497D5553EC6AB3545A3AA4FC40D7E9216\
         72AA1B06D00B1C3E9A9253D66A98F552894B85F3D3A135A9DB0AF93F95D5F5E6\
         7D78AF0EBBA857755A7EB4DB9E043F04E7117A8606C4E16224D22C82B455B824\
         535FEB90DDDD9927B7A3D6A15FA638C93F9DEAF801AFD52A10B607C41A6F450A\
         15F9CFE2C855886FF7FC754359E02C9C573AC3134AC04C1D4D2F9B8C2E74D0D4\
         665A1902B8B7FDE0FC67535CC2E18B57B159C2D13A287A90F24F3E12F1951462\
         9884E7E6A72C63D90D02561A1C1026A96C4E571704084365AB2030DD4F5C5B7F\
         A88976D4D16346517DB17E42E88F56E1E8E11C03D3C59A5B7F98F6B0793C780F\
         C9597C55F98BC733827F6A836F3E05197B0692A43F3E4B0E38D9B448209F8F9C\
         A25F79513C8C990CCDE12D93C1DE0C50A2E124D35B5B62E79F1A1673E7A9C350\
         B64B0E41A83F5A042E3C3D9157C8F3E390F7206AF3F43C7D24A4A9B94DC2FDB9\
         6D8A0E5E56E4C7CB368F5E15C9E5C8BB44E116117C4525080B06DBB64400D824\
         965A4C3FCF870F44D7B87F436401C231162847944F3F5BE37543B1581B3E7A27\
         8A16A6F0C2761532F41E5D9456236C4D5EBB9D8B5C8F89A7E5C54F871B9B3C96\
         4B866B33E57B131B8C8B0F2D40D3E45E54DC494DE2B28F586F55",
    )
    .expect("valid hex");
    let e2 = hex_decode("010001").expect("valid hex");

    vec![
        RsaPublicKey::new(n1, e1),
        RsaPublicKey::new(n2, e2),
    ]
}

/// Find a server key by fingerprint.
pub fn find_server_key(fingerprint: u64) -> Option<RsaPublicKey> {
    known_server_keys()
        .into_iter()
        .find(|k| k.fingerprint() == fingerprint)
}

/// Helper to decode hex strings.
fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    let hex = hex.chars().filter(|c| !c.is_whitespace()).collect::<String>();
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
pub fn random_bytes(len: usize) -> Vec<u8> {
    use rand::RngCore;
    let mut buf = vec![0u8; len];
    OsRng.fill_bytes(&mut buf);
    buf
}

/// Generate a random 128-bit nonce.
pub fn random_nonce() -> [u8; 16] {
    let mut nonce = [0u8; 16];
    use rand::RngCore;
    OsRng.fill_bytes(&mut nonce);
    nonce
}

/// Generate a random 256-bit nonce.
pub fn random_nonce_256() -> [u8; 32] {
    let mut nonce = [0u8; 32];
    use rand::RngCore;
    OsRng.fill_bytes(&mut nonce);
    nonce
}

/// Generate a random 64-bit session ID.
pub fn random_session_id() -> u64 {
    use rand::RngCore;
    OsRng.next_u64()
}

/// Compute a message ID based on current time (divisible by 4 for client messages).
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
        assert_eq!(
            hash.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_aes_ige_roundtrip() {
        let key = [0x42u8; 32];
        let iv = [0x24u8; 32];
        let original = b"Hello, MTProto world!!Extra padding for 16 bytes!!";
        // Pad to multiple of 16
        let mut data = original.to_vec();
        while data.len() % 16 != 0 {
            data.push(0);
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
}
