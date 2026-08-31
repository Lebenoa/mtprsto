//! File transfer: chunked upload and download (SPEC §7, gap items 3+4).
//!
//! Upload: `upload.saveFilePart` for files ≤ 10 MiB, `upload.saveBigFilePart`
//! for larger ones (512 KiB parts, ≤ 4000 parts per doc, split here across
//! parallel workers per SPEC §11.3 PartPlan).
//!
//! Download: `upload.getFile` in 1 MiB chunks (server max), iterating offsets
//! until a short read ends the stream.

use crate::error::{Error, Result};
use crate::rpc;
use crate::types::{FileLocation, InputFile};
use std::io::Read;
use std::sync::Arc;
use std::path::Path;

/// `upload.saveFilePart` part size (512 KiB is the only allowed value).
pub const SMALL_PART_SIZE: usize = 512 * 1024;
/// `upload.saveBigFilePart` part size (512 KiB).
pub const BIG_PART_SIZE: usize = 512 * 1024;
/// Files at or below this size use `saveFilePart`; above use `saveBigFilePart`.
pub const BIG_FILE_THRESHOLD: u64 = 10 * 1024 * 1024;
/// Server cap on getFile responses; request this much per chunk.
pub const DOWNLOAD_CHUNK: usize = 1024 * 1024;

// ---------------------------------------------------------------------------
// Upload
// ---------------------------------------------------------------------------

/// Upload reader contents as a single-file `InputFile`, chunking with
/// `saveFilePart`/`saveBigFilePart` and uploading parts round-robin through
/// the pool with `worker_count` concurrent tasks.
pub async fn upload<R: Read + Send>(
    pool: Arc<crate::pool::SenderPool>,
    name: String,
    reader: &mut R,
    size: u64,
    worker_count: usize,
) -> Result<InputFile> {
    if size == 0 {
        return Err(Error::Other("cannot upload an empty file".into()));
    }

    let big = size > BIG_FILE_THRESHOLD;
    let part_size = if big { BIG_PART_SIZE } else { SMALL_PART_SIZE };
    let total_parts = size.div_ceil(part_size as u64) as usize;
    if big && total_parts > 4000 {
        return Err(Error::Other(format!(
            "file too large: {size} bytes needs {total_parts} parts (max 4000)"
        )));
    }
    let file_id: i64 = rand::random();
    let parts: Vec<std::sync::Arc<[u8]>> = {
        let mut raw: Vec<Vec<u8>> = Vec::with_capacity(total_parts);
        for _ in 0..total_parts {
            let mut buf = vec![0u8; part_size];
            let mut filled = 0usize;
            while filled < part_size {
                let n = reader
                    .read(&mut buf[filled..])
                    .map_err(|e| Error::Other(format!("read failed: {e}")))?;
                if n == 0 {
                    break;
                }
                filled += n;
            }
            buf.truncate(filled);
            raw.push(buf);
        }
        raw.into_iter().map(std::sync::Arc::from).collect()
    };

    let parts = std::sync::Arc::new(parts);
    let workers = worker_count.clamp(1, total_parts);
    let mut handles = Vec::with_capacity(workers);
    for worker in 0..workers {
        let parts = std::sync::Arc::clone(&parts);
        let pool = std::sync::Arc::clone(&pool);
        handles.push(tokio::spawn(async move {
            for part_index in (worker..total_parts).step_by(workers) {
                let data = &parts[part_index];
                let payload = if big {
                    rpc::build_save_big_file_part(
                        file_id,
                        part_index as i32,
                        total_parts as i32,
                        data,
                    )
                } else {
                    rpc::build_save_file_part(file_id, part_index as i32, data)
                };
                pool.send_rpc(&payload).await.map_err(|e| {
                    Error::Other(format!("part {part_index} upload failed: {e}"))
                })?;
            }
            Ok::<(), Error>(())
        }));
    }
    for handle in handles {
        handle
            .await
            .map_err(|e| Error::Other(format!("upload worker panicked: {e}")))??;
    }

    Ok(if big {
        InputFile::Big {
            id: file_id,
            parts: total_parts as i32,
            name: name.clone(),
        }
    } else {
        InputFile::Id {
            id: file_id,
            parts: total_parts as i32,
            name,
            md5_checksum: String::new(),
        }
    })
}

/// Convenience: upload a file from disk.
pub async fn upload_file(
    pool: Arc<crate::pool::SenderPool>,
    path: &Path,
    worker_count: usize,
) -> Result<InputFile> {
    let size = std::fs::metadata(path)
        .map_err(|e| Error::Other(format!("stat {}: {e}", path.display())))?
        .len();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file.bin")
        .to_string();
    let mut file = std::fs::File::open(path)
        .map_err(|e| Error::Other(format!("open {}: {e}", path.display())))?;
    upload(pool, name, &mut file, size, worker_count).await
}

// ---------------------------------------------------------------------------
// Download
// ---------------------------------------------------------------------------

/// BS-5 download knobs (SPEC §12.2). Files at or above
/// [`DownloadConfig::parallel_threshold`] are fetched as
/// [`DownloadConfig::parallel_count`] contiguous concurrent ranges.
#[derive(Debug, Clone)]
pub struct DownloadConfig {
    /// Fetch files larger than this in parallel chunks. Default 8 MiB.
    pub parallel_threshold: u64,
    /// Number of concurrent range downloads. 1 = serial. Default 4.
    pub parallel_count: usize,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            parallel_threshold: 8 * 1024 * 1024,
            parallel_count: 4,
        }
    }
}

/// A parsed `upload.getFile` reply (SPEC §7 upload surface).
#[derive(Debug, Clone)]
pub enum GetFile {
    /// `upload.file#96a18f23` — the actual bytes.
    File { mtime: i32, bytes: Vec<u8> },
    /// `upload.fileCdnRedirect#f18cda2c` — content lives on a CDN DC and
    /// must be fetched with `upload.getCdnFile` + AES-CTR decryption.
    CdnRedirect {
        dc_id: i32,
        file_token: Vec<u8>,
        encryption_key: Vec<u8>,
        encryption_iv: Vec<u8>,
    },
}

/// Parse an `upload.getFile` response: `upload.file` or `fileCdnRedirect`.
pub fn parse_get_file(data: &[u8]) -> Result<GetFile> {
    use crate::serialize::{TLReader, UPLOAD_FILE, UPLOAD_FILE_CDN_REDIRECT};
    let mut r = TLReader::new(data);
    let ctor = r.read_u32()?;
    match ctor {
        UPLOAD_FILE => {
            // type:storage.fileType (bare ctor, 4 bytes), mtime:int, bytes
            let _type_ctor = r.read_u32()?;
            let mtime = r.read_i32()?;
            Ok(GetFile::File {
                mtime,
                bytes: r.read_bytes()?,
            })
        }
        UPLOAD_FILE_CDN_REDIRECT => Ok(GetFile::CdnRedirect {
            dc_id: r.read_i32()?,
            file_token: r.read_bytes()?,
            encryption_key: r.read_bytes()?,
            encryption_iv: r.read_bytes()?,
        }),
        other => Err(Error::UnexpectedResponse(format!(
            "expected upload.file or fileCdnRedirect, got {other:#x}"
        ))),
    }
}

/// Fetch one `getFile` chunk and unwrap the `upload.file` envelope.
async fn get_file_chunk(
    pool: &crate::pool::SenderPool,
    location: &FileLocation,
    offset: i64,
    limit: usize,
) -> Result<Vec<u8>> {
    let payload = rpc::build_get_file(location, offset, limit as i32);
    let raw = pool.send_rpc(&payload).await?;
    match parse_get_file(&raw)? {
        GetFile::File { bytes, .. } => Ok(bytes),
        GetFile::CdnRedirect { dc_id, .. } => Err(Error::Other(format!(
            "file is served from CDN DC {dc_id}; CDN download is not supported yet"
        ))),
    }
}

/// Download the whole file into memory (convenience wrapper). Serial.
pub async fn download(
    pool: Arc<crate::pool::SenderPool>,
    location: &FileLocation,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut offset: i64 = 0;
    loop {
        let chunk = get_file_chunk(&pool, location, offset, DOWNLOAD_CHUNK).await?;
        let n = chunk.len();
        out.extend_from_slice(&chunk);
        if n < DOWNLOAD_CHUNK {
            break;
        }
        offset = offset.saturating_add(n as i64);
    }
    Ok(out)
}

/// Download a file of known `size` in parallel contiguous ranges when the
/// size meets the threshold (SPEC BS-5), otherwise serial. Reassembled
/// in-order; the first chunk error aborts.
pub async fn download_parallel(
    pool: Arc<crate::pool::SenderPool>,
    location: &FileLocation,
    size: u64,
    config: &DownloadConfig,
) -> Result<Vec<u8>> {
    if size == 0 {
        return Ok(Vec::new());
    }
    if config.parallel_count <= 1 || size < config.parallel_threshold {
        return download(pool, location).await;
    }

    // Contiguous ranges; the last one ends at EOF so a short final chunk
    // needs no special handling.
    let workers = config
        .parallel_count
        .min(size.div_ceil(DOWNLOAD_CHUNK as u64) as usize)
        .max(1);
    let range = size.div_ceil(workers as u64);
    let location = std::sync::Arc::new(clone_location(location));
    let mut handles = Vec::with_capacity(workers);
    for w in 0..workers {
        let start = (w as u64) * range;
        let end = ((w as u64 + 1) * range).min(size);
        if start >= end {
            break;
        }
        let pool = std::sync::Arc::clone(&pool);
        let location = std::sync::Arc::clone(&location);
        handles.push(tokio::spawn(async move {
            let mut buf = Vec::with_capacity((end - start) as usize);
            let mut offset = start;
            while offset < end {
                let chunk =
                    get_file_chunk(&pool, &location, offset as i64, DOWNLOAD_CHUNK).await?;
                if chunk.is_empty() {
                    return Err(Error::Other(format!(
                        "download ended early at offset {offset} (expected end {end})"
                    )));
                }
                buf.extend_from_slice(&chunk);
                offset += chunk.len() as u64;
            }
            Ok::<(u64, Vec<u8>), Error>((start, buf))
        }));
    }
    let mut parts: Vec<(u64, Vec<u8>)> = Vec::with_capacity(handles.len());
    for handle in handles {
        parts.push(
            handle
                .await
                .map_err(|e| Error::Other(format!("download worker panicked: {e}")))??,
        );
    }
    parts.sort_by_key(|(start, _)| *start);
    Ok(parts.into_iter().flat_map(|(_, buf)| buf).collect())
}

/// Collect chunks with a callback instead of a stream. Serial.
pub async fn for_each_chunk(
    pool: Arc<crate::pool::SenderPool>,
    location: &FileLocation,
    mut f: impl FnMut(usize, &[u8]) -> Result<()>,
) -> Result<()> {
    let mut offset: i64 = 0;
    loop {
        let chunk = get_file_chunk(&pool, location, offset, DOWNLOAD_CHUNK).await?;
        let n = chunk.len();
        f(offset as usize, &chunk)?;
        if n < DOWNLOAD_CHUNK {
            break;
        }
        offset = offset.saturating_add(n as i64);
    }
    Ok(())
}

/// Clone a `FileLocation` for handing to spawned download tasks.
fn clone_location(loc: &FileLocation) -> FileLocation {
    loc.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialize::TLWriter;

    #[test]
    fn test_parse_get_file_upload_file() {
        let mut w = TLWriter::new();
        w.write_u32(crate::serialize::UPLOAD_FILE);
        w.write_u32(0xaa963b0d); // storage.fileUnknown (bare ctor)
        w.write_i32(1700000000); // mtime
        w.write_bytes(b"payload-bytes");
        match parse_get_file(w.as_bytes()).unwrap() {
            GetFile::File { mtime, bytes } => {
                assert_eq!(mtime, 1700000000);
                assert_eq!(bytes, b"payload-bytes");
            }
            other => panic!("expected File, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_get_file_cdn_redirect() {
        let mut w = TLWriter::new();
        w.write_u32(crate::serialize::UPLOAD_FILE_CDN_REDIRECT);
        w.write_i32(2);
        w.write_bytes(b"token");
        w.write_bytes(&[0x11u8; 32]); // encryption_key
        w.write_bytes(&[0x22u8; 16]); // encryption_iv
        match parse_get_file(w.as_bytes()).unwrap() {
            GetFile::CdnRedirect {
                dc_id,
                file_token,
                encryption_key,
                encryption_iv,
            } => {
                assert_eq!(dc_id, 2);
                assert_eq!(file_token, b"token");
                assert_eq!(encryption_key, &[0x11u8; 32]);
                assert_eq!(encryption_iv, &[0x22u8; 16]);
            }
            other => panic!("expected CdnRedirect, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_get_file_rejects_unknown_ctor() {
        let mut w = TLWriter::new();
        w.write_u32(0xdeadbeef);
        assert!(parse_get_file(w.as_bytes()).is_err());
    }

    #[test]
    fn test_download_config_defaults() {
        let cfg = DownloadConfig::default();
        assert_eq!(cfg.parallel_threshold, 8 * 1024 * 1024);
        assert_eq!(cfg.parallel_count, 4);
    }
}

