use std::sync::Arc;
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

const MAX_CHUNK_BARS: usize = 16;

pub struct ChunkProgress {
    pb: ProgressBar,
    overall: ProgressBar,
}

impl ChunkProgress {
    pub fn inc(&self, bytes: u64) {
        self.pb.inc(bytes);
        self.overall.inc(bytes);
    }
}

pub struct DownloadProgress {
    _multi: Arc<MultiProgress>,
    overall: ProgressBar,
    chunks: Vec<Arc<ChunkProgress>>,
}

impl DownloadProgress {
    pub fn new(total_size: u64, num_chunks: usize) -> Self {
        let multi = Arc::new(MultiProgress::new());

        let overall = multi.add(ProgressBar::new(total_size));
        overall.set_style(
            ProgressStyle::with_template(
                "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, ETA: {eta})",
            )
            .unwrap()
            .progress_chars("#>-"),
        );
        overall.enable_steady_tick(Duration::from_millis(500));

        let chunk_size = total_size / num_chunks.max(1) as u64;
        let show_chunks = num_chunks <= MAX_CHUNK_BARS;

        let chunks: Vec<Arc<ChunkProgress>> = if show_chunks {
            (0..num_chunks)
                .map(|i| {
                    let pb = multi.add(ProgressBar::new(chunk_size));
                    pb.set_style(
                        ProgressStyle::with_template(&format!(
                            "  chunk {i}: [{{bar:30.cyan/blue}}] {{bytes}}/{{total_bytes}}"
                        ))
                        .unwrap()
                        .progress_chars("#>-"),
                    );
                    Arc::new(ChunkProgress {
                        pb,
                        overall: overall.clone(),
                    })
                })
                .collect()
        } else {
            (0..num_chunks)
                .map(|_| {
                    Arc::new(ChunkProgress {
                        pb: ProgressBar::hidden(),
                        overall: overall.clone(),
                    })
                })
                .collect()
        };

        Self {
            _multi: multi,
            overall,
            chunks,
        }
    }

    pub fn chunk_progress(&self, index: usize) -> Arc<ChunkProgress> {
        self.chunks[index].clone()
    }

    pub fn finish(&self) {
        self.overall.finish_and_clear();
    }
}
