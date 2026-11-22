//! Native tab webviews.
//!
//! Each live tab is a WebView2 child webview of the main window, positioned
//! over the UI's viewport rectangle (reported by the UI in CSS pixels).
//! `reconcile` diffs the store against the existing webviews: it creates the
//! active tab's webview lazily, closes webviews of slept/closed/new-tab tabs,
//! and shows exactly one of them.
//!
//! Native webviews always paint above the HTML UI, so the UI "freezes" the
//! viewport while a modal or popover covers it: the active page is captured
//! to an image, shown by the UI, and the native webview is hidden.

use crate::app::Shared;
#[cfg(not(windows))]
use crate::app::TabEvent;
use crate::model::is_internal_url;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::webview::{NewWindowResponse, PageLoadEvent};
use tauri::{LogicalPosition, LogicalSize, Manager, Webview, WebviewBuilder, WebviewUrl};

static LABEL_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

pub struct TabView {
    pub webview: Webview,
    pub profile: String,
    pub shown: bool,
}

/// Puts the largest playing video into native picture-in-picture.
pub const PIP_ENTER_JS: &str = r#"(() => {
  if (document.pictureInPictureElement || !document.pictureInPictureEnabled) return false;
  const vs = Array.from(document.querySelectorAll('video')).filter((v) => !v.paused && !v.ended && v.readyState >= 2 && !v.disablePictureInPicture);
  if (!vs.length) return false;
  vs.sort((a, b) => b.clientWidth * b.clientHeight - a.clientWidth * a.clientHeight);
  const v = vs[0];
  if (v.clientWidth * v.clientHeight < 200 * 112) return false;
  v.requestPictureInPicture().catch(() => {});
  return true;
})()"#;

/// Brings a picture-in-picture video back into its page. Runs right as the tab
/// is shown, so it waits until the document is actually visible.
pub const PIP_EXIT_JS: &str = r#"(() => {
  const exit = () => { if (document.pictureInPictureElement) document.exitPictureInPicture().catch(() => {}); };
  if (document.visibilityState === 'visible') { exit(); return; }
  const onVisible = () => {
    if (document.visibilityState !== 'visible') return;
    document.removeEventListener('visibilitychange', onVisible);
    exit();
  };
  document.addEventListener('visibilitychange', onVisible);
})()"#;

pub struct TabHost {
    pub views: HashMap<String, TabView>,
    pub bounds: Option<Bounds>,
    /// False while a UI overlay covers the viewport.
    pub visible: bool,
    pub active: Option<String>,
    pub previews: HashMap<String, String>,
    pub assistant: Option<(Webview, Bounds)>,
    data_root: PathBuf,
    proxy_port: Option<u16>,
}

/// Runs in every tab's top frame: reports CSS classes/ids for Brave's generic
/// cosmetic filtering. Messages are ignored by Tauri's IPC (not invoke calls).
pub const PAGE_SCRIPT: &str = r#"(() => {
  if (window.top !== window || window.__sericaPage) return;
  Object.defineProperty(window, '__sericaPage', { value: true });
  const post = (m) => { try { window.chrome.webview.postMessage(JSON.stringify(Object.assign({ __serica: 1 }, m))); } catch (e) {} };
  const seenC = new Set(), seenI = new Set();
  let pendC = [], pendI = [], timer = 0;
  const flush = () => { timer = 0; if (pendC.length || pendI.length) { post({ t: 'cosmetic', classes: pendC, ids: pendI }); pendC = []; pendI = []; } };
  const visit = (el) => {
    if (seenC.size > 6000) return;
    const cl = el.classList;
    if (cl) for (const c of cl) if (!seenC.has(c)) { seenC.add(c); pendC.push(c); }
    const id = el.id;
    if (id && typeof id === 'string' && !seenI.has(id)) { seenI.add(id); pendI.push(id); }
  };
  const scan = (root) => {
    if (root.nodeType !== 1) return;
    visit(root);
    const els = root.querySelectorAll('[class],[id]');
    for (let i = 0; i < els.length && i < 4000; i++) visit(els[i]);
    if (!timer && (pendC.length || pendI.length)) timer = setTimeout(flush, 120);
  };
  const start = () => {
    scan(document.documentElement);
    new MutationObserver((muts) => { for (const m of muts) for (const n of m.addedNodes) scan(n); })
      .observe(document.documentElement, { childList: true, subtree: true });
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start, { once: true });
  else start();
})();"#;

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

impl TabHost {
    pub fn new(data_root: PathBuf, proxy_port: Option<u16>) -> Self {
        Self {
            views: HashMap::new(),
            bounds: None,
            visible: true,
            active: None,
            previews: HashMap::new(),
            assistant: None,
            data_root,
            proxy_port,
        }
    }

    pub fn profile_dir(&self, profile: &str) -> PathBuf {
        self.data_root.join(sanitize(profile))
    }

    fn browser_args(&self) -> String {
        let mut args = String::from("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection");
        if let Some(p) = self.proxy_port {
            args.push_str(&format!(" --proxy-server=http://127.0.0.1:{p}"));
        }
        args
    }

    fn builder(&self, app: &Shared, label: String, url: url::Url, profile: &str, tab_id: Option<&str>) -> WebviewBuilder<tauri::Wry> {
        let mut b = WebviewBuilder::new(label, WebviewUrl::External(url))
            .data_directory(self.profile_dir(profile))
            .additional_browser_args(&self.browser_args())
            .background_color(tauri::webview::Color(255, 255, 255, 255))
            .browser_extensions_enabled(true)
            .initialization_script(PAGE_SCRIPT)
            .initialization_script(crate::extensions::WEBSTORE_SCRIPT);
        if app.vpn.webrtc_guard_active() {
            b = b.initialization_script_for_all_frames(crate::vpn::WEBRTC_GUARD_JS);
        }
        if let Some(tab_id) = tab_id {
            let (a, id) = (app.clone(), tab_id.to_string());
            b = b.on_new_window(move |url, features| {
                // Sized popups (OAuth, payment) keep a real popup so window.opener works.
                if features.size().is_some() {
                    return NewWindowResponse::Allow;
                }
                let target = url.to_string();
                let id2 = id.clone();
                a.mutate(move |s| {
                    let space = s.tab(&id2).map(|t| t.space_id.clone());
                    s.create_tab(Some(target), space, true, Some(&id2));
                });
                NewWindowResponse::Deny
            });
            // Portable fallbacks; on Windows the native handlers in
            // `webview_win` report the same and more.
            #[cfg(not(windows))]
            {
                let (a, id) = (app.clone(), tab_id.to_string());
                b = b.on_document_title_changed(move |_, title| a.tab_event(&id, TabEvent::Title(title)));
                let (a, id) = (app.clone(), tab_id.to_string());
                b = b.on_page_load(move |_, p| match p.event() {
                    PageLoadEvent::Started => a.tab_event(&id, TabEvent::NavigationStarting(p.url().to_string())),
                    PageLoadEvent::Finished => a.tab_event(&id, TabEvent::Loaded { url: p.url().to_string(), success: true }),
                });
            }
            #[cfg(windows)]
            {
                let _ = PageLoadEvent::Started;
            }
        }
        b
    }

    /// Creates a tab webview at the current viewport bounds.
    fn create(&mut self, app: &Shared, tab_id: &str, url: &str, profile: &str) -> Option<Webview> {
        let bounds = self.bounds?;
        let window = app.handle.get_window("main")?;
        let parsed = url::Url::parse(url).ok()?;
        let label = format!("tab-{}-{}", LABEL_SEQ.fetch_add(1, Ordering::Relaxed), sanitize(tab_id));
        let builder = self.builder(app, label, parsed, profile, Some(tab_id));
        match window.add_child(builder, LogicalPosition::new(bounds.x, bounds.y), LogicalSize::new(bounds.w.max(1.0), bounds.h.max(1.0))) {
            Ok(wv) => {
                #[cfg(windows)]
                crate::webview_win::attach(app, tab_id, profile, &wv);
                Some(wv)
            }
            Err(e) => {
                tracing::error!("failed to create webview for {tab_id}: {e}");
                None
            }
        }
    }

    fn apply_bounds(wv: &Webview, b: Bounds) {
        let _ = wv.set_bounds(tauri::Rect {
            position: LogicalPosition::new(b.x, b.y).into(),
            size: LogicalSize::new(b.w.max(1.0), b.h.max(1.0)).into(),
        });
    }

    /// Brings native webviews in line with the store.
    pub fn reconcile(app: &Shared) {
        struct Want {
            url: String,
            live: bool,
            profile: String,
            media: bool,
        }
        let (wants, active, auto_pip): (HashMap<String, Want>, Option<String>, bool) = {
            let s = app.store.lock();
            let wants = s
                .tabs
                .iter()
                .map(|t| (t.id.clone(), Want { url: t.url.clone(), live: t.wants_webview(), profile: s.space_profile(&t.space_id), media: t.has_active_media }))
                .collect();
            (wants, s.active_tab_id.clone(), s.settings.auto_picture_in_picture)
        };
        let mut host = app.host.lock();

        // Close webviews whose tab is gone, asleep, unloaded, on an internal
        // page, or moved to another profile (which needs another data dir).
        let stale: Vec<String> = host
            .views
            .iter()
            .filter(|(id, v)| wants.get(*id).map(|w| !w.live || w.profile != v.profile).unwrap_or(true))
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            if let Some(v) = host.views.remove(&id) {
                let _ = v.webview.close();
            }
            if !wants.contains_key(&id) {
                host.previews.remove(&id);
            }
        }

        // Lazily create the active tab's webview.
        if let Some(id) = &active {
            if let Some(w) = wants.get(id) {
                if w.live && !host.views.contains_key(id) && !is_internal_url(&w.url) {
                    let (url, profile) = (w.url.clone(), w.profile.clone());
                    if let Some(wv) = host.create(app, id, &url, &profile) {
                        let _ = wv.hide();
                        host.views.insert(id.clone(), TabView { webview: wv, profile, shown: false });
                    }
                }
            }
        }

        let previous = host.active.clone();
        host.active = active.clone();
        let visible = host.visible;
        let bounds = host.bounds;
        let mut previews_to_capture = Vec::new();
        for (id, v) in host.views.iter_mut() {
            let should_show = visible && Some(id) == active.as_ref();
            if should_show {
                if let Some(b) = bounds {
                    Self::apply_bounds(&v.webview, b);
                }
                if !v.shown {
                    let _ = v.webview.show();
                    v.shown = true;
                    // Returning to a tab brings its mini-player video back into the page.
                    let _ = v.webview.eval(PIP_EXIT_JS);
                }
            } else if v.shown {
                // Snapshot the page we're leaving for hover previews.
                if previous.as_ref() == Some(id) {
                    previews_to_capture.push((id.clone(), v.webview.clone()));
                    // Leaving a playing video: keep watching it in a mini player.
                    if auto_pip && wants.get(id).map(|w| w.media).unwrap_or(false) {
                        let _ = v.webview.eval(PIP_ENTER_JS);
                    }
                }
                let _ = v.webview.hide();
                v.shown = false;
            }
        }
        drop(host);
        #[cfg(windows)]
        for (id, wv) in previews_to_capture {
            let app = app.clone();
            crate::webview_win::capture(&wv, move |img| {
                if let Some(img) = img {
                    app.host.lock().previews.insert(id, img);
                }
            });
        }
        #[cfg(not(windows))]
        let _ = previews_to_capture;
    }

    /// Navigates a tab's existing webview. Returns false if it has none yet
    /// (`reconcile` will create it with the tab's URL).
    pub fn navigate(app: &Shared, tab_id: &str, url: &str) -> bool {
        let host = app.host.lock();
        let Some(v) = host.views.get(tab_id) else { return false };
        match url::Url::parse(url) {
            Ok(u) => v.webview.navigate(u).is_ok(),
            Err(_) => false,
        }
    }

    pub fn webview(app: &Shared, tab_id: &str) -> Option<Webview> {
        app.host.lock().views.get(tab_id).map(|v| v.webview.clone())
    }

    pub fn active_webview(app: &Shared) -> Option<Webview> {
        let id = app.store.lock().active_tab_id.clone()?;
        Self::webview(app, &id)
    }

    pub fn set_bounds(app: &Shared, b: Bounds) {
        let mut host = app.host.lock();
        host.bounds = Some(b);
        let first = host.views.is_empty();
        // Always apply (the UI already dedupes): this also repairs a native
        // view that ended up somewhere else, e.g. right after creation.
        for v in host.views.values().filter(|v| v.shown) {
            Self::apply_bounds(&v.webview, b);
        }
        drop(host);
        if first {
            Self::reconcile(app);
        }
    }

    pub fn set_visible(app: &Shared, visible: bool) {
        app.host.lock().visible = visible;
        Self::reconcile(app);
        if !visible {
            // Give keyboard focus back to the UI (command bar, dialogs).
            if let Some(main) = app.handle.get_webview("main") {
                let _ = main.set_focus();
            }
        }
    }

    pub fn close_all(app: &Shared) {
        let mut host = app.host.lock();
        for (_, v) in host.views.drain() {
            let _ = v.webview.close();
        }
        if let Some((wv, _)) = host.assistant.take() {
            let _ = wv.close();
        }
    }

    // ------------------------------------------------------------- assistant

    pub fn assistant_open(app: &Shared, url: &str, b: Bounds) -> Result<(), String> {
        let parsed = url::Url::parse(url).map_err(|e| e.to_string())?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err("unsupported url".into());
        }
        let mut host = app.host.lock();
        if let Some((wv, _)) = &host.assistant {
            let _ = wv.navigate(parsed);
            Self::apply_bounds(wv, b);
            let _ = wv.show();
            host.assistant = Some((wv.clone(), b));
            return Ok(());
        }
        let window = app.handle.get_window("main").ok_or("no window")?;
        let profile = app.store.lock().active_profile_id();
        let label = format!("assistant-{}", LABEL_SEQ.fetch_add(1, Ordering::Relaxed));
        let builder = host.builder(app, label, parsed, &profile, None);
        let wv = window
            .add_child(builder, LogicalPosition::new(b.x, b.y), LogicalSize::new(b.w.max(1.0), b.h.max(1.0)))
            .map_err(|e| e.to_string())?;
        host.assistant = Some((wv, b));
        Ok(())
    }

    pub fn assistant_bounds(app: &Shared, b: Bounds) {
        let mut host = app.host.lock();
        if let Some((wv, bounds)) = host.assistant.as_mut() {
            *bounds = b;
            Self::apply_bounds(wv, b);
        }
    }

    pub fn assistant_close(app: &Shared) {
        if let Some((wv, _)) = app.host.lock().assistant.take() {
            let _ = wv.close();
        }
    }

    pub fn assistant_webview(app: &Shared) -> Option<Webview> {
        app.host.lock().assistant.as_ref().map(|(w, _)| w.clone())
    }
}
