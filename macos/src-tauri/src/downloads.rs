//! Download manager.
//!
//! Every download a tab starts becomes a `WKDownload` that Serica manages
//! (WebKit has no download UI of its own): tracked here, broadcast to the UI
//! and kept in `downloads.json`. Live downloads are main-thread objects, so
//! they stay in a main-thread map and actions are marshalled there.

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
    ui_emit(&app.handle, "serica:download-updated", item.clone());
}

/// The user's Downloads folder.
pub fn default_folder() -> PathBuf {
    std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Downloads")).filter(|p| p.is_dir()).unwrap_or_else(std::env::temp_dir)
}

/// `dir/name`, or "name (1).ext", "name (2).ext"... if taken (as browsers name them).
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let clean: String = name.chars().map(|c| if c == '/' || c == ':' || c == '\0' { '_' } else { c }).collect();
    let clean = clean.trim_start_matches('.').trim();
    let clean = if clean.is_empty() { "download" } else { clean };
    let mut p = dir.join(clean);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = clean.split_once('.').map(|(b, e)| (b.to_string(), format!(".{e}"))).unwrap_or((clean.to_string(), String::new()));
    let mut n = 1;
    while p.exists() {
        p = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    p
}

/// Shows `path` selected in Finder (or its folder if the file is gone).
pub fn reveal(path: &Path) -> Result<(), String> {
    if path.exists() {
        let status = std::process::Command::new("/usr/bin/open").arg("-R").arg(path).status().map_err(|e| e.to_string())?;
        return if status.success() { Ok(()) } else { Err("Finder could not show the file".into()) };
    }
    let dir = path.parent().filter(|d| d.exists()).map(Path::to_path_buf).unwrap_or_else(default_folder);
    open::that_detached(dir).map_err(|e| e.to_string())
}

pub mod native {
    use super::*;
    use crate::mac_util::{ns, post_main, url_string};
    use block2::RcBlock;
    use objc2::rc::{Retained, Weak};
    use objc2::runtime::{NSObject, ProtocolObject};
    use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly, Message};
    use objc2_foundation::{NSData, NSError, NSObjectProtocol, NSProgress, NSString, NSURLResponse, NSURL};
    use objc2_web_kit::{WKDownload, WKDownloadDelegate, WKWebView};
    use std::cell::RefCell;
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Op {
        download: Option<Retained<WKDownload>>,
        resume: Option<Retained<NSData>>,
        webview: Weak<WKWebView>,
        /// Set while a pause (cancel keeping resume data) is under way.
        pausing: bool,
        canceling: bool,
    }

    thread_local! {
        static OPS: RefCell<HashMap<String, Op>> = RefCell::new(HashMap::new());
        static BY_PTR: RefCell<HashMap<usize, (String, String)>> = RefCell::new(HashMap::new());
        static DELEGATE: RefCell<Option<Retained<SericaDownloadDelegate>>> = const { RefCell::new(None) };
    }
    static TICKING: AtomicBool = AtomicBool::new(false);

    fn ptr(d: &WKDownload) -> usize {
        d as *const WKDownload as usize
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        struct SericaDownloadDelegate;

        unsafe impl NSObjectProtocol for SericaDownloadDelegate {}

        unsafe impl WKDownloadDelegate for SericaDownloadDelegate {
            #[unsafe(method(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:))]
            fn destination(&self, download: &WKDownload, response: &NSURLResponse, suggested: &NSString, handler: &block2::Block<dyn Fn(*mut NSURL)>) {
                decide_destination(download, response, suggested, handler);
            }

            #[unsafe(method(downloadDidFinish:))]
            fn finished(&self, download: &WKDownload) {
                did_finish(download);
            }

            #[unsafe(method(download:didFailWithError:resumeData:))]
            fn failed(&self, download: &WKDownload, error: &NSError, resume: Option<&NSData>) {
                did_fail(download, error, resume);
            }
        }
    );

    fn delegate() -> Retained<SericaDownloadDelegate> {
        let mtm = MainThreadMarker::new().expect("main thread");
        DELEGATE.with(|d| d.borrow_mut().get_or_insert_with(|| unsafe { msg_send![SericaDownloadDelegate::alloc(mtm), init] }).clone())
    }

    /// Takes over a download a tab started (a link, a response WebKit can't
    /// show, an attachment). Main thread.
    pub fn adopt(app: &Shared, tab_id: &str, download: &WKDownload) {
        let id = uuid::Uuid::new_v4().to_string();
        adopt_as(app, tab_id, download, id);
    }

    fn adopt_as(app: &Shared, tab_id: &str, download: &WKDownload, id: String) {
        unsafe {
            let d = delegate();
            download.setDelegate(Some(ProtocolObject::from_ref(&*d)));
            let wv = download.webView();
            BY_PTR.with(|m| m.borrow_mut().insert(ptr(download), (id.clone(), tab_id.to_string())));
            let webview = wv.as_deref().map(Weak::from).unwrap_or_default();
            OPS.with(|o| o.borrow_mut().insert(id, Op { download: Some(download.retain()), resume: None, webview, pausing: false, canceling: false }));
        }
        let _ = app;
    }

    fn lookup(download: &WKDownload) -> Option<(String, String)> {
        BY_PTR.with(|m| m.borrow().get(&ptr(download)).cloned())
    }

    fn decide_destination(download: &WKDownload, response: &NSURLResponse, suggested: &NSString, handler: &block2::Block<dyn Fn(*mut NSURL)>) {
        let Some(app) = crate::app::global() else { return handler.call((std::ptr::null_mut(),)) };
        let Some((id, tab_id)) = lookup(download) else { return handler.call((std::ptr::null_mut(),)) };
        let name = suggested.to_string();
        // A resumed download keeps the file it was saving to.
        let existing = app.downloads.get(&id).filter(|i| !i.path.is_empty()).map(|i| PathBuf::from(&i.path));
        let path = existing.clone().unwrap_or_else(|| unique_path(&default_folder(), &name));
        unsafe {
            let url = url_string(download.originalRequest().and_then(|r| r.URL()).as_deref());
            let mime = response.MIMEType().map(|m| m.to_string()).unwrap_or_default();
            let total = response.expectedContentLength();
            if existing.is_none() {
                let item = DownloadItem {
                    id: id.clone(),
                    url,
                    filename: path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "download".into()),
                    path: path.to_string_lossy().to_string(),
                    mime,
                    total_bytes: if total > 0 { total } else { -1 },
                    received_bytes: 0,
                    bytes_per_sec: 0.0,
                    state: DownloadState::InProgress,
                    error: None,
                    can_resume: false,
                    started_at: now_ms(),
                    ended_at: None,
                    tab_id: Some(tab_id),
                    file_exists: true,
                };
                app.downloads.add(item.clone());
                ui_emit(&app.handle, "serica:download-started", item);
            } else if let Some(item) = app.downloads.set_state(&id, |i| {
                i.state = DownloadState::InProgress;
                i.error = None;
                i.can_resume = false;
                i.ended_at = None;
            }) {
                emit(app, &item);
            }
            let dest = NSURL::fileURLWithPath(&ns(&path.to_string_lossy()));
            handler.call((Retained::as_ptr(&dest) as *mut NSURL,));
        }
        start_ticking(app);
    }

    /// Marks a finished download as coming from the internet (Gatekeeper
    /// checks downloaded apps, as for Safari's downloads).
    fn quarantine(path: &Path, url: &str) {
        use objc2_foundation::{NSDictionary, NSURLQuarantinePropertiesKey};
        let file = NSURL::fileURLWithPath(&ns(&path.to_string_lossy()));
        let agent = crate::app::global().and_then(|a| a.handle.config().product_name.clone()).unwrap_or_else(|| "Serica".into());
        let mut keys = vec![ns("LSQuarantineAgentName"), ns("LSQuarantineType")];
        let mut vals: Vec<objc2::rc::Retained<objc2::runtime::AnyObject>> = vec![ns(&agent).into(), ns("LSQuarantineTypeWebDownload").into()];
        // Where it came from (a URL object, as LaunchServices expects).
        if let Some(u) = crate::mac_util::nsurl(url).filter(|_| url.starts_with("http")) {
            keys.push(ns("LSQuarantineDataURL"));
            vals.push(u.into());
        }
        let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
        let val_refs: Vec<&objc2::runtime::AnyObject> = vals.iter().map(|v| &**v).collect();
        let dict = NSDictionary::from_slices(&key_refs, &val_refs);
        if let Err(e) = unsafe { file.setResourceValue_forKey_error(Some(&dict), NSURLQuarantinePropertiesKey) } {
            tracing::warn!("could not mark {} as downloaded: {}", path.display(), e.localizedDescription());
        }
    }

    fn did_finish(download: &WKDownload) {
        let Some(app) = crate::app::global() else { return };
        let Some((id, _)) = lookup(download) else { return };
        BY_PTR.with(|m| m.borrow_mut().remove(&ptr(download)));
        OPS.with(|o| o.borrow_mut().remove(&id));
        let updated = app.downloads.set_state(&id, |it| {
            it.state = DownloadState::Completed;
            if it.total_bytes > 0 {
                it.received_bytes = it.total_bytes;
            } else if let Ok(m) = std::fs::metadata(&it.path) {
                it.received_bytes = m.len() as i64;
                it.total_bytes = it.received_bytes;
            }
            it.error = None;
            it.can_resume = false;
        });
        if let Some(item) = updated {
            quarantine(Path::new(&item.path), &item.url);
            emit(app, &item);
        }
    }

    fn did_fail(download: &WKDownload, error: &NSError, resume: Option<&NSData>) {
        let user_cancel = error.domain().to_string() == "NSURLErrorDomain" && error.code() == -999;
        stopped(download, resume, Some(error.localizedDescription().to_string()), user_cancel);
    }

    /// A download that ended without finishing: failed, or stopped by Pause
    /// or Cancel. WebKit reports an explicit `cancel` only to its completion
    /// handler (with the resume data), a failure to the delegate; whichever
    /// comes first is handled, the other finds nothing left.
    fn stopped(download: &WKDownload, resume: Option<&NSData>, failure: Option<String>, user_cancel: bool) {
        let Some(app) = crate::app::global() else { return };
        let Some((id, _)) = lookup(download) else { return };
        BY_PTR.with(|m| m.borrow_mut().remove(&ptr(download)));
        let (pausing, canceling) = OPS.with(|o| {
            let mut o = o.borrow_mut();
            match o.get_mut(&id) {
                Some(op) => {
                    op.download = None;
                    op.resume = resume.map(|r| r.retain());
                    (op.pausing, op.canceling)
                }
                None => (false, false),
            }
        });
        let can_resume = resume.is_some_and(|r| r.length() > 0);
        let message = match failure {
            Some(m) if !m.is_empty() => m,
            _ if pausing => "This download can't be paused: the site doesn't let it carry on later.".to_string(),
            _ => "Failed".to_string(),
        };
        let updated = app.downloads.set_state(&id, |it| {
            if pausing && can_resume {
                it.state = DownloadState::Paused;
                it.can_resume = true;
                it.error = None;
            } else if canceling || (user_cancel && !pausing) {
                it.state = DownloadState::Canceled;
                it.can_resume = false;
                it.error = None;
                it.file_exists = false;
            } else {
                it.state = DownloadState::Failed;
                it.error = Some(message.clone());
                it.can_resume = can_resume;
            }
        });
        if let Some(item) = updated {
            // A cancelled download leaves nothing behind.
            if item.state == DownloadState::Canceled {
                let _ = std::fs::remove_file(&item.path);
            }
            if !item.can_resume {
                OPS.with(|o| o.borrow_mut().remove(&id));
            } else {
                OPS.with(|o| {
                    if let Some(op) = o.borrow_mut().get_mut(&id) {
                        op.pausing = false;
                    }
                });
            }
            emit(app, &item);
        }
    }

    /// Progress for the UI, four times a second while anything downloads.
    fn start_ticking(app: &Shared) {
        if TICKING.swap(true, Ordering::AcqRel) {
            return;
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(PROGRESS_INTERVAL).await;
                let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
                let a = app.clone();
                post_main(&app.handle, move || {
                    let mut any = false;
                    let samples: Vec<(String, i64, i64)> = OPS.with(|o| {
                        o.borrow()
                            .iter()
                            .filter_map(|(id, op)| {
                                let d = op.download.as_ref()?;
                                let p: Retained<NSProgress> = unsafe { msg_send![&**d, progress] };
                                Some((id.clone(), p.completedUnitCount(), p.totalUnitCount()))
                            })
                            .collect()
                    });
                    for (id, got, total) in samples {
                        any = true;
                        if let Some(item) = a.downloads.progress(&id, got, if total > 0 { total } else { -1 }) {
                            emit(&a, &item);
                        }
                    }
                    let _ = tx.send(any);
                });
                if !rx.await.unwrap_or(false) {
                    TICKING.store(false, Ordering::Release);
                    break;
                }
            }
        });
    }

    /// Pause / resume / cancel a live download. Main thread only.
    pub fn control(app: &Shared, id: &str, action: &str) -> bool {
        match action {
            "pause" => OPS.with(|o| {
                let mut o = o.borrow_mut();
                let Some(op) = o.get_mut(id) else { return false };
                let Some(d) = op.download.clone() else { return false };
                op.pausing = true;
                // Cancelling with resume data is WebKit's pause.
                let d2 = d.clone();
                let blk = RcBlock::new(move |data: *mut NSData| stopped(&d2, unsafe { data.as_ref() }, None, true));
                unsafe { d.cancel(Some(&blk)) };
                true
            }),
            "resume" => {
                let Some((data, webview)) = OPS.with(|o| o.borrow().get(id).and_then(|op| op.resume.clone().map(|r| (r, op.webview.clone())))) else { return false };
                // Any page of the profile can carry on (the one it came from may be gone).
                let tab_id = app.downloads.get(id).and_then(|i| i.tab_id).unwrap_or_default();
                let wv = webview.load().or_else(|| crate::webview_mac::any_tab_view(&tab_id));
                let Some(wv) = wv else { return false };
                let (app2, id2, tab2) = (app.clone(), id.to_string(), tab_id.clone());
                let blk = RcBlock::new(move |d: NonNull<WKDownload>| {
                    let d = unsafe { d.as_ref() };
                    adopt_as(&app2, &tab2, d, id2.clone());
                    start_ticking(&app2);
                });
                unsafe { wv.resumeDownloadFromResumeData_completionHandler(&data, &blk) };
                if let Some(item) = app.downloads.set_state(id, |i| {
                    i.state = DownloadState::InProgress;
                    i.can_resume = false;
                    i.ended_at = None;
                }) {
                    emit(app, &item);
                }
                true
            }
            "cancel" => {
                let live = OPS.with(|o| {
                    let mut o = o.borrow_mut();
                    let Some(op) = o.get_mut(id) else { return None };
                    op.canceling = true;
                    Some(op.download.clone())
                });
                match live {
                    Some(Some(d)) => {
                        let d2 = d.clone();
                        let blk = RcBlock::new(move |_data: *mut NSData| stopped(&d2, None, None, true));
                        unsafe { d.cancel(Some(&blk)) };
                        true
                    }
                    Some(None) => {
                        // Paused: drop what was kept for resuming.
                        OPS.with(|o| o.borrow_mut().remove(id));
                        if let Some(item) = app.downloads.set_state(id, |i| {
                            i.state = DownloadState::Canceled;
                            i.can_resume = false;
                            i.file_exists = false;
                        }) {
                            let _ = std::fs::remove_file(&item.path);
                            emit(app, &item);
                        }
                        true
                    }
                    None => false,
                }
            }
            _ => false,
        }
    }

    /// Downloads `url` from a page of the tab's profile (its cookies apply).
    pub fn start(app: &Shared, tab_id: &str, wv: &tauri::Webview, url: &str) {
        let (app, tab_id, url) = (app.clone(), tab_id.to_string(), url.to_string());
        crate::webview_mac::with_wk(wv, move |wk| unsafe {
            let Some(u) = crate::mac_util::nsurl(&url) else { return };
            let req = objc2_foundation::NSURLRequest::requestWithURL(&u);
            let blk = RcBlock::new(move |d: NonNull<WKDownload>| adopt(&app, &tab_id, d.as_ref()));
            wk.startDownloadUsingRequest_completionHandler(&req, &blk);
        });
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
            path: "/nonexistent/serica/f.zip".into(),
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
        let p = std::env::temp_dir().join(format!("serica-dl-test-{name}-{}.json", std::process::id()));
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
    fn colliding_names_are_numbered() {
        let dir = std::env::temp_dir().join(format!("serica-dl-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "report.pdf"), dir.join("report.pdf"));
        std::fs::write(dir.join("report.pdf"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "report.pdf"), dir.join("report (1).pdf"));
        std::fs::write(dir.join("report (1).pdf"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "report.pdf"), dir.join("report (2).pdf"));
        assert_eq!(unique_path(&dir, "../evil:name"), dir.join("_evil_name"));
        let _ = std::fs::remove_dir_all(&dir);
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
