use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "dchunked")]
#[command(about = "Chunked HTTP downloader with SOCKS5 proxy pool support")]
#[command(version)]
pub struct Args {
    /// URL to download
    pub url: String,

    /// Output file path
    #[arg(short, long)]
    pub output: Option<String>,

    /// Number of chunks (segments)
    #[arg(short, long, default_value = "8")]
    pub chunks: usize,

    /// Single SOCKS5 proxy address (e.g. socks5h://127.0.0.1:1080)
    #[arg(short = 'x', long)]
    pub proxy: Option<String>,

    /// Path to proxy pool config file (TOML)
    #[arg(short, long)]
    pub proxy_file: Option<String>,

    /// Max retries per chunk
    #[arg(long, default_value = "3")]
    pub retry: u32,

    /// Connection timeout in seconds
    #[arg(long, default_value = "30")]
    pub timeout: u64,
}

pub fn extract_filename_from_url(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("download")
        .to_string()
}
