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
                // photo#fb197a65 flags:# has_stickers:flags.0?true id:long
                //   access_hash:long file_reference:bytes date:int
                //   sizes:Vector<PhotoSize> video_sizes:flags.1?Vector<VideoSize>
                //   dc_id:int
                let flags = r.read_i32()?;
                let id = PhotoId(r.read_i64()?);
                let access_hash = AccessHash(r.read_i64()?);
                let file_reference = r.read_bytes()?;
                let date = r.read_i32()?;
                let _sizes = crate::types::read_photo_sizes(r)?;
                if flags & (1 << 1) != 0 {
                    crate::types::skip_video_sizes(r)?;
                }
                let _dc_id = r.read_i32()?;
                Ok(Photo::Photo {
                    id, access_hash, file_reference,
                    dates: PhotoDateInfo { date },
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
                // document#8fd4c4d8 flags:# id:long access_hash:long
                //   file_reference:bytes date:int mime_type:string size:long
                //   thumbs:flags.0?Vector<PhotoSize>
                //   video_thumbs:flags.1?Vector<VideoSize> dc_id:int
                //   attributes:Vector<DocumentAttribute>
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
                // video_thumbs:flags.1?Vector<VideoSize>
                if flags & (1 << 1) != 0 {
                    crate::types::skip_video_sizes(r)?;
                }
                let dc_id = r.read_i32()?;
                let _attributes = crate::types::read_document_attributes(r)?;
                Ok(Document::Document {
                    id, access_hash, file_reference, date, mime_type, size,
                    thumb, dc_id, version: 0,
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
    /// `webPage#e89c45b2 flags:# ... id:long url:string display_url:string
    /// hash:int <conditionals through attributes:flags.12>` — skip all
    /// optional fields to stay stream-aligned.
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        if ctor != crate::types::WEB_PAGE {
            return Err(Error::Serialization(format!(
                "unknown WebPage constructor {ctor:#x}"
            )));
        }
        let flags = r.read_i32()?;
        let id = r.read_i64()?;
        let _url = r.read_bytes()?;
        let _display_url = r.read_bytes()?;
        let _hash = r.read_i32()?;
        if flags & (1 << 0) != 0 { let _type_ = r.read_bytes()?; }
        if flags & (1 << 1) != 0 { let _site_name = r.read_bytes()?; }
        if flags & (1 << 2) != 0 { let _title = r.read_bytes()?; }
        if flags & (1 << 3) != 0 { let _description = r.read_bytes()?; }
        if flags & (1 << 4) != 0 { Photo::read_from(r)?; }
        if flags & (1 << 5) != 0 {
            let _embed_url = r.read_bytes()?;
            let _embed_type = r.read_bytes()?;
        }
        if flags & (1 << 6) != 0 { let _embed_width = r.read_i32()?; }
        if flags & (1 << 6) != 0 { let _embed_height = r.read_i32()?; }
        if flags & (1 << 7) != 0 { let _duration = r.read_i32()?; }
        if flags & (1 << 8) != 0 { let _author = r.read_bytes()?; }
        if flags & (1 << 9) != 0 { Document::read_from(r)?; }
        if flags & (1 << 10) != 0 {
            return Err(Error::Serialization(
                "webPage cached_page (Page) not supported".into(),
            ));
        }
        if flags & (1 << 12) != 0 {
            return Err(Error::Serialization(
                "webPage attributes (WebPageAttribute) not supported".into(),
            ));
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
    /// `geoPoint#b2a2f663 flags:# long:double lat:double access_hash:long
    /// accuracy_radius:flags.0?int` / `geoPointEmpty#1117dd5f` (ctor included).
    pub fn read_from(r: &mut TLReader) -> Result<Self> {
        let ctor = r.read_u32()?;
        match ctor {
            crate::types::GEO_POINT => {
                let flags = r.read_i32()?;
                let long_bits = r.read_u64()?;
                let lat_bits = r.read_u64()?;
                let _access_hash = r.read_i64()?;
                if flags & (1 << 0) != 0 {
                    let _accuracy_radius = r.read_i32()?;
                }
                Ok(GeoPoint {
                    long: f64::from_bits(long_bits),
                    lat: f64::from_bits(lat_bits),
                })
            }
            crate::types::GEO_POINT_EMPTY => Ok(GeoPoint { long: 0.0, lat: 0.0 }),
            other => Err(Error::Serialization(format!(
                "unknown GeoPoint constructor {other:#x}"
            ))),
        }
    }
}

// ===========================================================================
