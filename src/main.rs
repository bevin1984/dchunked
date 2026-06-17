mod block;
mod cli;
mod config;
mod error;
mod merger;
mod progress;
mod proxy;
mod worker;

use std::path::Path;
use std::sync::Arc;

use clap::Parser;

fn main() -> anyhow::Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
    let args = cli::Args::parse();

    // Collect proxy addresses from CLI and/or config file
    let mut proxy_addrs: Vec<String> = Vec::new();

    if let Some(ref proxy) = args.proxy {
        proxy_addrs.push(proxy.clone());
    }

    if let Some(ref path) = args.proxy_file {
        let config = config::load_proxy_config(Path::new(path))?;
        for entry in config.proxies {
            proxy_addrs.push(entry.addr);
        }
    }

    let first_proxy = proxy_addrs
        .first()
        .map(|s| reqwest::Proxy::all(s))
        .transpose()?;
    let pool = proxy::ProxyPool::new(proxy_addrs);
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| cli::extract_filename_from_url(&args.url));

    // Build client for HEAD request (use proxy if configured)
    let mut head_builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(args.timeout))
        .read_timeout(std::time::Duration::from_secs(args.timeout));

    if let Some(proxy) = first_proxy {
        head_builder = head_builder.proxy(proxy);
    }

    let head_client = head_builder.build()?;

    // Plan blocks
    let block_plan = block::plan_blocks(&head_client, &args.url, args.block_size).await?;

    if block_plan.supports_range {
        eprintln!(
            "File size: {} bytes, {} blocks ({} bytes/block), {} workers",
            block_plan.total_size,
            block_plan.blocks.len(),
            args.block_size,
            args.chunks,
        );
    } else {
        eprintln!(
            "File size: {} bytes, downloading (no range support)",
            block_plan.total_size,
        );
    }

    // Determine output path and hidden directory
    let output_path = Path::new(&output);
    let output_dir = output_path
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let output_filename = output_path
        .file_name()
        .unwrap_or(std::ffi::OsStr::new("download"))
        .to_string_lossy()
        .to_string();
    let block_dir = output_dir.join(format!(".{}", output_filename));
    let manifest_path = block_dir.join("manifest.toml");

    // Resume: scan existing blocks
    let existing_completed = block::scan_existing_blocks(&block_dir, &block_plan);

    let already_done = existing_completed.iter().filter(|&&b| b).count();
    if already_done > 0 {
        eprintln!(
            "Resuming: {}/{} blocks already complete",
            already_done,
            block_plan.blocks.len()
        );
    }

    // Create block dir and write manifest
    block::init_block_dir(&block_dir, &block_plan, &args.url, &existing_completed)?;

    // Create scheduler
    let scheduler = Arc::new(block::BlockScheduler::new(
        block_plan.blocks.clone(),
        block_dir.clone(),
        manifest_path,
        &existing_completed,
        args.url.clone(),
        block_plan.total_size,
        args.block_size,
    ));

    // Setup progress
    let num_workers = args.chunks.min(block_plan.blocks.len());
    let prog = Arc::new(progress::DownloadProgress::new(
        block_plan.total_size,
        num_workers,
    ));

    // Count already-downloaded bytes toward progress
    let done_bytes: u64 = existing_completed
        .iter()
        .enumerate()
        .filter(|(_, &done)| done)
        .map(|(i, _)| block_plan.blocks[i].expected_size)
        .sum();
    if done_bytes > 0 {
        // Use a hidden progress to report pre-existing bytes
        for (i, &done) in existing_completed.iter().enumerate() {
            if done {
                let wp = prog.worker_progress(0, i, block_plan.blocks[i].expected_size);
                wp.inc(block_plan.blocks[i].expected_size);
            }
        }
        // 清除初始 done_bytes 瞬时 inc 对速度估算的污染
        prog.reset_overall_eta();
    }

    // Spawn N worker tasks
    let supports_range = block_plan.supports_range;
    let mut handles = Vec::new();
    for worker_id in 0..num_workers {
        let url = args.url.clone();
        let timeout = args.timeout;
        let retry = args.retry;
        let pool = pool.clone();
        let scheduler = scheduler.clone();
        let prog = prog.clone();

        let handle = tokio::spawn(async move {
            loop {
                let assignment = match scheduler.acquire_next() {
                    Some(a) => a,
                    None => break,
                };

                let block_index = assignment.block.index;
                let block_prog = prog.worker_progress(
                    worker_id,
                    block_index,
                    assignment.block.expected_size,
                );

                match worker::download_block(
                    assignment.block,
                    &url,
                    &assignment.block_dir,
                    timeout,
                    retry,
                    pool.clone(),
                    block_prog,
                    supports_range,
                )
                .await
                {
                    Ok(result) => {
                        scheduler.mark_complete(result.index);
                    }
                    Err(e) => {
                        eprintln!(
                            "Worker {}: block {} failed: {}",
                            worker_id, block_index, e
                        );
                        let failures = scheduler.record_failure(block_index);
                        scheduler.release(block_index);
                        if failures >= block::MAX_FAILURES_PER_BLOCK {
                            eprintln!(
                                "Worker {}: block {} reached failure threshold ({}), will skip",
                                worker_id, block_index, failures
                            );
                        }
                        continue;
                    }
                }
            }
        });
        handles.push(handle);
    }

    // Await all workers
    let mut panics = Vec::new();
    for handle in handles {
        if let Err(e) = handle.await {
            eprintln!("Worker task panicked: {e}");
            panics.push(e.to_string());
        }
    }

    let failed = scheduler.failed_blocks();
    if !panics.is_empty() || !failed.is_empty() {
        let mut msg = String::new();
        if !failed.is_empty() {
            msg.push_str(&format!(
                "{} block(s) reached failure threshold {:?}; ",
                failed.len(),
                failed
            ));
        }
        if !panics.is_empty() {
            msg.push_str(&format!(
                "{} worker task(s) panicked; ",
                panics.len()
            ));
        }
        anyhow::bail!(
            "Download failed: {}re-run to resume (already-downloaded bytes are preserved)",
            msg
        );
    }

    // Merge blocks into final file
    prog.finish();
    eprintln!("Merging {} blocks into {} ...", block_plan.blocks.len(), output);
    merger::merge_blocks(
        &block_dir,
        &block_plan.blocks,
        Path::new(&output),
        block_plan.total_size,
    )
    .await?;

    // Cleanup hidden directory
    let _ = tokio::fs::remove_dir_all(&block_dir).await;

    eprintln!("\nDownload complete: {}", output);

    Ok(())
}
