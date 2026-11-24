//! Downloads manager: fetches a resource on a worker thread, writes it to
//! disk and reports progress to the UI.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use veyra_common::util::human_bytes;
use veyra_common::Url;

use crate::client::{guess_mime, Client, NetError, Request, ResourceType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStatus {
    Pending,
    Running,
    Complete,
    Failed,
    Canceled,
}

#[derive(Debug, Clone)]
pub struct DownloadState {
    pub status: DownloadStatus,
    pub bytes_done: u64,
    pub total: u64,
    pub error: Option<String>,
    pub elapsed_ms: u64,
}

pub struct Download {
    pub id: u64,
    pub url: String,
    pub filename: String,
    pub target: PathBuf,
    pub state: Arc<Mutex<DownloadState>>,
    pub cancel: Arc<AtomicBool>,
    pub started: Instant,
    pub mime: String,
}

impl Download {
    pub fn progress(&self) -> f32 {
        let s = self.state.lock().unwrap();
        if s.total == 0 {
            if s.status == DownloadStatus::Complete {
                1.0
            } else {
                0.0
            }
        } else {
            (s.bytes_done as f32 / s.total as f32).clamp(0.0, 1.0)
        }
    }
    pub fn summary(&self) -> String {
        let s = self.state.lock().unwrap();
        match s.status {
            DownloadStatus::Complete => format!("{} • {}", human_bytes(s.bytes_done), "complete"),
            DownloadStatus::Failed => format!("failed: {}", s.error.clone().unwrap_or_default()),
            DownloadStatus::Canceled => "canceled".into(),
            DownloadStatus::Running => format!("{} / {}", human_bytes(s.bytes_done), human_bytes(s.total)),
            DownloadStatus::Pending => "starting…".into(),
        }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

#[derive(Default)]
pub struct DownloadManager {
    pub items: Vec<Download>,
    next_id: u64,
}

impl DownloadManager {
    pub fn new() -> Self {
        DownloadManager { items: Vec::new(), next_id: 1 }
    }

    pub fn start(&mut self, client: Arc<Client>, url: &str, directory: PathBuf) -> Result<u64, NetError> {
        let parsed = Url::parse(url).ok_or_else(|| NetError::InvalidUrl(url.to_string()))?;
        let mut filename = parsed.path.rsplit('/').next().unwrap_or("").to_string();
        if filename.is_empty() || filename == "/" {
            filename = "download".into();
        }
        let filename = sanitise_filename(&filename);
        let id = self.next_id;
        self.next_id += 1;
        let state = Arc::new(Mutex::new(DownloadState {
            status: DownloadStatus::Pending,
            bytes_done: 0,
            total: 0,
            error: None,
            elapsed_ms: 0,
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let target = directory.join(&filename);
        let mut download = Download {
            id,
            url: url.to_string(),
            filename: filename.clone(),
            target: target.clone(),
            state: state.clone(),
            cancel: cancel.clone(),
            started: Instant::now(),
            mime: "application/octet-stream".into(),
        };
        download.mime = parsed
            .file_extension()
            .and_then(|ext| guess_mime(&format!("x.{ext}")))
            .unwrap_or("application/octet-stream")
            .to_string();
        let url_owned = url.to_string();
        let cancel_thread = cancel.clone();
        let state_thread = state.clone();
        std::thread::spawn(move || {
            {
                let mut s = state_thread.lock().unwrap();
                s.status = DownloadStatus::Running;
            }
            let started = Instant::now();
            let mut req = Request::get(url_owned.clone());
            req.resource_type = ResourceType::Other;
            let result = client.fetch(req);
            if cancel_thread.load(Ordering::SeqCst) {
                let mut s = state_thread.lock().unwrap();
                s.status = DownloadStatus::Canceled;
                return;
            }
            let mut finish = |status: DownloadStatus, err: Option<String>, bytes: u64| {
                let mut s = state_thread.lock().unwrap();
                s.status = status;
                s.error = err;
                s.bytes_done = bytes;
                s.total = bytes;
                s.elapsed_ms = started.elapsed().as_millis() as u64;
            };
            match result {
                Ok(resp) if resp.is_success() => {
                    let len = resp.body.len() as u64;
                    if let Some(dir) = target.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    match std::fs::File::create(&target).and_then(|mut f| f.write_all(&resp.body)) {
                        Ok(()) => finish(DownloadStatus::Complete, None, len),
                        Err(e) => finish(DownloadStatus::Failed, Some(e.to_string()), 0),
                    }
                }
                Ok(resp) => finish(
                    DownloadStatus::Failed,
                    Some(format!("HTTP {}", resp.status)),
                    0,
                ),
                Err(e) => finish(DownloadStatus::Failed, Some(e.to_string()), 0),
            }
        });
        self.items.push(download);
        Ok(id)
    }

    pub fn active(&self) -> usize {
        self.items
            .iter()
            .filter(|d| matches!(d.state.lock().unwrap().status, DownloadStatus::Running | DownloadStatus::Pending))
            .count()
    }

    pub fn clear_finished(&mut self) {
        self.items.retain(|d| {
            !matches!(
                d.state.lock().unwrap().status,
                DownloadStatus::Complete | DownloadStatus::Failed | DownloadStatus::Canceled
            )
        });
    }
}

fn sanitise_filename(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '_' } else { c })
        .collect();
    out = out.trim_matches('.').to_string();
    if out.is_empty() {
        out = "download".into();
    }
    out
}

pub fn default_download_dir() -> PathBuf {
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        PathBuf::from(home).join("Downloads")
    } else {
        PathBuf::from("downloads")
    }
}

pub fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{} ms", ms)
    } else {
        format!("{:.1} s", ms as f64 / 1000.0)
    }
}

pub fn wait_until_done(download: &Download, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        let status = download.state.lock().unwrap().status;
        if !matches!(status, DownloadStatus::Pending | DownloadStatus::Running) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}
