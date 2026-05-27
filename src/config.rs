use serde::Deserialize;
use std::path::Path;

use crate::error::DChunkedError;

#[derive(Debug, Deserialize)]
pub struct ProxyConfig {
    pub proxies: Vec<ProxyEntry>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProxyEntry {
    pub addr: String,
}

pub fn load_proxy_config(path: &Path) -> Result<ProxyConfig, DChunkedError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| DChunkedError::Config(format!("Failed to read {}: {e}", path.display())))?;
    let config: ProxyConfig = toml::from_str(&content)
        .map_err(|e| DChunkedError::Config(format!("Failed to parse TOML: {e}")))?;
    Ok(config)
}
