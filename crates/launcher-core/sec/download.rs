//! Resumable-safe, checksummed, cancellable downloads with shared progress.
use crate::error::{Error, Result};
use futures_util::{StreamExt, stream};
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone)]
pub struct DownloadTask {
    pub url: String,
    pub dest: PathBuf,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

/// Shared progress, safe to poll from the UI thread every frame.
#[derive(Debug, Default)]
pub struct Progress {
    pub bytes_done: AtomicU64,
    pub bytes_total: AtomicU64,
    pub files_done: AtomicU64,
    pub files_total: AtomicU64,
    pub label: Mutex<String>,
    pub cancel: AtomicBool,
}

impl Progress {
    pub fn new() -> Arc<Self> { Arc::new(Self::default()) }
    pub fn fraction(&self) -> f32 {
        let t = self.bytes_total.load(Ordering::Relaxed);
        if t == 0 {
            let ft = self.files_total.load(Ordering::Relaxed);
            if ft == 0 { return 0.0; }
            return self.files_done.load(Ordering::Relaxed) as f32 / ft as f32;
        }
        (self.bytes_done.load(Ordering::Relaxed) as f32 / t as f32).clamp(0.0, 1.0)
    }
    pub fn set_label(&self, s: impl Into<String>) { *self.label.lock().unwrap() = s.into(); }
    pub fn label(&self) -> String { self.label.lock().unwrap().clone() }
    pub fn cancel(&self) { self.cancel.store(true, Ordering::Relaxed); }
    pub fn is_cancelled(&self) -> bool { self.cancel.load(Ordering::Relaxed) }
}

pub fn sha1_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 { break; }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

fn already_valid(t: &DownloadTask) -> bool {
    let Ok(meta) = std::fs::metadata(&t.dest) else { return false };
    if let Some(sz) = t.size { if meta.len() != sz { return false; } }
    match &t.sha1 {
        Some(h) => sha1_file(&t.dest).map(|a| a.eq_ignore_ascii_case(h)).unwrap_or(false),
        None => true,
    }
}

async fn fetch_once(client: &reqwest::Client, t: &DownloadTask, p: &Progress) -> Result<()> {
    if let Some(parent) = t.dest.parent() { tokio::fs::create_dir_all(parent).await?; }
    let part = t.dest.with_extension("part");
    let resp = client.get(&t.url).send().await?.error_for_status()?;
    let mut stream = resp.bytes_stream();
    let mut file = tokio::fs::File::create(&part).await?;
    let mut hasher = Sha1::new();
    while let Some(chunk) = stream.next().await {
        if p.is_cancelled() {
            drop(file);
            let _ = tokio::fs::remove_file(&part).await;
            return Err(Error::Cancelled);
        }
        let chunk = chunk?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        p.bytes_done.fetch_add(chunk.len() as u64, Ordering::Relaxed);
    }
    file.flush().await?;
    drop(file);
    if let Some(expected) = &t.sha1 {
        let actual = hex::encode(hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(Error::Checksum { file: t.dest.display().to_string(), expected: expected.clone(), actual });
        }
    }
    tokio::fs::rename(&part, &t.dest).await?;
    Ok(())
}

pub fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("RustcraftLauncher/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(300))
        .build()?)
}

/// Download one file with up to 3 attempts; skips files that already verify.
pub async fn download_one(client: &reqwest::Client, t: &DownloadTask, p: &Progress) -> Result<()> {
    if already_valid(t) {
        if let Some(sz) = t.size { p.bytes_done.fetch_add(sz, Ordering::Relaxed); }
        p.files_done.fetch_add(1, Ordering::Relaxed);
        return Ok(());
    }
    let mut last = None;
    for attempt in 0..3u32 {
        if p.is_cancelled() { return Err(Error::Cancelled); }
        match fetch_once(client, t, p).await {
            Ok(()) => { p.files_done.fetch_add(1, Ordering::Relaxed); return Ok(()); }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) => {
                last = Some(e);
                tokio::time::sleep(std::time::Duration::from_millis(400 * (attempt as u64 + 1))).await;
            }
        }
    }
    Err(last.unwrap_or_else(|| Error::Net("download failed".into())))
}

pub async fn download_all(client: &reqwest::Client, tasks: Vec<DownloadTask>, concurrency: usize, p: Arc<Progress>) -> Result<()> {
    p.files_total.fetch_add(tasks.len() as u64, Ordering::Relaxed);
    p.bytes_total.fetch_add(tasks.iter().filter_map(|t| t.size).sum::<u64>(), Ordering::Relaxed);
    let results: Vec<Result<()>> = stream::iter(tasks)
        .map(|t| {
            let c = client.clone();
            let p = p.clone();
            async move { download_one(&c, &t, &p).await }
        })
        .buffer_unordered(concurrency.max(1))
        .collect()
        .await;
    for r in results { r?; }
    Ok(())
}

pub async fn get_json_text(client: &reqwest::Client, url: &str) -> Result<String> {
    Ok(client.get(url).send().await?.error_for_status()?.text().await?)
}
