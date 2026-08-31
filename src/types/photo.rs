//! FileLocation — the synthesized download-location union. Schema-shaped
//! photo types (Photo, PhotoSize, Document, WebDocument, GeoPoint) live
//! in `photo_gen.rs`.


/// Download locations assembled from document/photo fields (client-side
/// synthesis — not a TL union on the wire).
#[derive(Debug, Clone)]
pub enum FileLocation {
    VolumeId { volume_id: i64, local_id: i32, secret: i64, reference: Vec<u8>, dc_id: i32 },
    /// `inputDocumentFileLocation#bad07584` — documents/files. Empty
    /// `thumb_size` downloads the full file; "m" fetches a thumbnail.
    Document { id: i64, access_hash: i64, reference: Vec<u8>, thumb_size: String, dc_id: i32 },
    Web { dc_id: i32, url: String, size: i32 },
    EmojiStickerSet { version: i32, set_id: i64 },
    Unknown,
}
