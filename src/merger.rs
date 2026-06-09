use std::path::Path;

use tokio::io::AsyncWriteExt;

use crate::error::DChunkedError;

pub async fn merge_blocks(
    block_dir: &Path,
    num_blocks: usize,
    output_path: &Path,
    total_size: u64,
) -> Result<(), DChunkedError> {
    let mut file = tokio::fs::File::create(output_path).await?;

    let mut remaining = total_size;
    let mut buf = vec![0u8; 64 * 1024];
    for i in 0..num_blocks {
        let block_path = block_dir.join(format!("{}.block", i));
        let mut input = tokio::fs::File::open(&block_path).await?;
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
    Ok(())
}
