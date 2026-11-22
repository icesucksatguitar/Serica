//! Download manager.
//!
//! Every download a tab starts is taken over from WebView2's built-in flyout
//! (`DownloadStarting` → `Handled`), tracked here, broadcast to the UI and kept
//! in `downloads.json`. The live `ICoreWebView2DownloadOperation`s are not
//! `Send`, so they stay in a UI-thread map and actions are marshalled there.

use crate::app::{ui_emit, Shared};
use crate::model::now_ms;
use crate::store::{read_json, write_atomic};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_ENTRIES: usize = 300;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadState {
    InProgress,
    Paused,
    Completed,
    Canceled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadItem {
    pub id: String,
    pub url: String,
    pub filename: String,
    pub path: String,
    #[serde(default)]
    pub mime: String,
    /// Total size, or -1 when the server did not say.
    pub total_bytes: i64,
    pub received_bytes: i64,
    /// Smoothed transfer rate (bytes/s) while in progress.
    #[serde(default)]
    pub bytes_per_sec: f64,
    pub state: DownloadState,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub can_resume: bool,
    pub started_at: i64,
    #[serde(default)]
    pub ended_at: Option<i64>,
    #[serde(default)]
    pub tab_id: Option<String>,
    /// False once a finished file was moved or deleted (checked on listing).
    #[serde(default = "yes")]
    pub file_exists: bool,
}

fn yes() -> bool {
    true
}

impl DownloadItem {
    pub fn is_active(&self) -> bool {
        matches!(self.state, DownloadState::InProgress | DownloadState::Paused)
    }
}

struct Sample {
    at: Instant,
    bytes: i64,
    emitted: Instant,
}

pub struct Downloads {
    file: PathBuf,
    items: Mutex<Vec<DownloadItem>>,
    samples: Mutex<HashMap<String, Sample>>,
}

/// Human-readable text for a WebView2 interrupt reason.
pub fn interrupt_text(reason: i32) -> &'static str {
    match reason {
        1 => "File error",
        2 => "Access denied",
        3 => "Disk full",
        4 => "Path too long",
        5 => "File too large",
        6 => "Blocked as malicious",
        7 => "Temporary file error",
        8 => "Blocked by policy",
        9 => "Security check failed",
        10 => "File incomplete",
        11 => "File hash mismatch",
        12 => "Network error",
        13 => "Network timeout",
        14 => "Network disconnected",
        15 => "Server unreachable",
        16 => "Invalid request",
        17 => "Server error",
        18 => "Server does not support resuming",
        19 => "Server returned bad content",
        20 => "Unauthorized",
        21 => "Certificate problem",
        22 => "Forbidden",
        23 => "Unexpected server response",
        24 => "Size mismatch",
        25 => "Cross-origin redirect blocked",
        26 => "Canceled",
        27 => "Browser closed",
        28 => "Paused",
        29 => "Download process crashed",
        _ => "Failed",
    }
}

impl Downloads {
    pub fn new(file: PathBuf) -> Self {
        let mut items: Vec<DownloadItem> = read_json(&file).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        // Operations do not survive a restart.
        for it in items.iter_mut().filter(|i| i.is_active()) {
            it.state = DownloadState::Failed;
            it.error = Some("Browser closed".into());
            it.can_resume = false;
            it.bytes_per_sec = 0.0;
            it.ended_at.get_or_insert_with(now_ms);
        }
        Self { file, items: Mutex::new(items), samples: Mutex::new(HashMap::new()) }
    }

    fn save(&self) {
        let items = self.items.lock().clone();
        if let Ok(bytes) = serde_json::to_vec(&items) {
            if let Err(e) = write_atomic(&self.file, &bytes) {
                tracing::warn!("failed to save downloads: {e}");
            }
        }
    }

    /// Newest first; finished entries are checked against the disk.
    pub fn list(&self) -> Vec<DownloadItem> {
        let mut items = self.items.lock().clone();
        for it in items.iter_mut().filter(|i| i.state == DownloadState::Completed) {
            it.file_exists = Path::new(&it.path).exists();
        }
        items.reverse();
        items
    }

    pub fn get(&self, id: &str) -> Option<DownloadItem> {
        self.items.lock().iter().find(|i| i.id == id).cloned()
    }

    pub fn add(&self, item: DownloadItem) {
        {
            let mut items = self.items.lock();
            items.push(item.clone());
            let excess = items.len().saturating_sub(MAX_ENTRIES);
            if excess > 0 {
                // Drop the oldest finished entries.
                let mut dropped = 0;
                items.retain(|i| {
                    if dropped < excess && !i.is_active() {
                        dropped += 1;
                        false
                    } else {
                        true
                    }
                });
            }
        }
        let now = Instant::now();
        self.samples.lock().insert(item.id.clone(), Sample { at: now, bytes: item.received_bytes, emitted: now });
        self.save();
    }

    /// Applies a progress sample; returns the item when the UI should be told.
    pub fn progress(&self, id: &str, received: i64, total: i64) -> Option<DownloadItem> {
        let now = Instant::now();
        let mut samples = self.samples.lock();
        let s = samples.entry(id.to_string()).or_insert(Sample { at: now, bytes: received, emitted: now - PROGRESS_INTERVAL });
        let mut items = self.items.lock();
        let it = items.iter_mut().find(|i| i.id == id)?;
        let dt = now.duration_since(s.at).as_secs_f64();
        if dt >= 0.2 {
            let rate = (received - s.bytes).max(0) as f64 / dt;
            it.bytes_per_sec = if it.bytes_per_sec > 0.0 { it.bytes_per_sec * 0.6 + rate * 0.4 } else { rate };
            s.at = now;
            s.bytes = received;
        }
        it.received_bytes = received;
        if total >= 0 {
            it.total_bytes = total;
        }
        if now.duration_since(s.emitted) < PROGRESS_INTERVAL {
            return None;
        }
        s.emitted = now;
        Some(it.clone())
    }

    /// Records a state change; persists and returns the updated item.
    pub fn set_state(&self, id: &str, f: impl FnOnce(&mut DownloadItem)) -> Option<DownloadItem> {
        let item = {
            let mut items = self.items.lock();
            let it = items.iter_mut().find(|i| i.id == id)?;
            f(it);
            if !it.is_active() {
                it.bytes_per_sec = 0.0;
                it.ended_at.get_or_insert_with(now_ms);
            }
            if it.state == DownloadState::Paused {
                it.bytes_per_sec = 0.0;
            }
            it.clone()
        };
        if !item.is_active() {
            self.samples.lock().remove(id);
        }
        self.save();
        Some(item)
    }

    pub fn remove(&self, id: &str) -> bool {
        let removed = {
            let mut items = self.items.lock();
            let before = items.len();
            items.retain(|i| i.id != id || i.is_active());
            before != items.len()
        };
        if removed {
            self.save();
        }
        removed
    }

    /// Removes every finished entry (files stay on disk).
    pub fn clear_finished(&self) {
        self.items.lock().retain(|i| i.is_active());
        self.save();
    }
}

pub fn emit(app: &Shared, item: &DownloadItem) {
    ui_emit(&app.handle, "neutrino:download-updated", item.clone());
}

/// Default folder WebView2 saves to (the user's Downloads folder).
pub fn default_folder() -> PathBuf {
    std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Downloads")).unwrap_or_else(std::env::temp_dir)
}

/// Opens Explorer with `path` selected (or its folder if the file is gone).
pub fn reveal(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("explorer.exe");
        if path.exists() {
            cmd.raw_arg(format!("/select,\"{}\"", path.display()));
        } else {
            let dir = path.parent().filter(|d| d.exists()).map(Path::to_path_buf).unwrap_or_else(default_folder);
            cmd.arg(dir);
        }
        cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        open::that(path.parent().unwrap_or(path)).map_err(|e| e.to_string())
    }
}

#[cfg(windows)]
pub mod native {
    use super::*;
    use std::cell::RefCell;
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use webview2_com::*;
    use windows::core::{Interface, PWSTR};

    thread_local! {
        static OPS: RefCell<HashMap<String, ICoreWebView2DownloadOperation>> = RefCell::new(HashMap::new());
    }

    fn take(p: PWSTR) -> String {
        if p.is_null() {
            return String::new();
        }
        let s = unsafe { p.to_string() }.unwrap_or_default();
        unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _)) };
        s
    }

    unsafe fn read_state(op: &ICoreWebView2DownloadOperation) -> (DownloadState, Option<String>, bool) {
        let mut st = COREWEBVIEW2_DOWNLOAD_STATE::default();
        let _ = op.State(&mut st);
        let mut reason = COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON::default();
        let _ = op.InterruptReason(&mut reason);
        let mut can = Default::default();
        let _ = op.CanResume(&mut can);
        let can = can.as_bool();
        if st == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED {
            (DownloadState::Completed, None, false)
        } else if st == COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS {
            (DownloadState::InProgress, None, false)
        } else if reason == COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_USER_PAUSED {
            (DownloadState::Paused, None, can)
        } else if reason == COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_USER_CANCELED {
            (DownloadState::Canceled, None, false)
        } else {
            (DownloadState::Failed, Some(interrupt_text(reason.0).to_string()), can)
        }
    }

    /// Hooks `DownloadStarting` on a tab webview. UI thread only.
    pub unsafe fn install(app: &Shared, tab_id: &str, core: &ICoreWebView2) -> windows::core::Result<()> {
        let Ok(core4) = core.cast::<ICoreWebView2_4>() else { return Ok(()) };
        let (app, tab_id) = (app.clone(), tab_id.to_string());
        let mut token = Default::default();
        core4.add_DownloadStarting(
            &DownloadStartingEventHandler::create(Box::new(move |sender, args| {
                let Some(args) = args else { return Ok(()) };
                // Our manager replaces the built-in download flyout.
                args.SetHandled(true)?;
                // A download is a navigation that never commits: the page stays,
                // so put the tab's URL back (NavigationStarting already moved it).
                if let Some(core) = &sender {
                    let mut src = PWSTR::null();
                    if core.Source(&mut src).is_ok() {
                        let src = take(src);
                        if !src.is_empty() {
                            app.tab_event(&tab_id, crate::app::TabEvent::UrlChanged(src));
                        }
                    }
                }
                let op = args.DownloadOperation()?;
                let mut p = PWSTR::null();
                args.ResultFilePath(&mut p)?;
                let path = take(p);
                let mut u = PWSTR::null();
                op.Uri(&mut u)?;
                let mut m = PWSTR::null();
                let _ = op.MimeType(&mut m);
                let mut total = -1i64;
                let _ = op.TotalBytesToReceive(&mut total);
                let id = uuid::Uuid::new_v4().to_string();
                let filename = Path::new(&path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "download".into());
                let item = DownloadItem {
                    id: id.clone(),
                    url: take(u),
                    filename,
                    path,
                    mime: take(m),
                    total_bytes: total,
                    received_bytes: 0,
                    bytes_per_sec: 0.0,
                    state: DownloadState::InProgress,
                    error: None,
                    can_resume: false,
                    started_at: now_ms(),
                    ended_at: None,
                    tab_id: Some(tab_id.clone()),
                    file_exists: true,
                };
                app.downloads.add(item.clone());
                ui_emit(&app.handle, "neutrino:download-started", item);

                let mut t = Default::default();
                {
                    let (app, id) = (app.clone(), id.clone());
                    op.add_BytesReceivedChanged(
                        &BytesReceivedChangedEventHandler::create(Box::new(move |sender, _| {
                            let Some(op) = sender else { return Ok(()) };
                            let (mut got, mut total) = (0i64, -1i64);
                            op.BytesReceived(&mut got)?;
                            let _ = op.TotalBytesToReceive(&mut total);
                            if let Some(item) = app.downloads.progress(&id, got, total) {
                                emit(&app, &item);
                            }
                            Ok(())
                        })),
                        &mut t,
                    )?;
                }
                {
                    let (app, id) = (app.clone(), id.clone());
                    op.add_StateChanged(
                        &StateChangedEventHandler::create(Box::new(move |sender, _| {
                            let Some(op) = sender else { return Ok(()) };
                            let (state, error, can_resume) = read_state(&op);
                            let mut got = 0i64;
                            let _ = op.BytesReceived(&mut got);
                            let mut total = -1i64;
                            let _ = op.TotalBytesToReceive(&mut total);
                            let mut p = PWSTR::null();
                            let _ = op.ResultFilePath(&mut p);
                            let path = take(p);
                            let updated = app.downloads.set_state(&id, |it| {
                                it.state = state;
                                it.error = error;
                                it.can_resume = can_resume;
                                it.received_bytes = got;
                                if total >= 0 {
                                    it.total_bytes = total;
                                }
                                if !path.is_empty() {
                                    it.filename = Path::new(&path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| it.filename.clone());
                                    it.path = path;
                                }
                                if state == DownloadState::InProgress {
                                    it.ended_at = None;
                                }
                            });
                            if let Some(item) = updated {
                                if !item.is_active() && !item.can_resume {
                                    OPS.with(|ops| ops.borrow_mut().remove(&id));
                                }
                                emit(&app, &item);
                            }
                            Ok(())
                        })),
                        &mut t,
                    )?;
                }
                OPS.with(|ops| ops.borrow_mut().insert(id, op));
                Ok(())
            })),
            &mut token,
        )?;
        Ok(())
    }

    /// Pause / resume / cancel a live download. UI thread only.
    pub fn control(id: &str, action: &str) -> bool {
        OPS.with(|ops| {
            let ops = ops.borrow();
            let Some(op) = ops.get(id) else { return false };
            unsafe {
                match action {
                    "pause" => op.Pause().is_ok(),
                    "resume" => op.Resume().is_ok(),
                    "cancel" => op.Cancel().is_ok(),
                    _ => false,
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, state: DownloadState) -> DownloadItem {
        DownloadItem {
            id: id.into(),
            url: "https://example.com/f.zip".into(),
            filename: "f.zip".into(),
            path: "C:\\nope\\f.zip".into(),
            mime: String::new(),
            total_bytes: 100,
            received_bytes: 0,
            bytes_per_sec: 0.0,
            state,
            error: None,
            can_resume: false,
            started_at: 1,
            ended_at: None,
            tab_id: None,
            file_exists: true,
        }
    }

    fn temp_file(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("neutrino-dl-test-{name}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn active_downloads_fail_after_restart() {
        let file = temp_file("restart");
        let d = Downloads::new(file.clone());
        d.add(item("a", DownloadState::InProgress));
        d.add(item("b", DownloadState::Completed));
        let d2 = Downloads::new(file.clone());
        let a = d2.get("a").unwrap();
        assert_eq!(a.state, DownloadState::Failed);
        assert_eq!(a.error.as_deref(), Some("Browser closed"));
        assert_eq!(d2.get("b").unwrap().state, DownloadState::Completed);
        // Newest first, and missing files are flagged.
        let list = d2.list();
        assert_eq!(list[0].id, "b");
        assert!(!list[0].file_exists);
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn clear_keeps_active_and_remove_skips_active() {
        let file = temp_file("clear");
        let d = Downloads::new(file.clone());
        d.add(item("a", DownloadState::InProgress));
        d.add(item("b", DownloadState::Completed));
        assert!(!d.remove("a"));
        d.clear_finished();
        let ids: Vec<_> = d.list().into_iter().map(|i| i.id).collect();
        assert_eq!(ids, vec!["a"]);
        let done = d.set_state("a", |i| i.state = DownloadState::Canceled).unwrap();
        assert!(done.ended_at.is_some());
        assert!(d.remove("a"));
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn progress_is_throttled() {
        let d = Downloads::new(temp_file("progress"));
        d.add(item("a", DownloadState::InProgress));
        assert!(d.progress("a", 10, 100).is_none());
        assert_eq!(d.get("a").unwrap().received_bytes, 10);
        std::thread::sleep(PROGRESS_INTERVAL);
        let it = d.progress("a", 50, 100).unwrap();
        assert_eq!(it.received_bytes, 50);
        assert!(it.bytes_per_sec > 0.0);
    }
}
