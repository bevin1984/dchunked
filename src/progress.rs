use std::sync::Arc;
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

pub struct WorkerProgress {
    pb: ProgressBar,
    overall: ProgressBar,
}

impl WorkerProgress {
    pub fn inc(&self, bytes: u64) {
        self.pb.inc(bytes);
        self.overall.inc(bytes);
    }

    pub fn set_block(&self, block_index: usize, block_size: u64) {
        self.pb.reset();
        self.pb.set_length(block_size);
        self.pb.set_message(format!("block {} ", block_index));
    }
}

pub struct DownloadProgress {
    multi: Arc<MultiProgress>,
    overall: ProgressBar,
    workers: Vec<Arc<WorkerProgress>>,
}

impl DownloadProgress {
    pub fn new(total_size: u64, num_workers: usize) -> Self {
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

        let workers: Vec<Arc<WorkerProgress>> = (0..num_workers)
            .map(|id| {
                let pb = multi.add(ProgressBar::new(0));
                pb.set_style(
                    ProgressStyle::with_template(&format!(
                        "  worker {id}: {{msg}}[{{bar:30.cyan/blue}}] {{bytes}}/{{total_bytes}}"
                    ))
                    .unwrap()
                    .progress_chars("#>-"),
                );
                Arc::new(WorkerProgress {
                    pb,
                    overall: overall.clone(),
                })
            })
            .collect();

        Self {
            multi,
            overall,
            workers,
        }
    }

    pub fn worker_progress(&self, worker_id: usize, block_index: usize, block_size: u64) -> Arc<WorkerProgress> {
        let wp = self.workers[worker_id].clone();
        wp.set_block(block_index, block_size);
        wp
    }

    pub fn reset_overall_eta(&self) {
        self.overall.reset_eta();
    }

    pub fn finish(&self) {
        // Finish all worker bars first, then the overall bar
        for wp in &self.workers {
            wp.pb.finish_and_clear();
        }
        self.overall.finish_and_clear();
        // Ensure the MultiProgress stops drawing
        self.multi.clear().unwrap_or(());
    }
}
