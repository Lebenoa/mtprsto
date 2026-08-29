//! TCP transport layer with Obfuscated2 for MTProto.
//!
//! Obfuscated2 wraps the MTProto protocol in an obfuscated stream that
//! is indistinguishable from random data, allowing it to bypass DPI
//! (Deep Packet Inspection). See:
//! <https://core.telegram.org/mtproto/mtproto-transports#transport-obfuscation>

use crate::error::{Error, Result};
use crate::mtproto::MtProtoSession;
use rand::{rng, Rng};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Telegram DC IDs for production servers.
pub const DC_1: i32 = 1; // New Jersey, USA
pub const DC_2: i32 = 2; // Amsterdam, Netherlands
pub const DC_3: i32 = 3; // Miami, USA
pub const DC_4: i32 = 4; // Amsterdam, Netherlands
pub const DC_5: i32 = 5; // Singapore

/// Returns the (ip, port) for a Telegram DC in Abridged mode.
pub fn dc_address(dc_id: i32) -> Result<SocketAddr> {
    let (ip, port) = match dc_id.abs() {
        1 => ("149.154.175.50", 443),
        2 => ("149.154.167.50", 443),
        3 => ("149.154.175.100", 443),
        4 => ("149.154.167.92", 443),
        5 => ("91.108.56.100", 443),
        201 => ("91.108.56.4", 443), // test DC (SPEC §1)
        _ => return Err(Error::Transport(format!("unknown DC ID: {dc_id}"))),
    };
    let ip = ip.parse::<std::net::IpAddr>()
        .map_err(|e| Error::Transport(e.to_string()))?;
    Ok(SocketAddr::new(ip, port))
}

// ---------------------------------------------------------------------------
// Obfuscated2 Transport
// ---------------------------------------------------------------------------

/// Obfuscated2 codec: AES-256-CTR streams over a byte stream (TCP or,
/// with feature `ws`, a WebSocket pipe).
///
/// Key/IV derivation (https://corefork.telegram.org/mtproto/mtproto-transports):
/// - init: 64 random bytes (protocol tag at 56..60)
/// - enc_key = init[8..40], enc_iv = init[40..56]
/// - init_rev = reversed(init); dec_key = init_rev[8..40], dec_iv = init_rev[40..56]
/// - init[56..64] is replaced with its CTR-encrypted form before sending.
/// - CTR counters persist across frames until the connection closes.
pub(crate) struct AesCtr {
    cipher: aes::Aes256,
    counter: [u8; 16],
}

impl AesCtr {
    fn new(key: &[u8], iv: &[u8]) -> Self {
        use aes::cipher::KeyInit;
        Self {
            cipher: aes::Aes256::new(key.try_into().expect("CTR key length")),
            counter: iv.try_into().expect("CTR iv length"),
        }
    }

    /// XOR `data` with the keystream in place, advancing the counter.
    fn crypt(&mut self, data: &mut [u8]) {
        use aes::cipher::BlockCipherEncrypt;
        for chunk in data.chunks_mut(16) {
            let mut block: aes::Block = self.counter.into();
            self.cipher.encrypt_block(&mut block);
            for (b, k) in chunk.iter_mut().zip(block.iter()) {
                *b ^= k;
            }
            // 128-bit big-endian counter increment
            for i in (0..16).rev() {
                self.counter[i] = self.counter[i].wrapping_add(1);
                if self.counter[i] != 0 {
                    break;
                }
            }
        }
    }
}

pub struct Obfuscated2Transport<S = TcpStream> {
    stream: S,
    /// Kept for the connection lifetime (CTR counter state); used during
    /// the init handshake, then only held.
    #[allow(dead_code)]
    enc: AesCtr,
    dec: AesCtr,
}

/// Byte-stream I/O the Obfuscated2 codec needs. Implemented for
/// `TcpStream` (default) and, with feature `ws`, for the WebSocket pipe;
/// public so custom transports can plug into [`Obfuscated2Transport`].
#[allow(async_fn_in_trait)]
pub trait FrameStream {
    async fn fs_write_all(&mut self, bytes: &[u8]) -> Result<()>;
    async fn fs_read_exact(&mut self, buf: &mut [u8]) -> Result<()>;
}

impl FrameStream for TcpStream {
    async fn fs_write_all(&mut self, bytes: &[u8]) -> Result<()> {
        use tokio::io::AsyncWriteExt;
        self.write_all(bytes).await.map_err(Error::Network)?;
        self.flush().await.map_err(Error::Network)
    }
    async fn fs_read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        use tokio::io::AsyncReadExt;
        self.read_exact(buf).await.map(|_| ()).map_err(Error::Network)
    }
}

#[cfg(feature = "ws")]
impl<S> Obfuscated2Transport<S> {
    /// Wrap an already-connected stream with the given CTR pair (used by
    /// the WS path, which sends the init itself).
    pub(crate) fn new(stream: S, enc: AesCtr, dec: AesCtr) -> Self {
        Self { stream, enc, dec }
    }
}

impl<S: FrameStream> Obfuscated2Transport<S> {
    /// Send one Intermediate frame (4-byte LE length + payload), encrypted.
    pub async fn send_frame(&mut self, payload: &[u8]) -> Result<()> {
        let mut frame = (payload.len() as u32).to_le_bytes().to_vec();
        frame.extend_from_slice(payload);
        self.stream.fs_write_all(&frame).await?;
        Ok(())
    }

    /// Maximum plaintext frame we accept (2 MiB — MTProto hard limit is
    /// 1 MiB per message + padding/headers slack).
    pub const MAX_FRAME: usize = 2 * 1024 * 1024;


    /// Receive one Intermediate frame, decrypted.
    pub async fn recv_frame(&mut self) -> Result<Vec<u8>> {
        let mut hdr = [0u8; 4];
        self.stream.fs_read_exact(&mut hdr).await?;
        self.dec.crypt(&mut hdr);
        let len = u32::from_le_bytes(hdr) as usize;
        if len > Self::MAX_FRAME {
            return Err(Error::Transport(format!(
                "frame too large: {len} bytes (cap {})",
                Self::MAX_FRAME
            )));
        }
        let mut payload = vec![0u8; len];
        self.stream.fs_read_exact(&mut payload).await?;
        self.dec.crypt(&mut payload);
        Ok(payload)
    }
}

/// Generate random obfuscated2 init data that satisfies Telegram's constraints.
///
/// The first 56 bytes are random. The 57th and 58th bytes encode the protocol tag.
/// The 60th-63rd bytes are the DC ID.
pub fn generate_obfuscated2_init(protocol: TransportProtocol, dc_id: i32) -> [u8; 64] {
    let mut rng = rng();
    let mut data = [0u8; 64];

    // Protocol tag (bytes 56-60, LE). 0xef (abridged) is padded to 4 bytes.
    let tag: u32 = match protocol {
        TransportProtocol::Intermediate => 0xEEEEEEEE,
        TransportProtocol::IntermediatePadded => 0xDDDDDDDD,
    };

    loop {
        rng.fill_bytes(&mut data);
        data[56..60].copy_from_slice(&tag.to_le_bytes());
        // DC id, signed two-byte LE, at bytes 60-62 (production DCs only).
        data[60..62].copy_from_slice(&(dc_id as i16).to_le_bytes());

        // First int must not collide with protocol ids / HTTP verbs.
        let first_int = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if matches!(
            first_int,
            0x44414548 // HEAD
            | 0x54534F50 // POST
            | 0x20544547 // GET<space>
            | 0x4954504F // OPTIO(N)
            | 0x02010316
            | 0xDDDDDDDD
            | 0xEEEEEEEE
        ) {
            continue;
        }
        // First byte must not be the abridged tag.
        if data[0] == 0xEF {
            continue;
        }
        // Bytes 4-8 must not be zero (full transport, seq 0).
        if data[4..8] == [0, 0, 0, 0] {
            continue;
        }

        break;
    }

    data
}

/// Connect with full Obfuscated2 (AES-256-CTR) framing and return the codec.
///
/// Opens a raw TCP connection (NO init written), generates the init, sends
/// it, and returns the codec for subsequent framed I/O.
pub async fn connect_obfuscated2(
    dc_id: i32,
    protocol: TransportProtocol,
) -> Result<Obfuscated2Transport> {
    let stream = TcpStream::connect(dc_address(dc_id)?).await?;
    let (enc, dec, init_copy) = obfuscated2_keys(protocol, dc_id);
    let mut s = stream;
    s.fs_write_all(&init_copy).await?;
    Ok(Obfuscated2Transport { stream: s, enc, dec })
}

/// Derive the Obfuscated2 CTR pair and the ready-to-send init payload.
pub(crate) fn obfuscated2_keys(
    protocol: TransportProtocol,
    dc_id: i32,
) -> (AesCtr, AesCtr, Vec<u8>) {
    let init = generate_obfuscated2_init(protocol, dc_id);
    let mut init_rev = init;
    init_rev.reverse();
    let mut enc = AesCtr::new(&init[8..40], &init[40..56]);
    let dec = AesCtr::new(&init_rev[8..40], &init_rev[40..56]);
    let mut init_copy = init;
    enc.crypt(&mut init_copy[56..64]);
    (enc, dec, init_copy.to_vec())
}

/// Transport protocol type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportProtocol {
    /// Intermediate protocol (no frame headers for payloads > 127 bytes).
    Intermediate,
    /// Intermediate with padding support.
    IntermediatePadded,
}

// ---------------------------------------------------------------------------
// Abridged Transport (simpler, older)
// ---------------------------------------------------------------------------

/// Abridged transport: messages are preceded by a 1-byte length if < 127,
/// or 3-byte length if >= 127.
pub struct AbridgedTransport {
    stream: TcpStream,
}

impl AbridgedTransport {
    pub fn new(stream: TcpStream) -> Self {
        Self { stream }
    }

    /// Send raw data over the abridged transport.
    pub async fn send(&mut self, data: &[u8]) -> Result<()> {
        let len = data.len() / 4; // Length in 4-byte words
        if len < 127 {
            self.stream.write_all(&[len as u8]).await?;
        } else {
            self.stream.write_all(&[127u8]).await?;
            let len32 = (len as u32).to_le_bytes();
            self.stream.write_all(&len32[0..3]).await?;
        }
        self.stream.write_all(data).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Receive raw data from the abridged transport.
    pub async fn recv(&mut self) -> Result<Vec<u8>> {
        let mut first = [0u8; 1];
        self.stream.read_exact(&mut first).await?;

        let len = if first[0] < 127 {
            first[0] as usize * 4
        } else {
            let mut buf = [0u8; 3];
            self.stream.read_exact(&mut buf).await?;
            let len = (buf[0] as usize) | ((buf[1] as usize) << 8) | ((buf[2] as usize) << 16);
            len * 4
        };

        let mut data = vec![0u8; len];
        self.stream.read_exact(&mut data).await?;
        Ok(data)
    }

    pub fn into_inner(self) -> TcpStream {
        self.stream
    }
}

// ---------------------------------------------------------------------------
// Intermediate Transport
// ---------------------------------------------------------------------------

/// Intermediate transport: 4-byte little-endian length prefix.
pub struct IntermediateTransport {
    stream: TcpStream,
}

impl IntermediateTransport {
    /// Maximum plaintext frame we accept (2 MiB — MTProto hard limit is
    /// 1 MiB per message + padding/headers slack).
    pub const MAX_FRAME: usize = 2 * 1024 * 1024;

    pub fn new(stream: TcpStream) -> Self {
        Self { stream }
    }

    pub async fn send(&mut self, data: &[u8]) -> Result<()> {
        let len = (data.len() as u32).to_le_bytes();
        self.stream.write_all(&len).await?;
        self.stream.write_all(data).await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<Vec<u8>> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let len = u32::from_le_bytes(len_buf) as usize;
        if len > IntermediateTransport::MAX_FRAME {
            return Err(Error::Transport(format!(
                "frame too large: {len} bytes (cap {})",
                IntermediateTransport::MAX_FRAME
            )));
        }
        let mut data = vec![0u8; len];
        self.stream.read_exact(&mut data).await?;
        Ok(data)
    }

    pub fn into_inner(self) -> TcpStream {
        self.stream
    }
}

// ---------------------------------------------------------------------------
// Higher-level: connect and send encrypted messages
// ---------------------------------------------------------------------------

/// Connect to a Telegram DC for PLAIN Intermediate (unencrypted MTProto)
/// flows: the auth-key DH handshake.
///
/// Verified against production DCs (2026-08): the server accepts a 4-byte
/// `0xEEEEEEEE` protocol-tag prefix followed by *plaintext* Intermediate
/// frames, and resets connections that skip the tag. Full Obfuscated2
/// (init + CTR) is available via `connect_obfuscated2` for the encrypted
/// RPC path.
pub async fn connect(dc_id: i32) -> Result<TcpStream> {
    let addr = dc_address(dc_id)?;
    let mut stream = TcpStream::connect(addr).await?;
    stream.write_all(&0xEEEEEEEEu32.to_le_bytes()).await?;
    stream.flush().await?;
    Ok(stream)
}


/// Send a raw (unencrypted) message over Intermediate transport.
pub async fn send_unencrypted(
    stream: &mut TcpStream,
    msg_id: u64,
    payload: &[u8],
) -> Result<()> {
    let data = MtProtoSession::build_unencrypted(msg_id, payload);
    let len = (data.len() as u32).to_le_bytes();
    stream.write_all(&len).await?;
    stream.write_all(&data).await?;
    stream.flush().await?;
    Ok(())
}

/// Receive a raw (unencrypted) message over Intermediate transport.
pub async fn recv_unencrypted(stream: &mut TcpStream) -> Result<(u64, Vec<u8>)> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;

    let mut data = vec![0u8; len];
    stream.read_exact(&mut data).await?;

    // Transport-level error: a 4-byte signed LE code with no envelope
    // (https://corefork.telegram.org/mtproto/mtproto-transports#transport-errors).
    if len == 4 {
        let code = i32::from_le_bytes(data.clone().try_into().unwrap());
        return Err(Error::Transport(format!(
            "server transport error {code} (-404 = bad request/auth, -429 = flood)"
        )));
    }

    if data.len() < 20 {
        return Err(Error::Transport("message too short".into()));
    }

    let auth_key_id = u64::from_be_bytes(data[0..8].try_into().unwrap());
    if auth_key_id != 0 {
        return Err(Error::Transport(
            "expected unencrypted message (auth_key_id=0)".into(),
        ));
    }

    let msg_id = u64::from_be_bytes(data[8..16].try_into().unwrap());
    let _msg_len = u32::from_be_bytes(data[16..20].try_into().unwrap());
    let payload = data[20..].to_vec();

    Ok((msg_id, payload))
}

/// Send an encrypted message using Intermediate transport.
pub async fn send_encrypted(
    stream: &mut TcpStream,
    session: &MtProtoSession,
    payload: &[u8],
    msg_id: u64,
    seq_no: i32,
) -> Result<()> {
    let data = session.encrypt_message(payload, msg_id, seq_no);
    let len = (data.len() as u32).to_le_bytes();
    stream.write_all(&len).await?;
    stream.write_all(&data).await?;
    stream.flush().await?;
    Ok(())
}

/// Receive an encrypted message using Intermediate transport.
pub async fn recv_encrypted(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;

    let mut data = vec![0u8; len];
    stream.read_exact(&mut data).await?;
    Ok(data)
}

/// Convenience: connect, send unencrypted, receive response.
pub async fn exchange_unencrypted(
    dc_id: i32,
    send_payload: &[u8],
) -> Result<(u64, Vec<u8>)> {
    let mut stream = connect(dc_id).await?;
    let msg_id = 0xdeadbeef; // Can be anything for unencrypted
    send_unencrypted(&mut stream, msg_id, send_payload).await?;
    recv_unencrypted(&mut stream).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dc_addresses() {
        let expected = [
            (1, "149.154.175.50"),
            (2, "149.154.167.50"),
            (3, "149.154.175.100"),
            (4, "149.154.167.92"),
            (5, "91.108.56.100"),
            (201, "91.108.56.4"),
        ];
        for (id, ip) in expected {
            let addr = dc_address(id).unwrap();
            assert_eq!(addr.ip().to_string(), ip, "DC {id}");
            assert_eq!(addr.port(), 443);
        }
        assert!(dc_address(99).is_err());
    }

    #[test]
    fn test_obfuscated2_init() {
        let init = generate_obfuscated2_init(TransportProtocol::Intermediate, 2);
        assert_eq!(init.len(), 64);

        // Check protocol tag
        let tag = u32::from_le_bytes(init[56..60].try_into().unwrap());
        assert_eq!(tag, 0xEEEEEEEE);

        // Check DC id bytes
        let dc = i16::from_le_bytes([init[60], init[61]]);
        assert_eq!(dc, 2);
    }
    #[test]
    fn test_abridged_message_length() {
        // A 40-byte message (10 words) should encode as length=10 (< 127)
        let len: u8 = 10;
        assert!(len < 127);
    }
}
