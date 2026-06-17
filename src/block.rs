use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

use crate::error::DChunkedError;

#[derive(Debug, Clone)]
pub struct BlockRange {
    pub index: usize,
    pub start: u64,
    pub end: u64,
    pub expected_size: u64,
}

pub struct BlockPlan {
    pub total_size: u64,
    pub supports_range: bool,
    pub blocks: Vec<BlockRange>,
    pub block_size: u64,
}

#[derive(Serialize, Deserialize)]
pub struct Manifest {
    pub url: String,
    pub total_size: u64,
    pub block_size: u64,
    pub completed: Vec<bool>,
}

pub struct BlockAssignment {
    pub block: BlockRange,
    pub block_dir: PathBuf,
}

pub const MAX_FAILURES_PER_BLOCK: usize = 5;

pub struct BlockScheduler {
    blocks: Vec<BlockRange>,
    completed: Vec<AtomicBool>,
    in_progress: Vec<AtomicBool>,
    failure_counts: Vec<AtomicUsize>,
    next_cursor: AtomicUsize,
    block_dir: PathBuf,
    manifest_path: PathBuf,
    url: String,
    total_size: u64,
    block_size: u64,
}

impl BlockScheduler {
    pub fn new(
        blocks: Vec<BlockRange>,
        block_dir: PathBuf,
        manifest_path: PathBuf,
        existing_completed: &[bool],
        url: String,
        total_size: u64,
        block_size: u64,
    ) -> Self {
        let completed: Vec<AtomicBool> = blocks
            .iter()
            .enumerate()
            .map(|(i, _)| AtomicBool::new(existing_completed.get(i).copied().unwrap_or(false)))
            .collect();
        let in_progress: Vec<AtomicBool> = blocks.iter().map(|_| AtomicBool::new(false)).collect();
        let failure_counts: Vec<AtomicUsize> =
            blocks.iter().map(|_| AtomicUsize::new(0)).collect();

        Self {
            blocks,
            completed,
            in_progress,
            failure_counts,
            next_cursor: AtomicUsize::new(0),
            block_dir,
            manifest_path,
            url,
            total_size,
            block_size,
        }
    }

    pub fn acquire_next(&self) -> Option<BlockAssignment> {
        let len = self.blocks.len();
        if len == 0 {
            return None;
        }
        let start = self.next_cursor.load(Ordering::Relaxed);

        for offset in 0..len {
            let i = (start + offset) % len;

            if self.completed[i].load(Ordering::Acquire) {
                continue;
            }

            if self.failure_counts[i].load(Ordering::Acquire) >= MAX_FAILURES_PER_BLOCK {
                continue;
            }

            if self
                .in_progress[i]
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                self.next_cursor.store((i + 1) % len, Ordering::Relaxed);
                return Some(BlockAssignment {
                    block: self.blocks[i].clone(),
                    block_dir: self.block_dir.clone(),
                });
            }
        }
        None
    }

    pub fn mark_complete(&self, index: usize) {
        self.completed[index].store(true, Ordering::Release);
        self.in_progress[index].store(false, Ordering::Release);
        self.persist_manifest();
    }

    pub fn release(&self, index: usize) {
        self.in_progress[index].store(false, Ordering::Release);
    }

    pub fn record_failure(&self, index: usize) -> usize {
        self.failure_counts[index].fetch_add(1, Ordering::AcqRel) + 1
    }

    pub fn failed_blocks(&self) -> Vec<usize> {
        self.failure_counts
            .iter()
            .enumerate()
            .filter(|(_, c)| c.load(Ordering::Acquire) >= MAX_FAILURES_PER_BLOCK)
            .map(|(i, _)| i)
            .collect()
    }

    fn persist_manifest(&self) {
        let completed: Vec<bool> = self
            .completed
            .iter()
            .map(|b| b.load(Ordering::Acquire))
            .collect();
        let manifest = Manifest {
            url: self.url.clone(),
            total_size: self.total_size,
            block_size: self.block_size,
            completed,
        };
        // Best-effort write; don't fail the download if manifest write fails
        if let Ok(data) = toml::to_string_pretty(&manifest) {
            let tmp_path = self.manifest_path.with_extension("toml.tmp");
            if std::fs::write(&tmp_path, &data).is_ok() {
                let _ = std::fs::rename(&tmp_path, &self.manifest_path);
            }
        }
    }
}

pub async fn plan_blocks(
    client: &reqwest::Client,
    url: &str,
    block_size: u64,
) -> Result<BlockPlan, DChunkedError> {
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

    if !supports_range {
        return Ok(BlockPlan {
            total_size,
            supports_range: false,
            blocks: vec![BlockRange {
                index: 0,
                start: 0,
                end: total_size.saturating_sub(1),
                expected_size: total_size,
            }],
            block_size,
        });
    }

    let num_blocks = ((total_size + block_size - 1) / block_size) as usize;
    let blocks: Vec<BlockRange> = (0..num_blocks)
        .map(|i| {
            let start = i as u64 * block_size;
            let end = std::cmp::min(start + block_size, total_size) - 1;
            BlockRange {
                index: i,
                start,
                end,
                expected_size: end - start + 1,
            }
        })
        .collect();

    Ok(BlockPlan {
        total_size,
        supports_range: true,
        blocks,
        block_size,
    })
}

pub fn init_block_dir(
    block_dir: &Path,
    plan: &BlockPlan,
    url: &str,
    existing_completed: &[bool],
) -> Result<(), DChunkedError> {
    std::fs::create_dir_all(block_dir).map_err(DChunkedError::Io)?;

    let manifest = Manifest {
        url: url.to_string(),
        total_size: plan.total_size,
        block_size: plan.block_size,
        completed: existing_completed.to_vec(),
    };
    let data = toml::to_string_pretty(&manifest)
        .map_err(|e| DChunkedError::Config(format!("Failed to serialize manifest: {e}")))?;
    let manifest_path = block_dir.join("manifest.toml");
    std::fs::write(&manifest_path, data).map_err(DChunkedError::Io)?;

    Ok(())
}

pub fn scan_existing_blocks(block_dir: &Path, plan: &BlockPlan) -> Vec<bool> {
    let manifest_path = block_dir.join("manifest.toml");

    // Try to load existing manifest
    let manifest = match std::fs::read_to_string(&manifest_path) {
        Ok(data) => toml::from_str::<Manifest>(&data).ok(),
        Err(_) => None,
    };

    let mut completed = vec![false; plan.blocks.len()];

    // Validate manifest matches current plan
    if let Some(ref m) = manifest {
        if m.url.is_empty()
            || m.total_size != plan.total_size
            || m.block_size != plan.block_size
            || m.completed.len() != plan.blocks.len()
        {
            eprintln!("Manifest mismatch, starting fresh download");
            return completed;
        }

        for (i, &done) in m.completed.iter().enumerate() {
            if done {
                let block_path = block_dir.join(format!("{}.block", i));
                if let Ok(meta) = std::fs::metadata(&block_path) {
                    if meta.len() == plan.blocks[i].expected_size {
                        completed[i] = true;
                    }
                }
            }
        }
    }

    completed
}
