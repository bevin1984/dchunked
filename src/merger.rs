use std::path::Path;

use tokio::io::AsyncWriteExt;

use crate::error::DChunkedError;
use crate::worker::ChunkResult;

pub async fn merge_chunks(
    mut results: Vec<ChunkResult>,
    output_path: &Path,
    total_size: u64,
) -> Result<(), DChunkedError> {
    results.sort_by_key(|r| r.index);

    let mut file = tokio::fs::File::create(output_path).await?;

    let mut remaining = total_size;
    let mut buf = vec![0u8; 64 * 1024];
    for result in &results {
        let mut input = tokio::fs::File::open(&result.temp_path).await?;
        while remaining > 0 {
            let to_read = buf.len().min(remaining as usize);
            let n = tokio::io::AsyncReadExt::read(&mut input, &mut buf[..to_read]).await?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).await?;
            remaining -= n as u64;
        }
    }

    file.flush().await?;

    // Cleanup temp files
    for result in &results {
        let _ = tokio::fs::remove_file(&result.temp_path).await;
    }

    Ok(())
}
