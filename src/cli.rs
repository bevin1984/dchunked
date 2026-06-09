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

    /// Number of concurrent workers
    #[arg(short, long, default_value = "8")]
    pub chunks: usize,

    /// Block size (e.g. 10M, 1G)
    #[arg(short = 'b', long, default_value = "10M", value_parser = parse_block_size)]
    pub block_size: u64,

    /// Single SOCKS5 proxy address (e.g. socks5h://127.0.0.1:1080)
    #[arg(short = 'x', long)]
    pub proxy: Option<String>,

    /// Path to proxy pool config file (TOML)
    #[arg(short, long)]
    pub proxy_file: Option<String>,

    /// Max retries per block
    #[arg(long, default_value = "3")]
    pub retry: u32,

    /// Connection timeout in seconds
    #[arg(long, default_value = "30")]
    pub timeout: u64,
}

fn parse_block_size(input: &str) -> Result<u64, String> {
    let s = input.trim().to_uppercase();
    let (num_part, multiplier) = if s.ends_with("GB") {
        (&s[..s.len() - 2], 1024u64.pow(3))
    } else if s.ends_with("MB") {
        (&s[..s.len() - 2], 1024u64.pow(2))
    } else if s.ends_with("KB") {
        (&s[..s.len() - 2], 1024u64)
    } else if s.ends_with('G') {
        (&s[..s.len() - 1], 1024u64.pow(3))
    } else if s.ends_with('M') {
        (&s[..s.len() - 1], 1024u64.pow(2))
    } else if s.ends_with('K') {
        (&s[..s.len() - 1], 1024u64)
    } else {
        (s.as_str(), 1u64)
    };
    let n: u64 = num_part
        .trim()
        .parse()
        .map_err(|_| format!("Invalid block size: {}", input))?;
    if n == 0 {
        return Err("Block size must be > 0".into());
    }
    Ok(n * multiplier)
}

pub fn extract_filename_from_url(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("download")
        .to_string()
}
