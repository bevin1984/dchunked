use std::path::PathBuf;
use std::sync::Arc;

use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use crate::error::DChunkedError;
use crate::planner::ChunkRange;
use crate::progress::ChunkProgress;
use crate::proxy::ProxyPool;

pub struct ChunkResult {
    pub index: usize,
    pub temp_path: PathBuf,
}

pub async fn download_chunk(
    chunk: ChunkRange,
    url: &str,
    output_dir: &std::path::Path,
    output_file: &str,
    timeout: u64,
    max_retries: u32,
    pool: Arc<ProxyPool>,
    progress: Arc<ChunkProgress>,
) -> Result<ChunkResult, DChunkedError> {
    let temp_path = output_dir.join(format!(".{}.{}.part", output_file, chunk.index));
    let chunk_size = chunk.end - chunk.start + 1;
    let mut progress_reported: u64 = 0;

    // Check for already-complete chunk from previous run
    if let Ok(meta) = tokio::fs::metadata(&temp_path).await {
        if meta.len() >= chunk_size {
            eprintln!("  chunk {}: already complete, skipping", chunk.index);
            progress.inc(chunk_size);
            return Ok(ChunkResult {
                index: chunk.index,
                temp_path,
            });
        }
    }

    for attempt in 0..max_retries {
        // Recalculate resume offset from actual file size on each attempt
        let resume_offset = match tokio::fs::metadata(&temp_path).await {
            Ok(meta) => meta.len().min(chunk_size),
            Err(_) => 0,
        };

        // Report progress for already-downloaded but not-yet-reported bytes
        if resume_offset > progress_reported {
            eprintln!(
                "  chunk {}: resuming from {} bytes",
                chunk.index, resume_offset
            );
            progress.inc(resume_offset - progress_reported);
            progress_reported = resume_offset;
        }

        if resume_offset >= chunk_size {
            return Ok(ChunkResult {
                index: chunk.index,
                temp_path,
            });
        }

        let lease = pool.lease().await;

        let mut builder = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(timeout))
            .read_timeout(std::time::Duration::from_secs(timeout));

        if let Some(ref addr) = lease.proxy_addr {
            builder = builder.no_proxy();
            match reqwest::Proxy::all(addr) {
                Ok(proxy) => {
                    builder = builder.proxy(proxy);
                }
                Err(e) => {
                    eprintln!(
                        "  chunk {}: proxy parse error (attempt {}/{}): {}",
                        chunk.index,
                        attempt + 1,
                        max_retries,
                        e
                    );
                    if let Some(idx) = lease.proxy_index {
                        pool.report_failure(idx).await;
                    }
                    continue;
                }
            }
        }

        let client = match builder.build() {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "  chunk {}: client build error (attempt {}/{}): {}",
                    chunk.index,
                    attempt + 1,
                    max_retries,
                    e
                );
                continue;
            }
        };

        let range_start = chunk.start + resume_offset;
        let range_header = format!("bytes={}-{}", range_start, chunk.end);
        let resp = match client.get(url).header("Range", &range_header).send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "  chunk {}: request error (attempt {}/{}): {}",
                    chunk.index,
                    attempt + 1,
                    max_retries,
                    e
                );
                if let Some(idx) = lease.proxy_index {
                    pool.report_failure(idx).await;
                }
                continue;
            }
        };

        let status = resp.status();
        if status.as_u16() >= 400 {
            if let Some(idx) = lease.proxy_index {
                pool.report_failure(idx).await;
            }
            if status.as_u16() >= 400 && status.as_u16() < 500 {
                return Err(DChunkedError::Config(format!(
                    "HTTP {} for chunk {} (client error, not retrying)",
                    status, chunk.index
                )));
            }
            eprintln!(
                "  chunk {}: HTTP {} (attempt {}/{})",
                chunk.index,
                status,
                attempt + 1,
                max_retries
            );
            continue;
        }

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&temp_path)
            .await
            .map_err(DChunkedError::Io)?;

        let mut stream = resp.bytes_stream();
        let mut stream_error = None;
        while let Some(chunk_bytes) = stream.next().await {
            match chunk_bytes {
                Ok(data) => {
                    if let Err(e) = file.write_all(&data).await {
                        stream_error = Some(DChunkedError::Io(e));
                        break;
                    }
                    progress.inc(data.len() as u64);
                    progress_reported += data.len() as u64;
                }
                Err(e) => {
                    stream_error = Some(DChunkedError::Http(e));
                    break;
                }
            }
        }

        if let Some(ref err) = stream_error {
            eprintln!(
                "  chunk {}: stream error (attempt {}/{}): {}",
                chunk.index,
                attempt + 1,
                max_retries,
                err
            );
            if let Some(idx) = lease.proxy_index {
                pool.report_failure(idx).await;
            }
            continue;
        }

        file.flush().await?;

        if let Some(idx) = lease.proxy_index {
            pool.report_success(idx).await;
        }

        return Ok(ChunkResult {
            index: chunk.index,
            temp_path,
        });
    }

    Err(DChunkedError::RetryExhausted {
        chunk_id: chunk.index,
        retries: max_retries,
    })
}
