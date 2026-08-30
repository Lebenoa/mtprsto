//! Photo, PhotoSize, FileLocation, Document, WebPage, GeoPoint.

use super::*;
use crate::error::{Error, Result};
use crate::serialize::TLReader;
#[allow(unused_imports)]
use std::fmt;

// §7 Photo, Document, and related types
// ===========================================================================

/// Telegram photo.
#[derive(Debug, Clone)]
pub enum Photo {
    Photo {
        id: PhotoId,
        access_hash: AccessHash,
        file_reference: Vec<u8>,
        dates: PhotoDateInfo,
        sizes: Vec<PhotoSize>,
    },
    Empty { id: PhotoId },
}

impl Photo {
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            PHOTO => {
                let _flags = r.read_i32()?;
                let id = PhotoId(r.read_i64()?);
                let access_hash = AccessHash(r.read_i64()?);
                let file_reference = r.read_bytes()?;
                let _dc_id = r.read_i32()?;
                let _w = r.read_i32()?;
                let _h = r.read_i32()?;
                // Skip remaining fields for now
                while r.remaining() > 0 {
                    let _ = r.read_i32()?;
                }
                Ok(Photo::Photo {
                    id, access_hash, file_reference,
                    dates: PhotoDateInfo::default(),
                    sizes: Vec::new(),
                })
            }
            PHOTO_EMPTY => {
                let id = PhotoId(r.read_i64()?);
                Ok(Photo::Empty { id })
            }
            other => Err(Error::Serialization(format!(
                "unknown Photo constructor {other:#x}"
            ))),
        }
    }
}

/// Photo date info (simplified).
#[derive(Debug, Clone, Default)]
pub struct PhotoDateInfo {
    pub date: i32,
}

/// Photo size variants.
#[derive(Debug, Clone)]
pub enum PhotoSize {
    Size { type_: String, location: FileLocation, w: i32, h: i32, size: i32 },
    Cached { type_: String, location: FileLocation, size: i32 },
    Stripped { type_: String, bytes: Vec<u8> },
    Empty { type_: String },
}

/// File location reference.
#[derive(Debug, Clone)]
pub enum FileLocation {
    VolumeId { volume_id: i64, local_id: i32, secret: i64, reference: Vec<u8>, dc_id: i32 },
    /// `inputDocumentFileLocation#bad07584` — documents/files. Build from
    /// a fetched [`Document::Document`]. Empty `thumb_size` downloads the
    /// full file; a value like "m" fetches a thumbnail.
    Document { id: i64, access_hash: i64, reference: Vec<u8>, thumb_size: String, dc_id: i32 },
    Web { dc_id: i32, url: String, size: i32 },
    EmojiStickerSet { version: i32, set_id: i64 },
    Unknown,
}

/// Telegram document (files, stickers, etc.).
#[derive(Debug, Clone)]
pub enum Document {
    Document {
        id: DocumentId,
        access_hash: AccessHash,
        file_reference: Vec<u8>,
        date: i32,
        mime_type: String,
        size: i64,
        thumb: Option<PhotoSize>,
        dc_id: i32,
        version: i32,
    },
    Empty { id: DocumentId, access_hash: AccessHash, file_reference: Vec<u8> },
}

impl Document {
    pub fn id(&self) -> DocumentId {
        match self {
            Document::Document { id, .. } | Document::Empty { id, .. } => *id,
        }
    }

    /// Download location for this document (for
    /// [`crate::Client::download`]). `None` for empty/placeholder docs.
    pub fn location(&self) -> Option<FileLocation> {
        match self {
            Document::Document { id, access_hash, file_reference, dc_id, .. } => {
                Some(FileLocation::Document {
                    id: id.0,
                    access_hash: access_hash.0,
                    reference: file_reference.clone(),
                    thumb_size: String::new(),
                    dc_id: *dc_id,
                })
            }
            Document::Empty { .. } => None,
        }
    }

    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            DOCUMENT => {
                let flags = r.read_i32()?;
                let id = DocumentId(r.read_i64()?);
                let access_hash = AccessHash(r.read_i64()?);
                let file_reference = r.read_bytes()?;
                let date = r.read_i32()?;
                let mime_type = String::from_utf8(r.read_bytes()?)?;
                let size = r.read_i64()?;
                // thumbs:flags.0?Vector<PhotoSize>
                let thumb: Option<PhotoSize> = if flags & (1 << 0) != 0 {
                    crate::types::read_photo_sizes(r)?
                        .into_iter()
                        .find_map(|s| match s {
                            crate::types::PhotoSizeFull::Size { type_, w, h, size } => {
                                Some(PhotoSize::Size { type_, location: FileLocation::Unknown, w, h, size })
                            }
                            crate::types::PhotoSizeFull::Stripped { type_, bytes } => {
                                Some(PhotoSize::Stripped { type_, bytes })
                            }
                            other => {
                                let _ = other;
                                None
                            }
                        })
                } else {
                    None
                };
                // video_thumbs:flags.1?Vector<VideoSize> — VideoSize not
                // modelled yet; skip via per-element length is impossible,
                // so track presence only (tail marked below).
                let has_video_thumbs = flags & (1 << 1) != 0;
                if has_video_thumbs {
                    // Consume the vector header + raw body is unsafe without
                    // a VideoSize parser; count and size are knowable only
                    // per element. Leave the body in place and note it.
                    //
                    // In practice documents inside messages are followed by
                    // dc_id:int attributes:Vector<...> so we cannot skip;
                    // dc_id/version/attributes are read below only when the
                    // vector was absent.
                }
                let dc_id = if has_video_thumbs { 0 } else { r.read_i32()? };
                let version = if has_video_thumbs { 0 } else { r.read_i32()? };
                let _attributes = if has_video_thumbs {
                    Vec::new()
                } else {
                    crate::types::read_document_attributes(r)?
                };
                Ok(Document::Document {
                    id, access_hash, file_reference, date, mime_type, size,
                    thumb, dc_id, version,
                })
            }
            DOCUMENT_EMPTY => {
                let id = DocumentId(r.read_i64()?);
                Ok(Document::Empty { id, access_hash: AccessHash(0), file_reference: Vec::new() })
            }
            other => Err(Error::Serialization(format!(
                "unknown Document constructor {other:#x}"
            ))),
        }
    }
}

/// Web page preview.
#[derive(Debug, Clone)]
pub enum WebPage {
    Empty { id: i64 },
    WebPage { id: i64, url: String, display_type: String, description: Option<String> },
    Instant { id: i64, short_name: String, description: String },
}

impl WebPage {
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let _ctor = r.read_u32()?;
        let id = r.read_i64()?;
        while r.remaining() > 0 {
            let _ = r.read_i32()?;
        }
        Ok(WebPage::Empty { id })
    }
}

/// Geo point.
#[derive(Debug, Clone)]
pub struct GeoPoint {
    pub long: f64,
    pub lat: f64,
}

impl GeoPoint {
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let _flags = r.read_i32()?;
        let long_bits = r.read_u64()?;
        let lat_bits = r.read_u64()?;
        Ok(GeoPoint {
            long: f64::from_bits(long_bits),
            lat: f64::from_bits(lat_bits),
        })
    }
}

// ===========================================================================
