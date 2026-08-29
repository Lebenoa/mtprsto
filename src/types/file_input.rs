//! InputFile / InputDocument upload references.

use super::*;
use crate::serialize::TLWriter;
#[allow(unused_imports)]
use std::fmt;

// §7 File input types
// ===========================================================================

/// Reference to a file for upload.
#[derive(Debug, Clone)]
pub enum InputFile {
    /// File from local disk.
    Id { id: i64, parts: i32, name: String, md5_checksum: Option<String> },
    /// Large file from local disk (>10MB, sent in parts).
    Big { id: i64, parts: i32, name: String },
    /// Partial file from CDN.
    Cd { id: i64, file_chain_id: i32, file_chain_part: i32 },
    /// File from a URL.
    Url { url: String },
}

impl InputFile {
    pub fn write_to(&self, w: &mut TLWriter) {
        match self {
            InputFile::Id { id, parts, name, md5_checksum } => {
                // inputFile#f52ff27f id:long parts:int name:string md5_checksum:string
                // md5_checksum is UNCONDITIONAL (no flags field).
                w.write_u32(INPUT_FILE);
                w.write_i64(*id);
                w.write_i32(*parts);
                w.write_bytes(name.as_bytes());
                w.write_bytes(md5_checksum.as_deref().unwrap_or("").as_bytes());
            }
            InputFile::Big { id, parts, name } => {
                w.write_u32(INPUT_FILE_BIG);
                w.write_i64(*id);
                w.write_i32(*parts);
                w.write_bytes(name.as_bytes());
            }
            _ => {}
        }
    }
}

/// Reference to a file for download (the document attachment on a message).
#[derive(Debug, Clone)]
pub enum InputDocument {
    /// Standard document reference.
    Document { id: DocumentId, access_hash: AccessHash, file_reference: Vec<u8> },
    /// Empty/missing document.
    Empty,
}

impl InputDocument {
    pub fn write_to(&self, w: &mut TLWriter) {
        match self {
            InputDocument::Document { id, access_hash, file_reference } => {
                w.write_u32(INPUT_DOCUMENT);
                w.write_i64(id.0);
                w.write_i64(access_hash.0);
                w.write_bytes(file_reference);
            }
            InputDocument::Empty => {
                w.write_u32(INPUT_DOCUMENT_EMPTY);
                w.write_i64(0);
            }
        }
    }
}

// ===========================================================================
