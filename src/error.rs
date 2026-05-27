#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum DChunkedError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Server does not support range requests")]
    NoRangeSupport,

    #[error("All proxies failed for chunk {chunk_id}")]
    AllProxiesFailed { chunk_id: usize },

    #[error("Download failed after {retries} retries for chunk {chunk_id}")]
    RetryExhausted { chunk_id: usize, retries: u32 },

    #[error("File size could not be determined")]
    UnknownFileSize,

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
}
