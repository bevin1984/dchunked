use std::path::Path;

use tokio::io::AsyncWriteExt;

use crate::block::BlockRange;
use crate::error::DChunkedError;

pub async fn merge_blocks(
    block_dir: &Path,
    blocks: &[BlockRange],
    output_path: &Path,
    total_size: u64,
) -> Result<(), DChunkedError> {
    let mut file = tokio::fs::File::create(output_path).await?;

    let mut buf = vec![0u8; 64 * 1024];
    let mut written = 0u64;
    for (i, block) in blocks.iter().enumerate() {
        let block_path = block_dir.join(format!("{}.block", i));
        let mut input = tokio::fs::File::open(&block_path).await?;
        let mut block_written = 0u64;
        while block_written < block.expected_size {
            let to_read = buf
                .len()
                .min((block.expected_size - block_written) as usize);
            let n = tokio::io::AsyncReadExt::read(&mut input, &mut buf[..to_read])
                .await
                .map_err(DChunkedError::Io)?;
            if n == 0 {
                return Err(DChunkedError::Config(format!(
                    "block {}: unexpected EOF (read {}/{})",
                    i, block_written, block.expected_size
                )));
            }
            file.write_all(&buf[..n]).await?;
            block_written += n as u64;
            written += n as u64;
        }
    }

    file.flush().await?;
    if written != total_size {
        return Err(DChunkedError::Config(format!(
            "merged size mismatch: written {} != total {}",
            written, total_size
        )));
    }
    Ok(())
}
