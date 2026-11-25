//! Application core: owns every backend service and keeps the UI, the
//! native tab webviews and the disk in sync.
//!
//! Locking rules (important, WebView2 callbacks run on the UI thread):
//! * `store`/`history` locks are held only for in-memory work. Never call a
//!   webview API (which may wait for the UI thread) while holding them.
//! * UI-thread callbacks never take the `host` lock.

use crate::downloads::Downloads;
use crate::extensions::Extensions;
use crate::history::History;
use crate::model::*;
use crate::settings::BrowserSettings;
use crate::shields::Shields;
use crate::store::Store;
use crate::tabhost::TabHost;
use crate::vpn::Vpn;
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

pub type Shared = Arc<App>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// Tab/space state changed: broadcast and reconcile webviews.
    State,
    /// Settings changed: broadcast and reconfigure services.
    Settings,
    History,
    /// Requests were blocked: refresh counters only (no reconcile, no save).
    Blocked,
    Quit,
}

/// Events reported by a tab's webview (from the UI thread).
#[derive(Debug, Clone)]
pub enum TabEvent {
    NavigationStarting(String),
    UrlChanged(String),
    History { back: bool, forward: bool },
    Loaded { url: String, success: bool },
    Title(String),
    Favicon(String),
    Audio(bool),
    Zoom(f64),
}

/// Browser-level shortcut pressed while a web page had keyboard focus.
#[derive(Debug, Clone, Copy)]
pub enum Shortcut {
    NewTab,
    OpenCommandBar,
    CloseTab,
    ReopenTab,
    CycleTab(i32),
    ToggleSidebar,
    Find,
    History,
    Downloads,
    Settings,
    SpaceIndex(usize),
    Escape,
    Home,
    OpenFile,
}

pub struct App {
    pub handle: AppHandle,
    pub store: Mutex<Store>,
    pub history: Mutex<History>,
    pub shields: Arc<Shields>,
    pub vpn: Arc<Vpn>,
    pub extensions: Extensions,
    pub downloads: Downloads,
    pub mcp_http: crate::mcp_http::McpHttp,
    pub host: Mutex<TabHost>,
    pub mcp_logs: Mutex<VecDeque<Value>>,
    blocked_queue: Mutex<Vec<BlockedTrackerItem>>,
    tx: Mutex<mpsc::Sender<Notice>>,
    /// Serializes saves (the sync loop and the updater may save at once).
    save_lock: Mutex<()>,
    /// Hash of what each saved file last held, to skip rewriting it unchanged.
    written: Mutex<std::collections::HashMap<std::path::PathBuf, u64>>,
    /// Dark-PDF setting the open pages were last told about.
    pdf_dark: std::sync::atomic::AtomicBool,
    /// Profiles whose cookies were already swept this session ("clear cookies
    /// when Serica closes" also runs at start, in case it could not at exit).
    cookie_swept: Mutex<std::collections::HashSet<String>>,
}

/// Emits to the app UI only, never to web pages in tab webviews.
pub fn ui_emit<S: serde::Serialize + Clone>(handle: &AppHandle, event: &str, payload: S) {
    let _ = handle.emit_to("main", event, payload);
}

/// Percent of physical memory in use system-wide, if known.
pub fn memory_load_percent() -> Option<u32> {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        GlobalMemoryStatusEx(&mut m).ok()?;
        Some(m.dwMemoryLoad)
    }
    #[cfg(not(windows))]
    None
}

/// With system memory `load`% in use: sleep up to `.1` background tabs idle
/// at least `.0` ms right now, or `None` while memory is plentiful. Every
/// awake background tab holds its renderer plus GPU memory, and the GPU
/// process only gives memory back once the pages using it are closed.
pub fn pressure_sleep_plan(load: u32) -> Option<(i64, usize)> {
    match load {
        90.. => Some((60_000, 3)),
        80..=89 => Some((5 * 60_000, 1)),
        _ => None,
    }
}

/// Turns omnibox input into a URL: keeps real URLs, adds https:// to
/// host-like input, searches everything else. `javascript:` is refused.
pub fn normalize_input(input: &str, search_prefix: &str) -> Option<String> {
    let t = input.trim();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_lowercase();
    if lower.starts_with("javascript:") || lower.starts_with("vbscript:") {
        return None;
    }
    if lower.starts_with("serica://") || lower.starts_with("neutrino://") || lower == "about:blank" {
        return Some(lower);
    }
    for scheme in ["http://", "https://", "chrome-extension://", "file://", "about:", "data:", "view-source:", "edge://", "mailto:"] {
        if lower.starts_with(scheme) {
            return Some(t.to_string());
        }
    }
    // A local path typed or pasted: drive paths, quoted paths, UNC shares.
    let unquoted = t.trim_matches('"');
    let b = unquoted.as_bytes();
    let drive = b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
    if drive || unquoted.starts_with("\\\\") {
        if let Ok(u) = url::Url::from_file_path(unquoted) {
            return Some(u.to_string());
        }
    }
    let host_part = t.split(['/', '?', '#']).next().unwrap_or("");
    let looks_like_host = !t.contains(char::is_whitespace)
        && (host_part.starts_with("localhost")
            || host_part.parse::<std::net::IpAddr>().is_ok()
            || host_part.split(':').next().unwrap_or("").parse::<std::net::Ipv4Addr>().is_ok()
            || (host_part.contains('.')
                && !host_part.starts_with('.')
                && !host_part.ends_with('.')
                && host_part.rsplit('.').next().map(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())).unwrap_or(false)));
    if looks_like_host {
        let bare_host = host_part.split(':').next().unwrap_or("");
        let scheme = if bare_host == "localhost" || bare_host.parse::<std::net::IpAddr>().is_ok() { "http" } else { "https" };
        return Some(format!("{scheme}://{t}"));
    }
    let q: String = url::form_urlencoded::byte_serialize(t.as_bytes()).collect();
    Some(format!("{search_prefix}{q}"))
}

impl App {
    pub fn new(handle: AppHandle, data_dir: std::path::PathBuf) -> Shared {
        let store = Store::load(data_dir.clone());
        let history = History::load(data_dir.join("history.json"));
        let shields = Shields::new(data_dir.join("adblock"), store.shield_stats.clone());
        shields.configure(store.settings.adblock_enabled, &store.settings.shield_whitelist);
        let vpn = Vpn::new(data_dir.join("vpn.json"));
        let net = (vpn.needs_proxy(), vpn.webrtc_guard_active());
        let port = vpn.start_proxy();
        let (tx, rx) = mpsc::channel();
        let pdf_dark = std::sync::atomic::AtomicBool::new(store.settings.pdf_dark_mode);
        let block_3pc = store.settings.block_third_party_cookies;
        let app = Arc::new(App {
            handle,
            store: Mutex::new(store),
            history: Mutex::new(history),
            shields,
            vpn,
            extensions: Extensions::new(data_dir.join("extensions")),
            downloads: Downloads::new(data_dir.join("downloads.json")),
            mcp_http: crate::mcp_http::McpHttp::new(data_dir.join("mcp-token.txt")),
            host: Mutex::new(TabHost::new(data_dir.join("webdata"), port, net, block_3pc)),
            mcp_logs: Mutex::new(VecDeque::new()),
            blocked_queue: Mutex::new(Vec::new()),
            tx: Mutex::new(tx),
            pdf_dark,
            cookie_swept: Mutex::new(Default::default()),
            save_lock: Mutex::new(()),
            written: Mutex::new(Default::default()),
        });
        let worker = app.clone();
        std::thread::Builder::new()
            .name("serica-sync".into())
            .spawn(move || worker.sync_loop(rx))
            .expect("sync thread");
        app.start_background_jobs();
        app.shields.spawn_updater(false);
        app
    }

    pub fn notify(&self, n: Notice) {
        let _ = self.tx.lock().send(n);
    }

    /// Coalesces notices (~60 fps), then broadcasts, reconciles webviews and
    /// saves with a short debounce.
    fn sync_loop(self: Arc<Self>, rx: mpsc::Receiver<Notice>) {
        let mut dirty_since: Option<Instant> = None;
        // Blocked-request counters reach the UI at most this often. Ad-heavy
        // pages block hundreds of requests while loading; each used to cost a
        // full state broadcast, a webview reconcile and a save to disk.
        const BLOCKED_REFRESH: Duration = Duration::from_millis(1000);
        let mut blocked_pending = false;
        let mut last_state_emit = Instant::now() - BLOCKED_REFRESH;
        loop {
            let first = match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(n) => Some(n),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            let (mut state, mut settings, mut history, mut quit) = (false, false, false, false);
            let mut take = |n: Notice| match n {
                Notice::State => state = true,
                Notice::Settings => settings = true,
                Notice::History => history = true,
                Notice::Blocked => blocked_pending = true,
                Notice::Quit => quit = true,
            };
            if let Some(n) = first {
                take(n);
                std::thread::sleep(Duration::from_millis(16));
                while let Ok(n) = rx.try_recv() {
                    take(n);
                }
            }
            if settings {
                let s = self.store.lock().settings.clone();
                self.shields.configure(s.adblock_enabled, &s.shield_whitelist);
                if self.pdf_dark.swap(s.pdf_dark_mode, std::sync::atomic::Ordering::Relaxed) != s.pdf_dark_mode {
                    TabHost::apply_pdf_dark(&self, s.pdf_dark_mode);
                }
                TabHost::sync_cookie_policy(&self, s.block_third_party_cookies);
                ui_emit(&self.handle, "serica:settings-updated", s);
                crate::mcp_http::sync(&self);
            }
            if state || settings {
                let snap = self.snapshot();
                ui_emit(&self.handle, "serica:state-update", snap);
                last_state_emit = Instant::now();
                blocked_pending = false;
                TabHost::reconcile(&self);
            } else if blocked_pending && last_state_emit.elapsed() >= BLOCKED_REFRESH {
                // Per-tab blocked counts and totals live in the snapshot.
                ui_emit(&self.handle, "serica:state-update", self.snapshot());
                last_state_emit = Instant::now();
                blocked_pending = false;
            }
            let blocked: Vec<BlockedTrackerItem> = std::mem::take(&mut *self.blocked_queue.lock());
            if !blocked.is_empty() {
                // One message per cycle (each emit is a script call on the UI thread).
                let latest: Vec<BlockedTrackerItem> = blocked.into_iter().rev().take(30).rev().collect();
                ui_emit(&self.handle, "serica:trackers-blocked", latest);
            }
            if history {
                // A ping, not the list: only an open History panel needs the
                // entries, and it fetches them (keeps page loads light).
                ui_emit(&self.handle, "serica:history-updated", serde_json::Value::Null);
            }
            if state || settings || history {
                dirty_since.get_or_insert_with(Instant::now);
            }
            if quit || dirty_since.map(|t| t.elapsed() > Duration::from_millis(400)).unwrap_or(false) {
                self.save_now();
                dirty_since = None;
            }
            if quit {
                break;
            }
        }
    }

    /// Saves state, settings and history. Only serializing happens under the
    /// locks: the page event handlers on the UI thread take the same locks, so
    /// holding them through disk writes (and antivirus scans of those writes)
    /// stalled page events and input. Unchanged files are not rewritten.
    pub fn save_now(&self) {
        let stats = self.shields.stats();
        let files = {
            let mut s = self.store.lock();
            s.shield_stats = stats;
            s.serialized()
        };
        let history = self.history.lock().take_serialized();
        let _guard = self.save_lock.lock();
        crate::store::write_files(&self.written, files, true);
        if let Some((path, bytes)) = history {
            crate::store::write_files(&self.written, vec![(path, bytes)], false);
        }
    }

    pub fn snapshot(&self) -> BrowserAppState {
        let mut snap = self.store.lock().snapshot();
        snap.shield_stats = self.shields.stats();
        for t in &mut snap.tabs {
            t.blocked_count = self.shields.tab_count(&t.id);
        }
        snap
    }

    pub fn settings(&self) -> BrowserSettings {
        self.store.lock().settings.clone()
    }

    /// True the first time it is asked for `profile` this session.
    pub fn claim_cookie_sweep(&self, profile: &str) -> bool {
        self.cookie_swept.lock().insert(profile.to_string())
    }

    /// Mutates the store and schedules a broadcast. Keep `f` free of webview calls.
    pub fn mutate<T>(&self, f: impl FnOnce(&mut Store) -> T) -> T {
        let out = f(&mut self.store.lock());
        self.notify(Notice::State);
        out
    }

    fn start_background_jobs(self: &Arc<Self>) {
        crate::mcp_http::sync(self);
        // Zen auto-sleep and Today-tab archiving, twice a minute.
        let app = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(30));
            loop {
                tick.tick().await;
                let pressure = memory_load_percent().and_then(pressure_sleep_plan);
                let (slept, archived) = {
                    let mut s = app.store.lock();
                    let mut slept = 0;
                    if s.settings.zen_sleep_enabled {
                        let idle_ms = i64::from(s.settings.tab_sleep_minutes) * 60_000;
                        slept += s.sleep_inactive(idle_ms);
                        // Low on RAM: also sleep the least recently used
                        // background tabs, a few per tick, before the timer.
                        if let Some((min_idle_ms, limit)) = pressure {
                            slept += s.sleep_least_recent(min_idle_ms, limit);
                        }
                    }
                    (slept, s.archive_sweep())
                };
                if slept + archived > 0 {
                    app.notify(Notice::State);
                }
                if archived > 0 {
                    let list = app.store.lock().archive_list();
                    ui_emit(&app.handle, "serica:archive-updated", list);
                }
            }
        });
        // Refresh adblock lists every 6 hours (they refresh when older than 4 days).
        let app = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(6 * 3600));
            tick.tick().await;
            loop {
                tick.tick().await;
                app.shields.spawn_updater(false);
            }
        });
    }

    // ------------------------------------------------------------- tab events

    pub fn tab_event(&self, tab_id: &str, ev: TabEvent) {
        let mut history_changed = false;
        let mut settings_changed = false;
        // Zoom to apply to the page after the locks are released (a new site).
        let mut apply_zoom: Option<f64> = None;
        {
            let mut s = self.store.lock();
            let space = s.tab(tab_id).map(|t| t.space_id.clone());
            let Some(tab) = s.tab_mut(tab_id) else { return };
            match ev {
                TabEvent::NavigationStarting(url) => {
                    tab.is_loading = true;
                    if !url.is_empty() && !url.starts_with("about:") {
                        let host = |u: &str| url::Url::parse(u).ok().and_then(|u| u.host_str().map(str::to_string));
                        if host(&url) != host(&tab.url) {
                            // The old site's icon must not stick to a new site.
                            tab.favicon = None;
                        }
                        tab.url = url;
                    }
                    self.shields.reset_tab(tab_id);
                }
                TabEvent::UrlChanged(url) => {
                    if !url.is_empty() && url != "about:blank" {
                        tab.url = url;
                        // Zoom is per site: a page on another site takes that
                        // site's zoom (or the default).
                        if crate::settings::zoom_site(&tab.url).is_some() {
                            let want = s.settings.zoom_for(&s.tab(tab_id).map(|t| t.url.clone()).unwrap_or_default());
                            if let Some(tab) = s.tab_mut(tab_id) {
                                if tab.zoom_factor.map(|z| (z - want).abs() >= 0.005).unwrap_or(true) {
                                    tab.zoom_factor = Some(want);
                                    apply_zoom = Some(want);
                                }
                            }
                        }
                    }
                }
                TabEvent::History { back, forward } => {
                    tab.can_go_back = back;
                    tab.can_go_forward = forward;
                }
                TabEvent::Loaded { url, success } => {
                    tab.is_loading = false;
                    if success && !url.is_empty() && !url.starts_with("about:") {
                        tab.url = url.clone();
                        let title = tab.title.clone();
                        drop(s);
                        self.history.lock().record_visit(&url, if title == url { "" } else { &title }, space.as_deref());
                        history_changed = true;
                        s = self.store.lock();
                    }
                }
                TabEvent::Title(title) => {
                    if !title.trim().is_empty() {
                        tab.title = title.chars().take(300).collect();
                        let (url, t) = (tab.url.clone(), tab.title.clone());
                        drop(s);
                        self.history.lock().update_meta(&url, Some(&t), None);
                        history_changed = true;
                        s = self.store.lock();
                    }
                }
                TabEvent::Favicon(uri) => {
                    if uri.starts_with("http") || uri.starts_with("data:") {
                        tab.favicon = Some(uri.clone());
                        let url = tab.url.clone();
                        drop(s);
                        self.history.lock().update_meta(&url, None, Some(&uri));
                        s = self.store.lock();
                    }
                }
                TabEvent::Audio(playing) => {
                    tab.has_active_media = playing;
                }
                TabEvent::Zoom(z) => {
                    let z = (z * 100.0).round() / 100.0;
                    tab.zoom_factor = Some(z);
                    let url = tab.url.clone();
                    ui_emit(&self.handle, "serica:zoom-updated", serde_json::json!({ "tabId": tab_id, "zoomFactor": z }));
                    settings_changed = s.settings.remember_zoom(&url, z);
                }
            }
            drop(s);
        }
        if let Some(z) = apply_zoom {
            let wv = self.host.lock().views.get(tab_id).map(|v| v.webview.clone());
            #[cfg(windows)]
            if let Some(wv) = wv {
                crate::webview_win::set_zoom(&wv, z);
            }
            #[cfg(not(windows))]
            let _ = (wv, z);
        }
        if settings_changed {
            self.notify(Notice::Settings);
        }
        self.notify(Notice::State);
        if history_changed {
            self.notify(Notice::History);
        }
    }

    pub fn on_blocked(&self, item: BlockedTrackerItem) {
        let mut q = self.blocked_queue.lock();
        q.push(item);
        if q.len() > 200 {
            q.drain(0..100);
        }
        drop(q);
        self.notify(Notice::Blocked);
    }

    /// Handles a browser shortcut pressed inside a web page.
    pub fn shortcut(self: &Arc<Self>, sc: Shortcut) {
        let h = &self.handle;
        match sc {
            Shortcut::NewTab => ui_emit(h, "serica:open-command-bar", serde_json::json!({ "newTab": true })),
            Shortcut::OpenFile => crate::files::pick_and_open(self),
            Shortcut::OpenCommandBar => ui_emit(h, "serica:open-command-bar", serde_json::json!({})),
            Shortcut::ToggleSidebar => ui_emit(h, "serica:toggle-tabs", ()),
            Shortcut::Find => ui_emit(h, "serica:site-action", serde_json::json!({ "action": "open-find", "source": "shortcut" })),
            Shortcut::Home => ui_emit(h, "serica:site-action", serde_json::json!({ "action": "home", "source": "shortcut" })),
            Shortcut::History => ui_emit(h, "serica:open-history", ()),
            Shortcut::Downloads => ui_emit(h, "serica:open-downloads", ()),
            Shortcut::Settings => ui_emit(h, "serica:open-settings", ()),
            Shortcut::Escape => ui_emit(h, "serica:escape-pressed", ()),
            Shortcut::CloseTab => {
                self.mutate(|s| {
                    if let Some(id) = s.active_tab_id.clone() {
                        s.close_tab(&id);
                    }
                });
            }
            Shortcut::ReopenTab => {
                self.mutate(|s| s.reopen_closed_tab());
            }
            Shortcut::CycleTab(d) => {
                self.mutate(|s| s.cycle_tab(d));
            }
            Shortcut::SpaceIndex(n) => {
                let switched = self.mutate(|s| {
                    let order = s.switcher_order();
                    let target = order.get(n.wrapping_sub(1)).cloned()?;
                    if target == s.active_space_id {
                        return None;
                    }
                    s.activate_space(&target);
                    s.space(&target).map(|sp| (sp.id.clone(), sp.name.clone()))
                });
                if let Some((id, name)) = switched {
                    ui_emit(h, "serica:space-switched", serde_json::json!({ "spaceId": id, "spaceName": name }));
                }
            }
        }
    }

    pub fn log_mcp(&self, entry: Value) {
        let mut l = self.mcp_logs.lock();
        l.push_back(entry);
        while l.len() > 100 {
            l.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{memory_load_percent, normalize_input, pressure_sleep_plan};
    const G: &str = "https://www.google.com/search?q=";

    #[test]
    fn memory_pressure_plan() {
        assert_eq!(pressure_sleep_plan(50), None);
        assert_eq!(pressure_sleep_plan(79), None);
        assert_eq!(pressure_sleep_plan(80), Some((5 * 60_000, 1)));
        assert_eq!(pressure_sleep_plan(95), Some((60_000, 3)));
    }

    #[cfg(windows)]
    #[test]
    fn memory_load_is_read() {
        assert!(memory_load_percent().is_some_and(|p| p <= 100));
    }

    #[test]
    fn omnibox_input() {
        assert_eq!(normalize_input("example.com", G).unwrap(), "https://example.com");
        assert_eq!(normalize_input("localhost:3000/x", G).unwrap(), "http://localhost:3000/x");
        assert_eq!(normalize_input("https://a.b/c", G).unwrap(), "https://a.b/c");
        assert_eq!(normalize_input(r"C:\Docs\My File.pdf", G).unwrap(), "file:///C:/Docs/My%20File.pdf");
        assert_eq!(normalize_input(r#""D:\a\b.pdf""#, G).unwrap(), "file:///D:/a/b.pdf");
        assert_eq!(normalize_input(r"\\server\share\x.pdf", G).unwrap(), "file://server/share/x.pdf");
        assert_eq!(normalize_input("rust lang", G).unwrap(), format!("{G}rust+lang"));
        assert_eq!(normalize_input("what is 3.5", G).unwrap(), format!("{G}what+is+3.5"));
        assert_eq!(normalize_input("node.js", G).unwrap(), "https://node.js");
        assert_eq!(normalize_input("v1.2", G).unwrap(), format!("{G}v1.2"));
        assert!(normalize_input("javascript:alert(1)", G).is_none());
        assert_eq!(normalize_input("192.168.1.1", G).unwrap(), "http://192.168.1.1");
        assert_eq!(normalize_input("SERICA://settings", G).unwrap(), "serica://settings");
    }
}
