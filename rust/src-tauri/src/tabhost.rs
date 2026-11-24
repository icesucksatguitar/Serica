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
    /// When the view was last hidden (drives background suspension).
    pub hidden_at: Option<std::time::Instant>,
}

/// Hidden tabs drop to WebView2's low-memory level after this long, and are
/// frozen after `SUSPEND_AFTER`, unless they play media.
const TRIM_AFTER: std::time::Duration = std::time::Duration::from_secs(30);
const SUSPEND_AFTER: std::time::Duration = std::time::Duration::from_secs(5 * 60);

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

/// A rectangle of UI (CSS px, window coordinates) that shows through the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hole {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Corner radius (CSS px), so rounded UI is cut out exactly.
    pub r: f64,
}

impl Hole {
    fn lerp(a: Hole, b: Hole, p: f64) -> Hole {
        let l = |x: f64, y: f64| x + (y - x) * p;
        Hole { x: l(a.x, b.x), y: l(a.y, b.y), w: l(a.w, b.w), h: l(a.h, b.h), r: l(a.r, b.r) }
    }
}

/// CSS `cubic-bezier(x1, y1, x2, y2)` at progress `x` (0..1).
pub fn cubic_bezier(e: [f64; 4], x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    let [x1, y1, x2, y2] = e;
    let bez = |t: f64, a: f64, b: f64| 3.0 * (1.0 - t) * (1.0 - t) * t * a + 3.0 * (1.0 - t) * t * t * b + t * t * t;
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    let mut t = x;
    for _ in 0..30 {
        let v = bez(t, x1, x2);
        if (v - x).abs() < 1e-5 {
            break;
        }
        if v < x {
            lo = t;
        } else {
            hi = t;
        }
        t = (lo + hi) / 2.0;
    }
    bez(t, y1, y2)
}

pub struct TabHost {
    pub views: HashMap<String, TabView>,
    /// UI regions cut out of the page (sidebar, dock, menus), by owner id.
    pub holes: HashMap<String, Hole>,
    hole_gen: HashMap<String, u64>,
    pub bounds: Option<Bounds>,
    /// False while a UI overlay covers the viewport.
    pub visible: bool,
    pub active: Option<String>,
    pub previews: HashMap<String, String>,
    pub assistant: Option<(Webview, Bounds)>,
    data_root: PathBuf,
    proxy_port: Option<u16>,
    /// Network setup the current page webviews were created with:
    /// (routed through the local tunnel proxy, WebRTC guard script).
    net: (bool, bool),
    /// Failed creations in a row (a profile's browser process may still be
    /// shutting down after a network change); creation is retried a few times.
    create_failures: u32,
    /// Tabs whose webview is being built right now (outside the lock).
    creating: std::collections::HashSet<String>,
}

/// Two-finger horizontal touchpad swipe over a page goes back/forward, when
/// nothing under the pointer can scroll that way. An edge arrow follows the
/// swipe; crossing the threshold navigates (handled natively: `swipe-nav`).
pub const SWIPE_SCRIPT: &str = r#"(() => {
  if (window.top !== window || window.__sericaSwipe) return;
  Object.defineProperty(window, '__sericaSwipe', { value: 1 });
  const THRESHOLD = 160, GAP = 220;
  let acc = 0, last = 0, state = 'idle', dir = 0, ui = null, arrow = null, hideTimer = 0;
  const scrollsX = (n, dx) => {
    for (; n && n.nodeType === 1; n = n.parentElement || (n.getRootNode && n.getRootNode().host) || null) {
      if (n.scrollWidth <= n.clientWidth + 1) continue;
      const ox = getComputedStyle(n).overflowX;
      const root = n === document.scrollingElement || n === document.documentElement || n === document.body;
      if (root ? ox === 'hidden' || ox === 'clip' : ox !== 'auto' && ox !== 'scroll') continue;
      const left = Math.abs(n.scrollLeft), max = n.scrollWidth - n.clientWidth;
      if (dx < 0 ? left > 1 : left < max - 1) return true;
    }
    return false;
  };
  const show = (p) => {
    if (!ui) {
      ui = document.createElement('serica-swipe');
      ui.style.cssText = 'position:fixed;top:50%;z-index:2147483647;pointer-events:none;width:44px;height:44px;margin-top:-22px;border-radius:50%;display:flex;align-items:center;justify-content:center;font:600 22px/1 "Segoe UI",system-ui,sans-serif;box-shadow:0 4px 16px rgba(0,0,0,.25);transition:background .12s,color .12s';
      arrow = document.createElement('span');
      ui.appendChild(arrow);
    }
    if (!ui.isConnected) (document.body || document.documentElement).appendChild(ui);
    const done = p >= 1;
    arrow.textContent = dir < 0 ? '←' : '→';
    ui.style.left = dir < 0 ? '0' : '';
    ui.style.right = dir < 0 ? '' : '0';
    const off = -44 + Math.min(p, 1) * 64;
    ui.style.transform = 'translateX(' + (dir < 0 ? off : -off) + 'px)';
    ui.style.opacity = String(Math.min(1, p * 1.5));
    ui.style.background = done ? '#0b57d0' : 'rgba(60,64,67,.92)';
    ui.style.color = '#fff';
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => { if (ui) ui.remove(); }, done ? 260 : GAP + 40);
  };
  addEventListener('wheel', (e) => {
    const now = performance.now();
    if (now - last > GAP) { state = 'idle'; acc = 0; }
    last = now;
    if (state === 'off' || state === 'done') return;
    if (e.ctrlKey || e.shiftKey || e.deltaMode !== 0 || e.defaultPrevented) { state = 'off'; return; }
    if (state === 'idle') {
      if (Math.abs(e.deltaX) < 1 && Math.abs(e.deltaY) < 1) return;
      if (Math.abs(e.deltaX) <= Math.abs(e.deltaY) * 1.5 || scrollsX(e.target, e.deltaX)) { state = 'off'; return; }
      state = 'track';
      dir = e.deltaX < 0 ? -1 : 1;
    }
    acc += e.deltaX * dir;
    if (acc < -20) { state = 'off'; if (ui) ui.remove(); return; }
    const p = Math.max(0, acc) / THRESHOLD;
    show(p);
    if (p >= 1) {
      state = 'done';
      try { window.chrome.webview.postMessage(JSON.stringify({ __serica: 1, t: 'swipe-nav', dir: dir < 0 ? 'back' : 'forward' })); } catch (err) {}
    }
  }, { passive: true });
})();"#;

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

/// A tab webview ready to be built (see `TabHost::prepare`).
struct PendingView {
    window: tauri::Window,
    builder: WebviewBuilder<tauri::Wry>,
    bounds: Bounds,
    url: url::Url,
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

impl TabHost {
    pub fn new(data_root: PathBuf, proxy_port: Option<u16>, net: (bool, bool)) -> Self {
        Self {
            views: HashMap::new(),
            holes: HashMap::new(),
            hole_gen: HashMap::new(),
            bounds: None,
            visible: true,
            active: None,
            previews: HashMap::new(),
            assistant: None,
            data_root,
            proxy_port,
            net,
            create_failures: 0,
            creating: std::collections::HashSet::new(),
        }
    }

    pub fn profile_dir(&self, profile: &str) -> PathBuf {
        self.data_root.join(sanitize(profile))
    }

    fn browser_args(&self) -> String {
        // Background services Serica doesn't use: each costs memory, wakeups or network.
        let mut args = String::from(
            "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,Translate,OptimizationHints,MediaRouter,DialMediaRouteProvider,AutofillServerCommunication,InterestFeedContentSuggestions,SpareRendererForSitePerProcess --no-pings",
        );
        // Pages connect directly unless the VPN needs the tunnel: a proxy turns
        // off HTTP/3 and Chromium's own DNS and connection racing.
        if let (true, Some(p)) = (self.net.0, self.proxy_port) {
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
            .initialization_script(crate::extensions::WEBSTORE_SCRIPT)
            .initialization_script(SWIPE_SCRIPT);
        if self.net.1 {
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

    /// Everything needed to create a tab webview at the current viewport
    /// bounds. Cheap; done under the host lock.
    fn prepare(&self, app: &Shared, tab_id: &str, url: &str, profile: &str) -> Option<PendingView> {
        let bounds = self.bounds?;
        let window = app.handle.get_window("main")?;
        let parsed = url::Url::parse(url).ok()?;
        let label = format!("tab-{}-{}", LABEL_SEQ.fetch_add(1, Ordering::Relaxed), sanitize(tab_id));
        // On Windows the page starts blank so its adblock scripts can be
        // registered before the first load (see `webview_win::attach`).
        #[cfg(windows)]
        let start = url::Url::parse("about:blank").ok()?;
        #[cfg(not(windows))]
        let start = parsed.clone();
        let builder = self.builder(app, label, start, profile, Some(tab_id));
        Some(PendingView { window, builder, bounds, url: parsed })
    }

    /// Creates the webview. `add_child` blocks until the main thread has
    /// built it, and the main thread takes the host lock in its own callbacks
    /// (preview captures finishing, page events), so this must run without
    /// the host lock held, or both threads wait on each other forever.
    fn build(app: &Shared, tab_id: &str, profile: &str, p: PendingView) -> Option<Webview> {
        let PendingView { window, builder, bounds, url: parsed } = p;
        match window.add_child(builder, LogicalPosition::new(bounds.x, bounds.y), LogicalSize::new(bounds.w.max(1.0), bounds.h.max(1.0))) {
            Ok(wv) => {
                #[cfg(windows)]
                crate::webview_win::attach(app, tab_id, profile, &wv, Some(parsed.to_string()));
                #[cfg(not(windows))]
                let _ = (app, profile, parsed);
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
            zoom: f64,
        }
        let (wants, active, auto_pip, text_scale): (HashMap<String, Want>, Option<String>, bool, f64) = {
            let s = app.store.lock();
            let wants = s
                .tabs
                .iter()
                .map(|t| {
                    let want = Want {
                        url: t.url.clone(),
                        live: t.wants_webview(),
                        profile: s.space_profile(&t.space_id),
                        media: t.has_active_media,
                        zoom: t.zoom_factor.filter(|z| z.is_finite() && *z > 0.0).unwrap_or(s.settings.text_scale),
                    };
                    (t.id.clone(), want)
                })
                .collect();
            (wants, s.active_tab_id.clone(), s.settings.auto_picture_in_picture, s.settings.text_scale)
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

        let mut retry = false;
        // Lazily create the active tab's webview. The lock is released while
        // it is built, so another reconcile may be building it already.
        if let Some(id) = &active {
            if let Some(w) = wants.get(id) {
                if w.live && !host.views.contains_key(id) && !host.creating.contains(id) && !is_internal_url(&w.url) {
                    let profile = w.profile.clone();
                    let created = match host.prepare(app, id, &w.url, &profile) {
                        Some(p) => {
                            host.creating.insert(id.clone());
                            drop(host);
                            let wv = Self::build(app, id, &profile, p);
                            host = app.host.lock();
                            host.creating.remove(id);
                            wv
                        }
                        None => None,
                    };
                    if let Some(wv) = created {
                        host.create_failures = 0;
                        let _ = wv.hide();
                        #[cfg(windows)]
                        if (w.zoom - 1.0).abs() > 0.001 {
                            crate::webview_win::set_zoom(&wv, w.zoom);
                        }
                        if let Some(old) = host.views.insert(id.clone(), TabView { webview: wv, profile, shown: false, hidden_at: None }) {
                            let _ = old.webview.close();
                        }
                        // `wants`/`active` were read before the lock was
                        // released for the build: a reconcile that ran
                        // meanwhile (tab closed, slept or switched away) could
                        // not see this view. Start over from fresh state; the
                        // view now exists, so this does not build again.
                        drop(host);
                        return Self::reconcile(app);
                    } else if host.create_failures < 20 {
                        host.create_failures += 1;
                        retry = true;
                    }
                }
            }
        }

        let previous = host.active.clone();
        host.active = active.clone();
        #[cfg(windows)]
        let clip = Self::clip_rects(&host, text_scale);
        #[cfg(not(windows))]
        let _ = text_scale;
        let visible = host.visible;
        let bounds = host.bounds;
        let mut previews_to_capture = Vec::new();
        let mut to_suspend = Vec::new();
        for (id, v) in host.views.iter_mut() {
            let should_show = visible && Some(id) == active.as_ref();
            if should_show {
                if let Some(b) = bounds {
                    Self::apply_bounds(&v.webview, b);
                }
                if !v.shown {
                    #[cfg(windows)]
                    crate::webview_win::set_clip(&v.webview, clip.clone());
                    #[cfg(windows)]
                    crate::webview_win::set_background(&v.webview, false);
                    let _ = v.webview.show();
                    v.shown = true;
                    v.hidden_at = None;
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
                v.hidden_at = Some(std::time::Instant::now());
                // Trimmed after a short delay, not now: trimming right away
                // made quick switching back and forth slow.
                to_suspend.push(id.clone());
            }
        }
        drop(host);
        #[cfg(windows)]
        for id in to_suspend {
            Self::schedule_suspend(app, id);
        }
        #[cfg(not(windows))]
        let _ = to_suspend;
        if retry {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                let _ = tauri::async_runtime::spawn_blocking(move || Self::reconcile(&app)).await;
            });
        }
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

    /// Winds a hidden tab down in two steps, unless it is playing media
    /// (music, calls): after `TRIM_AFTER` WebView2 is asked to keep a low
    /// memory footprint, and after `SUSPEND_AFTER` its scripts are frozen.
    /// Showing the tab again in between cancels both. Neither returns the
    /// GPU process's memory (measured): only sleeping the tab does that.
    #[cfg(windows)]
    fn schedule_suspend(app: &Shared, id: String) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            // The view, if it is still hidden and has been for at least `after`.
            let still_hidden = |after: std::time::Duration| -> Option<Webview> {
                let media = app.store.lock().tab(&id).map(|t| t.has_active_media).unwrap_or(true);
                if media {
                    return None;
                }
                let host = app.host.lock();
                host.views
                    .get(&id)
                    .filter(|v| !v.shown && v.hidden_at.is_some_and(|t| t.elapsed() >= after))
                    .map(|v| v.webview.clone())
            };
            let slack = std::time::Duration::from_millis(500);
            tokio::time::sleep(TRIM_AFTER + slack).await;
            let Some(wv) = still_hidden(TRIM_AFTER) else { return };
            crate::webview_win::set_background(&wv, true);
            tokio::time::sleep(SUSPEND_AFTER - TRIM_AFTER).await;
            if let Some(wv) = still_hidden(SUSPEND_AFTER) {
                crate::webview_win::try_suspend(&wv);
            }
        });
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
        // Hole positions are relative to the page, which just moved.
        Self::apply_clips(app);
        if first {
            Self::reconcile(app);
        }
    }

    /// Holes in logical px relative to the page view, plus the page's
    /// logical width (the UI thread derives the pixel scale from it). Pure:
    /// callers read the text size `z` before taking the host lock, and nothing
    /// here locks or waits on anything else.
    fn clip_rects(host: &TabHost, z: f64) -> crate::webview_win::Clip {
        let Some(b) = host.bounds else { return crate::webview_win::Clip::default() };
        let holes = host
            .holes
            .values()
            .filter(|h| h.w > 0.5 && h.h > 0.5)
            .map(|h| (h.x * z - b.x, h.y * z - b.y, (h.x + h.w) * z - b.x, (h.y + h.h) * z - b.y, h.r * z))
            .collect();
        crate::webview_win::Clip { holes, page_width: b.w }
    }

    /// Applies the current holes to every page view on screen.
    pub fn apply_clips(app: &Shared) {
        #[cfg(windows)]
        {
            let z = app.store.lock().settings.text_scale;
            let host = app.host.lock();
            let clip = Self::clip_rects(&host, z);
            for v in host.views.values().filter(|v| v.shown) {
                crate::webview_win::set_clip(&v.webview, clip.clone());
            }
        }
        #[cfg(not(windows))]
        let _ = app;
    }

    /// Sets, animates or removes the hole owned by `id`. With `from` and
    /// `ms`, the hole moves from `from` to `rect` along the `ease` curve (the
    /// same curve as the CSS transition of the UI it uncovers); `remove`
    /// drops it once there.
    /// `lead_ms` runs the hole ahead of (or, negative, behind) the UI's own
    /// animation so the page never covers the moving edge.
    #[allow(clippy::too_many_arguments)]
    pub fn set_hole(app: &Shared, id: String, rect: Hole, from: Option<Hole>, ms: u32, ease: [f64; 4], remove: bool, lead_ms: f64) {
        let (gen, current) = {
            let mut host = app.host.lock();
            let g = host.hole_gen.entry(id.clone()).or_insert(0);
            *g += 1;
            (*g, host.holes.get(&id).copied())
        };
        // A change mid-animation continues from where the hole is now.
        let from = from.map(|f| current.unwrap_or(f));
        let hole_id = id.clone();
        let finish = move |app: &Shared| {
            let mut host = app.host.lock();
            if remove {
                host.holes.remove(&id);
            } else {
                host.holes.insert(id.clone(), rect);
            }
            drop(host);
            Self::apply_clips(app);
        };
        let Some(from) = from.filter(|_| ms > 0) else {
            finish(app);
            return;
        };
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let start = std::time::Instant::now();
            loop {
                let t = ((start.elapsed().as_secs_f64() * 1000.0 + lead_ms) / ms as f64).max(0.0);
                {
                    let mut host = app.host.lock();
                    if host.hole_gen.get(&hole_id) != Some(&gen) {
                        return; // superseded
                    }
                    if t >= 1.0 {
                        break;
                    }
                    host.holes.insert(hole_id.clone(), Hole::lerp(from, rect, cubic_bezier(ease, t)));
                }
                Self::apply_clips(&app);
                tokio::time::sleep(std::time::Duration::from_millis(7)).await;
            }
            if app.host.lock().hole_gen.get(&hole_id) == Some(&gen) {
                finish(&app);
            }
        });
    }

    /// Recreates page webviews when the VPN changes how they must connect
    /// (direct or through the tunnel proxy). Browser arguments are fixed per
    /// WebView2 environment, so open pages reload once.
    pub fn sync_network(app: &Shared) {
        let want = (app.vpn.needs_proxy(), app.vpn.webrtc_guard_active());
        {
            let mut host = app.host.lock();
            if host.net == want {
                return;
            }
            host.net = want;
            host.create_failures = 0;
            for (_, v) in host.views.drain() {
                let _ = v.webview.close();
            }
            if let Some((wv, _)) = host.assistant.take() {
                let _ = wv.close();
            }
        }
        Self::reconcile(app);
    }

    /// Snapshots a tab for hover previews shortly after its page loads, while
    /// it is on screen. (A tab is hidden the moment it's left, and WebView2
    /// can't capture a hidden view, so the switch itself is too late.)
    #[cfg(windows)]
    pub fn capture_after_load(app: &Shared, tab_id: String) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(900)).await;
            let wv = {
                let host = app.host.lock();
                match host.views.get(&tab_id) {
                    Some(v) if host.visible && v.shown => v.webview.clone(),
                    _ => return,
                }
            };
            let app2 = app.clone();
            crate::webview_win::capture(&wv, move |img| {
                if let Some(img) = img {
                    app2.host.lock().previews.insert(tab_id, img);
                }
            });
        });
    }

    pub fn set_visible(app: &Shared, visible: bool) {
        // Only a page that was on screen can hold keyboard focus. Moving focus
        // when the UI already has it makes the UI document blur and refocus,
        // which closes menus and hover cards that listen for window blur.
        let page_was_shown = {
            let active = app.store.lock().active_tab_id.clone();
            let mut host = app.host.lock();
            let shown = host.visible && active.as_ref().and_then(|id| host.views.get(id)).is_some_and(|v| v.shown);
            host.visible = visible;
            shown
        };
        Self::reconcile(app);
        if !visible && page_was_shown {
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
        // Never hold the host lock across `add_child` (see `TabHost::build`).
        drop(host);
        let wv = window
            .add_child(builder, LogicalPosition::new(b.x, b.y), LogicalSize::new(b.w.max(1.0), b.h.max(1.0)))
            .map_err(|e| e.to_string())?;
        if let Some((old, _)) = app.host.lock().assistant.replace((wv, b)) {
            let _ = old.close();
        }
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
