use crate::error::DChunkedError;

pub struct ChunkPlan {
    pub total_size: u64,
    pub supports_range: bool,
    pub chunks: Vec<ChunkRange>,
}

#[derive(Debug, Clone)]
pub struct ChunkRange {
    pub index: usize,
    pub start: u64,
    pub end: u64,
}

pub async fn plan(
    client: &reqwest::Client,
    url: &str,
    num_chunks: usize,
) -> Result<ChunkPlan, DChunkedError> {
    let resp = client.head(url).send().await?;

    let total_size = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or(DChunkedError::UnknownFileSize)?;

    let supports_range = resp
        .headers()
        .get("accept-ranges")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("bytes"))
        .unwrap_or(false);

    if !supports_range || num_chunks <= 1 {
        return Ok(ChunkPlan {
            total_size,
            supports_range: false,
            chunks: vec![ChunkRange {
                index: 0,
                start: 0,
                end: total_size.saturating_sub(1),
            }],
        });
    }

    let actual_chunks = num_chunks.min(total_size as usize);
    let chunk_size = total_size / actual_chunks as u64;

    let mut chunks = Vec::with_capacity(actual_chunks);
    for i in 0..actual_chunks {
        let start = i as u64 * chunk_size;
        let end = if i == actual_chunks - 1 {
            total_size.saturating_sub(1)
        } else {
            (i as u64 + 1) * chunk_size - 1
        };
        chunks.push(ChunkRange {
            index: i,
            start,
            end,
        });
    }

    Ok(ChunkPlan {
        total_size,
        supports_range: true,
        chunks,
    })
}
