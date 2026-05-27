mod cli;
mod config;
mod error;
mod merger;
mod planner;
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

    let first_proxy = proxy_addrs.first().map(|s| reqwest::Proxy::all(s)).transpose()?;
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

    // Plan chunks
    let plan = planner::plan(&head_client, &args.url, args.chunks).await?;

    if plan.supports_range {
        eprintln!(
            "File size: {} bytes, downloading in {} chunks",
            plan.total_size,
            plan.chunks.len()
        );
    } else {
        eprintln!(
            "File size: {} bytes, downloading (no range support)",
            plan.total_size
        );
    }

    // Setup progress
    let prog = Arc::new(progress::DownloadProgress::new(
        plan.total_size,
        plan.chunks.len(),
    ));

    // Spawn download tasks
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
    let mut handles = Vec::new();
    for chunk in plan.chunks {
        let url = args.url.clone();
        let dir = output_dir.clone();
        let fname = output_filename.clone();
        let timeout = args.timeout;
        let retry = args.retry;
        let pool = pool.clone();
        let chunk_prog = prog.chunk_progress(chunk.index);

        let handle = tokio::spawn(async move {
            worker::download_chunk(chunk, &url, &dir, &fname, timeout, retry, pool, chunk_prog)
                .await
        });
        handles.push(handle);
    }

    // Await all tasks
    let mut results = Vec::new();
    let mut errors = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Ok(result)) => results.push(result),
            Ok(Err(e)) => errors.push(e.to_string()),
            Err(e) => errors.push(format!("Task panicked: {e}")),
        }
    }

    if !errors.is_empty() {
        for err in &errors {
            eprintln!("Error: {err}");
        }
        anyhow::bail!(
            "Download failed: {} chunk(s) failed, re-run to resume",
            errors.len()
        );
    }

    // Merge
    merger::merge_chunks(results, Path::new(&output), plan.total_size).await?;

    prog.finish();
    eprintln!("Download complete: {}", output);

    Ok(())
}
