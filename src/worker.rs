use std::path::PathBuf;
use std::sync::Arc;

use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use crate::block::BlockRange;
use crate::error::DChunkedError;
use crate::progress::WorkerProgress;
use crate::proxy::ProxyPool;

pub struct BlockResult {
    pub index: usize,
    #[allow(dead_code)]
    pub path: PathBuf,
}

pub async fn download_block(
    block: BlockRange,
    url: &str,
    block_dir: &std::path::Path,
    timeout: u64,
    max_retries: u32,
    pool: Arc<ProxyPool>,
    progress: Arc<WorkerProgress>,
) -> Result<BlockResult, DChunkedError> {
    let block_path = block_dir.join(format!("{}.block", block.index));
    let block_size = block.expected_size;
    let mut progress_reported: u64 = 0;

    // Check for already-complete block from previous run
    if let Ok(meta) = tokio::fs::metadata(&block_path).await {
        if meta.len() >= block_size {
            eprintln!("  block {}: already complete, skipping", block.index);
            progress.inc(block_size);
            return Ok(BlockResult {
                index: block.index,
                path: block_path,
            });
        }
    }

    for attempt in 0..max_retries {
        // Recalculate resume offset from actual file size on each attempt
        let resume_offset = match tokio::fs::metadata(&block_path).await {
            Ok(meta) => meta.len().min(block_size),
            Err(_) => 0,
        };

        // Report progress for already-downloaded but not-yet-reported bytes
        if resume_offset > progress_reported {
            eprintln!(
                "  block {}: resuming from {} bytes",
                block.index, resume_offset
            );
            progress.inc(resume_offset - progress_reported);
            progress_reported = resume_offset;
        }

        if resume_offset >= block_size {
            return Ok(BlockResult {
                index: block.index,
                path: block_path,
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
                        "  block {}: proxy parse error (attempt {}/{}): {}",
                        block.index,
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
                    "  block {}: client build error (attempt {}/{}): {}",
                    block.index,
                    attempt + 1,
                    max_retries,
                    e
                );
                continue;
            }
        };

        let range_start = block.start + resume_offset;
        let range_header = format!("bytes={}-{}", range_start, block.end);
        let resp = match client.get(url).header("Range", &range_header).send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "  block {}: request error (attempt {}/{}): {}",
                    block.index,
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
                    "HTTP {} for block {} (client error, not retrying)",
                    status, block.index
                )));
            }
            eprintln!(
                "  block {}: HTTP {} (attempt {}/{})",
                block.index,
                status,
                attempt + 1,
                max_retries
            );
            continue;
        }

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&block_path)
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
                "  block {}: stream error (attempt {}/{}): {}",
                block.index,
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

        return Ok(BlockResult {
            index: block.index,
            path: block_path,
        });
    }

    Err(DChunkedError::RetryExhausted {
        chunk_id: block.index,
        retries: max_retries,
    })
}
