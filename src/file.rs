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
            md5_checksum: None,
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

/// Download the whole file into memory (convenience wrapper).
pub async fn download(
    pool: Arc<crate::pool::SenderPool>,
    location: &FileLocation,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut offset: i32 = 0;
    loop {
        let payload = rpc::build_get_file(location, offset, DOWNLOAD_CHUNK as i32);
        let chunk = pool.send_rpc(&payload).await?;
        let n = chunk.len();
        out.extend_from_slice(&chunk);
        if (n as usize) < DOWNLOAD_CHUNK {
            break;
        }
        offset = offset.saturating_add(n as i32);
    }
    Ok(out)
}

/// Collect chunks with a callback instead of a stream.
pub async fn for_each_chunk(
    pool: Arc<crate::pool::SenderPool>,
    location: &FileLocation,
    mut f: impl FnMut(usize, &[u8]) -> Result<()>,
) -> Result<()> {
    let mut offset: i32 = 0;
    loop {
        let payload = rpc::build_get_file(location, offset, DOWNLOAD_CHUNK as i32);
        let chunk = pool.send_rpc(&payload).await?;
        let n = chunk.len();
        f(offset as usize, &chunk)?;
        if n < DOWNLOAD_CHUNK {
            break;
        }
        offset = offset.saturating_add(n as i32);
    }
    Ok(())
}

