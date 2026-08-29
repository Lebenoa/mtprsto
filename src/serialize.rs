//! TL (Type Language) binary serialization for MTProto.
//!
//! Implements the serialization format described at
//! <https://core.telegram.org/mtproto/serialize>.
//!
//! Elementary types:
//! - `int` (32-bit, little-endian)
//! - `long` (64-bit, little-endian)
//! - `int128` (128-bit, little-endian)
//! - `int256` (256-bit, little-endian)
//! - `string` (length-prefixed, padded to 4 bytes)
//! - `boolTrue` / `boolFalse` — boxed Bool constructors

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// A buffer that grows as TL values are serialized into it.
#[derive(Debug, Default, Clone)]
pub struct TLWriter {
    buf: Vec<u8>,
}

impl TLWriter {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Returns `true` when nothing has been written yet.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    // --- elementary writes ---

    pub fn write_i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn write_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn write_i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Write a bare TL `double` (8-byte IEEE 754, little-endian on the wire).
    pub fn write_double(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn write_u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn write_i128(&mut self, v: u128) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn write_u128(&mut self, v: [u8; 16]) {
        self.buf.extend_from_slice(&v);
    }

    pub fn write_u256(&mut self, v: [u8; 32]) {
        self.buf.extend_from_slice(&v);
    }

    pub fn write_raw_bytes(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Write a bare TL `string` (length-prefixed, padded to 4-byte alignment).
    pub fn write_bytes(&mut self, data: &[u8]) {
        let len = data.len();
        if len <= 252 {
            self.buf.push(len as u8);
        } else {
            self.buf.push(254);
            self.buf.push((len & 0xFF) as u8);
            self.buf.push(((len >> 8) & 0xFF) as u8);
            self.buf.push(((len >> 16) & 0xFF) as u8);
        }
        self.buf.extend_from_slice(data);
        // Pad to 4-byte alignment
        let pad = 4 - (self.buf.len() % 4);
        if pad != 4 {
            self.buf.resize(self.buf.len() + pad, 0);
        }
    }

    /// Write a constructor ID (TL combinator ID as u32).
    pub fn write_constructor_id(&mut self, id: u32) {
        self.write_u32(id);
    }

    /// Write the BoolTrue constructor.
    pub fn write_bool_true(&mut self) {
        self.write_u32(BOOL_TRUE);
    }

    /// Write the BoolFalse constructor.
    pub fn write_bool_false(&mut self) {
        self.write_u32(BOOL_FALSE);
    }

    /// Write a TL `Vector<int>` (box of int vector).
    pub fn write_vector_int(&mut self, items: &[i32]) {
        // vector#1cb5c415 count:Vector<int> = Vector<int>;
        self.write_u32(VECTOR);
        self.write_i32(items.len() as i32);
        for &item in items {
            self.write_i32(item);
        }
    }

    /// Write a TL `Vector<long>` (box of long vector).
    pub fn write_vector_long(&mut self, items: &[i64]) {
        self.write_u32(VECTOR);
        self.write_i32(items.len() as i32);
        for &item in items {
            self.write_i64(item);
        }
    }

    /// Write a nullable int (int? = Int).
    pub fn write_nullable_int(&mut self, v: Option<i32>) {
        match v {
            Some(v) => self.write_i32(v),
            None => self.write_i32(0x14377020), // null#14377020 = Null;
        }
    }
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

/// A cursor over a byte slice for deserializing TL values.
#[derive(Debug)]
pub struct TLReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> TLReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    fn ensure(&self, n: usize) -> Result<()> {
        if self.remaining() < n {
            Err(Error::Serialization(format!(
                "need {n} bytes but only {} remaining at position {}",
                self.remaining(),
                self.pos
            )))
        } else {
            Ok(())
        }
    }

    // --- elementary reads ---

    pub fn read_i32(&mut self) -> Result<i32> {
        self.ensure(4)?;
        let val = i32::from_le_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(val)
    }

    pub fn read_u32(&mut self) -> Result<u32> {
        self.ensure(4)?;
        let val = u32::from_le_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(val)
    }

    pub fn read_i64(&mut self) -> Result<i64> {
        self.ensure(8)?;
        let val = i64::from_le_bytes(self.data[self.pos..self.pos + 8].try_into().unwrap());
        self.pos += 8;
        Ok(val)
    }

    pub fn read_u64(&mut self) -> Result<u64> {
        self.ensure(8)?;
        let val = u64::from_le_bytes(self.data[self.pos..self.pos + 8].try_into().unwrap());
        self.pos += 8;
        Ok(val)
    }

    pub fn read_u128(&mut self) -> Result<u128> {
        self.ensure(16)?;
        let val = u128::from_le_bytes(self.data[self.pos..self.pos + 16].try_into().unwrap());
        self.pos += 16;
        Ok(val)
    }

    pub fn read_i128_bytes(&mut self) -> Result<[u8; 16]> {
        self.ensure(16)?;
        let mut buf = [0u8; 16];
        buf.copy_from_slice(&self.data[self.pos..self.pos + 16]);
        self.pos += 16;
        Ok(buf)
    }

    pub fn read_u256_bytes(&mut self) -> Result<[u8; 32]> {
        self.ensure(32)?;
        let mut buf = [0u8; 32];
        buf.copy_from_slice(&self.data[self.pos..self.pos + 32]);
        self.pos += 32;
        Ok(buf)
    }

    /// Read a bare TL `string` (length-prefixed, padded to 4-byte
    /// alignment — padding is computed from the absolute stream offset,
    /// matching `TLWriter::write_bytes`).
    pub fn read_bytes(&mut self) -> Result<Vec<u8>> {
        self.ensure(1)?;
        let first = self.data[self.pos];
        let (len, skip) = if first <= 252 {
            (first as usize, 1usize)
        } else if first == 254 {
            self.ensure(4)?;
            let b1 = self.data[self.pos + 1] as usize;
            let b2 = self.data[self.pos + 2] as usize;
            let b3 = self.data[self.pos + 3] as usize;
            (b1 | (b2 << 8) | (b3 << 16), 4)
        } else {
            return Err(Error::Serialization("invalid string length byte".into()));
        };

        self.pos += skip;
        self.ensure(len)?;
        let mut buf = vec![0u8; len];
        buf.copy_from_slice(&self.data[self.pos..self.pos + len]);
        self.pos += len;

        // Skip padding so the stream position is 4-byte aligned again.
        let pad = (4 - (self.pos % 4)) % 4;
        self.pos += pad;

        Ok(buf)
    }

    /// Peek at the next 4 bytes as a constructor ID without advancing.
    pub fn peek_constructor_id(&self) -> Result<u32> {
        self.ensure(4)?;
        Ok(u32::from_le_bytes(
            self.data[self.pos..self.pos + 4].try_into().unwrap(),
        ))
    }

    /// Skip `n` bytes.
    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.ensure(n)?;
        self.pos += n;
        Ok(())
    }

    /// Read a TL `Vector<T>` header and return the element count.
    /// The caller then reads `count` elements directly from this reader.
    pub fn read_vector_header(&mut self) -> Result<i32> {
        let ctor = self.read_u32()?;
        if ctor != VECTOR {
            return Err(crate::error::Error::Serialization(format!(
                "expected vector constructor, got {ctor:#x}"
            )));
        }
        self.read_i32()
    }
}

// ---------------------------------------------------------------------------
// TL Constructor ID computation (CRC32 of description string)
// ---------------------------------------------------------------------------

use crate::crypto::crc32;

/// Compute a TL constructor ID from its description string.
///
/// The ID is the CRC32 of the description, which always falls in 0x01000000..0xFFFFFF00.
pub fn constructor_id(description: &str) -> u32 {
    crc32(description.as_bytes())
}

// ---------------------------------------------------------------------------
// Common TL constructor IDs
// ---------------------------------------------------------------------------

// ResPQ
pub const RES_PQ: u32 = 0x05162463;

// P_Q_inner_data
pub const P_Q_INNER_DATA: u32 = 0x83c95aec;
pub const P_Q_INNER_DATA_DC: u32 = 0xa9f55f95;
pub const P_Q_INNER_DATA_TEMP: u32 = 0x3c6a84d4;
pub const P_Q_INNER_DATA_TEMP_DC: u32 = 0x56fddf88;

// Server_DH_Params
pub const SERVER_DH_PARAMS_OK: u32 = 0xd0e8075c;
pub const SERVER_DH_PARAMS_FAIL: u32 = 0x79cbcd00;

// Server_DH_inner_data
pub const SERVER_DH_INNER_DATA: u32 = 0xb5890dba;

// Client_DH_inner_data
pub const CLIENT_DH_INNER_DATA: u32 = 0x6643b654;

// Set_client_DH_params_answer
pub const DH_GEN_OK: u32 = 0x3bcbf734;
pub const DH_GEN_RETRY: u32 = 0x46dc1fb9;
pub const DH_GEN_FAIL: u32 = 0xa69dae02;

// req_pq_multi
pub const REQ_PQ_MULTI: u32 = 0xbe7e8ef1;

// req_DH_params
pub const REQ_DH_PARAMS: u32 = 0xd712e4be;

// set_client_DH_params
pub const SET_CLIENT_DH_PARAMS: u32 = 0xf5045f1f;

// MTProto message containers
pub const MSG_CONTAINER: u32 = 0x73f1f8dc;
pub const MSG_COPY: u32 = 0xe06046b2;

// gzip_packed
pub const GZIP_PACKED: u32 = 0x3072cfa1;

// ping / pong
pub const PING: u32 = 0x7abe77ec;
pub const PONG: u32 = 0x347773c5;

// msgs_ack
pub const MSGS_ACK: u32 = 0x62d6b459;

// new_session_created
pub const NEW_SESSION_CREATED: u32 = 0x9ec209d4;

// bad_msg_notification
pub const BAD_MSG_NOTIFICATION: u32 = 0xa7eff811;

// bad_server_salt
pub const BAD_SERVER_SALT: u32 = 0xedab447b;
// new_server_salt#1160b89c new_server_salt:long
pub const NEW_SERVER_SALT: u32 = 0x1160b89c;


// future_salt
// getFutureSalts#b921bd04 num:int — request; reply is future_salts
pub const FUTURE_SALTS_REQUEST: u32 = 0xb921bd04;
pub const FUTURE_SALT: u32 = 0x0949dfe1;

// future_salts
pub const FUTURE_SALTS: u32 = 0xae500895;

// msgs_state_req#da69fb52 msg_ids:Vector<long>
pub const MSGS_STATE_REQ: u32 = 0xda69fb52;

// msgs_state_info#04deb57d req_msg_id:long info:bytes
pub const MSGS_STATE_INFO: u32 = 0x04deb57d;

// msgs_all_info#8cc0d131 info:bytes msg_ids:Vector<long>
pub const MSGS_ALL_INFO: u32 = 0x8cc0d131;

// msg_resend_req#7d861a08 msg_ids:Vector<long>
pub const MSGS_RESEND_REQ: u32 = 0x7d861a08;

// rpc_result
pub const RPC_RESULT: u32 = 0xf35c6d01;

// rpc_error
pub const RPC_ERROR: u32 = 0x2144ca19;

// Bool
pub const BOOL_TRUE: u32 = 0x997275b5;
pub const BOOL_FALSE: u32 = 0xbc799737;

// Vector constructors
pub const VECTOR: u32 = 0x1cb5c415;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_read_i32() {
        let mut w = TLWriter::new();
        w.write_i32(42);
        let mut r = TLReader::new(w.as_bytes());
        assert_eq!(r.read_i32().unwrap(), 42);
    }

    #[test]
    fn test_write_read_string() {
        let mut w = TLWriter::new();
        let data = b"hello world";
        w.write_bytes(data);
        let mut r = TLReader::new(w.as_bytes());
        let read_back = r.read_bytes().unwrap();
        assert_eq!(read_back, data);
    }

    #[test]
    fn test_write_read_long_string() {
        let mut w = TLWriter::new();
        let data = vec![0xABu8; 300];
        w.write_bytes(&data);
        let mut r = TLReader::new(w.as_bytes());
        let read_back = r.read_bytes().unwrap();
        assert_eq!(read_back, data);
    }

    #[test]
    fn test_constructor_id_vector() {
        assert_eq!(constructor_id("vector t:Type # [ t ] = Vector t"), 0x1cb5c415);
    }

    #[test]
    fn test_write_read_i64() {
        let mut w = TLWriter::new();
        w.write_i64(0x1234567890ABCDEF);
        let mut r = TLReader::new(w.as_bytes());
        assert_eq!(r.read_i64().unwrap(), 0x1234567890ABCDEF);
    }

    #[test]
    fn test_read_bool_true() {
        let mut w = TLWriter::new();
        w.write_bool_true();
        let mut r = TLReader::new(w.as_bytes());
        assert_eq!(r.read_u32().unwrap(), BOOL_TRUE);
    }
}
