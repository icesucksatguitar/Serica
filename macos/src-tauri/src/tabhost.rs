//! Native tab webviews.
//!
//! Each live tab is a WebKit (`WKWebView`) child webview of the main window,
//! positioned over the UI's viewport rectangle (reported by the UI in CSS
//! pixels; Cocoa points match CSS pixels at 100%).
//! `reconcile` diffs the store against the existing webviews: it creates the
//! active tab's webview lazily, closes webviews of slept/closed/new-tab tabs,
//! and shows exactly one of them.
//!
//! Native webviews always paint above the HTML UI, so the UI "freezes" the
//! viewport while a modal or popover covers it: the active page is captured
//! to an image, shown by the UI, and the native webview is hidden.

use crate::app::Shared;
use crate::model::is_internal_url;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::webview::NewWindowResponse;
use tauri::{LogicalPosition, LogicalSize, Manager, Webview, WebviewBuilder, WebviewUrl};

static LABEL_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Split view geometry (UI CSS px; the UI draws the same): the gap between
/// the panes (the divider) and a strip above each pane (focus bar).
pub const SPLIT_GAP: f64 = 10.0;
pub const SPLIT_TOP: f64 = 3.0;

/// One pane of a split in the page area `b` (logical px, zoomed by `z`).
pub fn pane_bounds(b: Bounds, ratio: f64, left: bool, z: f64) -> Bounds {
    let gap = SPLIT_GAP * z;
    let top = (SPLIT_TOP * z).min(b.h);
    let w = (b.w - gap).max(0.0);
    let lw = (w * ratio.clamp(0.2, 0.8)).round();
    if left {
        Bounds { x: b.x, y: b.y + top, w: lw, h: b.h - top }
    } else {
        Bounds { x: b.x + lw + gap, y: b.y + top, w: w - lw, h: b.h - top }
    }
}

pub struct TabView {
    pub webview: Webview,
    pub profile: String,
    pub shown: bool,
    /// When the view was last hidden (drives background suspension).
    pub hidden_at: Option<std::time::Instant>,
    /// Built from an extension's configuration, for its own pages: WebKit
    /// loads nothing else in it (and those pages nowhere else).
    pub extension_page: bool,
    /// The page is in a window of its own (see `detached.rs`): the main
    /// window's reconcile neither shows, hides, clips nor suspends it.
    pub detached: bool,
}

/// An extension's own page (`webkit-extension://`).
pub(crate) fn is_extension_page(url: &str) -> bool {
    url.starts_with("webkit-extension:")
}

/// Hidden tabs are throttled by WebKit at once (timers, painting), and
/// suspended after `SUSPEND_AFTER` (WebKit's inactive scheduling policy,
/// the counterpart of WebView2's `TrySuspend`), unless they play media.
/// With memory to spare they stay live much longer, so switching back is
/// instant; Memory saver still frees tabs unused for longer.
const TRIM_AFTER: std::time::Duration = std::time::Duration::from_secs(10 * 60);
const SUSPEND_AFTER: std::time::Duration = std::time::Duration::from_secs(20 * 60);
/// The same steps while system memory runs low (`LOW_MEMORY_LOAD`% in use).
const TRIM_AFTER_LOW: std::time::Duration = std::time::Duration::from_secs(30);
const SUSPEND_AFTER_LOW: std::time::Duration = std::time::Duration::from_secs(5 * 60);
const LOW_MEMORY_LOAD: u32 = 75;

/// Finds the page's media elements: in the document, in open shadow roots
/// (players built as web components) and in same-origin frames. Players in
/// frames from another site are reached in their own frame (see
/// `webview_mac::media_eval`), where this runs again.
pub const MEDIA_FIND_JS: &str = r#"const __sericaMedia = () => {
  const quick = Array.from(document.querySelectorAll('video, audio'));
  if (quick.length) return quick;
  const out = [], seen = new Set();
  const walk = (root, depth) => {
    if (!root || depth > 5 || seen.has(root)) return;
    seen.add(root);
    let all;
    try { all = root.querySelectorAll('*'); } catch (e) { return; }
    for (const n of all) {
      if (n.tagName === 'VIDEO' || n.tagName === 'AUDIO') out.push(n);
      if (n.shadowRoot) walk(n.shadowRoot, depth + 1);
      if (n.tagName === 'IFRAME' || n.tagName === 'FRAME') { try { if (n.contentDocument) walk(n.contentDocument, depth + 1); } catch (e) {} }
    }
  };
  walk(document, 0);
  return out;
};"#;

/// Moves the largest playing video into a picture-in-picture window (the
/// mini player that keeps playing when its tab is left). Resolves true once
/// this document shows one (false when it has no such video, or the browser
/// refused). Sites that set `disablePictureInPicture` are overridden: the
/// user asked for the mini player.
pub fn pip_enter_js() -> String {
    format!(
        r#"(async () => {{
  {MEDIA_FIND_JS}
  if (document.pictureInPictureElement) return true;
  const vs = __sericaMedia().filter((v) => v.tagName === 'VIDEO' && !v.paused && !v.ended && v.readyState >= 2);
  if (vs.some((v) => v.webkitPresentationMode === 'picture-in-picture')) return true;
  if (!vs.length) return false;
  vs.sort((a, b) => b.clientWidth * b.clientHeight - a.clientWidth * a.clientHeight);
  const v = vs[0];
  if (v.clientWidth * v.clientHeight < 200 * 112) return false;
  if (v.disablePictureInPicture) v.disablePictureInPicture = false;
  if (document.pictureInPictureEnabled && v.requestPictureInPicture) {{
    try {{ await v.requestPictureInPicture(); return true; }} catch (e) {{}}
  }}
  // WebKit's own presentation API (macOS picture in picture).
  if (v.webkitSupportsPresentationMode && v.webkitSupportsPresentationMode('picture-in-picture')) {{
    try {{ v.webkitSetPresentationMode('picture-in-picture'); return true; }} catch (e) {{}}
  }}
  return false;
}})()"#
    )
}

/// Brings a picture-in-picture video back into its page. Runs right as the tab
/// is shown, so it waits until the document is actually visible. True when
/// this document had one.
pub const PIP_EXIT_JS: &str = r#"(() => {
  const wk = Array.from(document.querySelectorAll('video')).filter((v) => v.webkitPresentationMode === 'picture-in-picture');
  if (!document.pictureInPictureElement && !wk.length) return false;
  const exit = () => {
    if (document.pictureInPictureElement) document.exitPictureInPicture().catch(() => {});
    for (const v of wk) { try { v.webkitSetPresentationMode('inline'); } catch (e) {} }
  };
  if (document.visibilityState === 'visible') { exit(); return true; }
  const onVisible = () => {
    if (document.visibilityState !== 'visible') return;
    document.removeEventListener('visibilitychange', onVisible);
    exit();
  };
  document.addEventListener('visibilitychange', onVisible);
  return true;
})()"#;

/// The playing (or last played) media element's state for the mini player
/// bar, or null when this document has none.
pub fn media_state_js() -> String {
    format!(
        r#"(() => {{
  {MEDIA_FIND_JS}
  const els = __sericaMedia();
  const m = els.find((e) => !e.paused && !e.ended) || els.find((e) => e.currentTime > 0) || null;
  if (!m) return null;
  const d = m.duration;
  const live = d === Infinity || isNaN(d);
  return {{ hasVideo: m.tagName === 'VIDEO', isPlaying: !m.paused && !m.ended, currentTime: m.currentTime || 0,
           duration: live ? 0 : (d || 0), volume: m.muted ? 0 : (m.volume ?? 1), tabTitle: (window.top === window ? document.title : '') || 'Media',
           inPip: !!document.pictureInPictureElement || m.webkitPresentationMode === 'picture-in-picture' }};
}})()"#
    )
}

/// Runs `body` (which uses `m`) on the playing media element; true when this
/// document had one.
pub fn media_action_js(body: &str) -> String {
    format!(
        r#"(() => {{
  {MEDIA_FIND_JS}
  const els = __sericaMedia();
  const m = els.find((e) => !e.paused) || els.find((e) => e.currentTime > 0) || els[0];
  if (!m) return false;
  {body}
  return true;
}})()"#
    )
}

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
    pub(crate) fn lerp(a: Hole, b: Hole, p: f64) -> Hole {
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
    /// Applies the holes to the page views (layer masks, at display refresh).
    pub clipper: std::sync::Arc<crate::clip_mac::Clipper>,
    pub bounds: Option<Bounds>,
    /// False while a UI overlay covers the viewport.
    pub visible: bool,
    pub active: Option<String>,
    /// The split on screen (the active tab's), if any.
    pub split: Option<crate::model::Split>,
    pub assistant: Option<(Webview, Bounds)>,
    /// Last page of each assistant (by host), so switching assistants or
    /// closing the panel returns to the same chat instead of its home page.
    assistant_last: HashMap<String, url::Url>,
    data_root: PathBuf,
    proxy_port: Option<u16>,
    /// Network setup the current page webviews were created with:
    /// (routed through the local tunnel proxy, WebRTC guard script).
    net: (bool, bool),
    /// Third-party cookies blocked in the current page webviews (applies to
    /// pages created after a change; open pages are recreated).
    block_3pc: bool,
    /// HTML fullscreen: WebKit shows the page in its own window, so the UI's
    /// layout changes meanwhile don't move it.
    html_fullscreen: bool,
    /// Failed creations in a row (a profile's browser process may still be
    /// shutting down after a network change); creation is retried a few times.
    create_failures: u32,
    /// Tabs whose webview is being built right now (outside the lock).
    pub(crate) creating: std::collections::HashSet<String>,
    /// While set, pages that come on screen leave keyboard focus in the UI. Set for a space switch made from the open sidebar, where the
    /// pointer is still in use: a focused page takes the touchpad's scroll
    /// and swipe input, so the sidebar could not swipe again until clicked.
    /// The UI hands focus to the page (`focus_page`) once the sidebar closes.
    pub keep_ui_focus: bool,
}

/// A tab webview ready to be built (see `TabHost::prepare`).
pub(crate) struct PendingView {
    window: tauri::Window,
    label: String,
    bounds: Bounds,
    url: url::Url,
    net: crate::webview_mac::Net,
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

impl TabHost {
    pub fn new(data_root: PathBuf, proxy_port: Option<u16>, net: (bool, bool), block_3pc: bool) -> Self {
        Self {
            views: HashMap::new(),
            clipper: crate::clip_mac::Clipper::new(),
            bounds: None,
            visible: true,
            active: None,
            split: None,
            assistant: None,
            assistant_last: HashMap::new(),
            data_root,
            proxy_port,
            net,
            block_3pc,
            html_fullscreen: false,
            create_failures: 0,
            creating: std::collections::HashSet::new(),
            keep_ui_focus: false,
        }
    }

    /// Where a profile's data is said to live (the UI shows it as the
    /// profile's "partition"). WebKit keeps it in its own website data
    /// store, found by the profile's identifier.
    pub fn profile_dir(&self, profile: &str) -> PathBuf {
        self.data_root.join(sanitize(profile))
    }

    /// How pages connect: through the local tunnel proxy (when the VPN needs
    /// it) and with the WebRTC guard.
    fn net_options(&self) -> crate::webview_mac::Net {
        crate::webview_mac::Net { proxy_port: if self.net.0 { self.proxy_port } else { None }, webrtc_guard: self.net.1 }
    }

    /// A page webview of a profile, on Serica's own WebKit configuration
    /// (built on the main thread, see `webview_mac::configuration`).
    fn builder(app: &Shared, label: String, url: url::Url, config: crate::mac_util::MainBox<objc2::rc::Retained<objc2_web_kit::WKWebViewConfiguration>>, tab_id: Option<&str>) -> WebviewBuilder<tauri::Wry> {
        let mut b = WebviewBuilder::new(label, WebviewUrl::External(url))
            .with_webview_configuration(config.0)
            // Pages take dropped files (uploads), links and text themselves,
            // and HTML5 drag and drop inside them works, as in Safari.
            .disable_drag_drop_handler()
            // Not focused when built: `reconcile` gives focus to the page on
            // screen when it should have it. Built focused, a page woken by a
            // space swipe took focus from the sidebar (and with it the
            // touchpad), so the next swipe went to the page.
            .focused(false)
            .background_color(tauri::webview::Color(255, 255, 255, 255))
            .devtools(true);
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
                    // A private page's links stay private.
                    if s.tab(&id2).is_some_and(|t| t.private) {
                        s.create_private_tab(Some(target));
                        return;
                    }
                    let space = s.tab(&id2).map(|t| t.space_id.clone());
                    s.create_tab(Some(target), space, true, Some(&id2));
                });
                NewWindowResponse::Deny
            });
        }
        b
    }

    /// Everything needed to create a tab webview at the current viewport
    /// bounds. Cheap; done under the host lock.
    fn prepare(&self, app: &Shared, tab_id: &str, url: &str) -> Option<PendingView> {
        let bounds = self.bounds?;
        let window = app.handle.get_window("main")?;
        self.prepare_in(tab_id, url, window, bounds)
    }

    /// `prepare` for a page in `window` at `bounds` (a tab's own window).
    pub(crate) fn prepare_in(&self, tab_id: &str, url: &str, window: tauri::Window, bounds: Bounds) -> Option<PendingView> {
        let parsed = url::Url::parse(url).ok()?;
        let label = format!("tab-{}-{}", LABEL_SEQ.fetch_add(1, Ordering::Relaxed), sanitize(tab_id));
        Some(PendingView { window, label, bounds, url: parsed, net: self.net_options() })
    }

    /// Creates the webview. `add_child` blocks until the main thread has
    /// built it, and the main thread takes the host lock in its own work
    /// (preview captures finishing), so this must run without the host lock
    /// held, or both threads wait on each other forever. The page starts
    /// blank so its handlers are in place before its first load (see
    /// `webview_mac::attach`).
    pub(crate) fn build(app: &Shared, tab_id: &str, profile: &str, private: bool, p: PendingView) -> Option<Webview> {
        let PendingView { window, label, bounds, url: parsed, net } = p;
        let _perf = crate::perf::span("build_webview(any thread)");
        // An extension's own pages load only in its configuration.
        let config = match parsed.scheme() {
            "webkit-extension" => crate::extensions::webkit::page_configuration(app, parsed.as_str())?,
            _ => crate::webview_mac::configuration(app, profile, private, net, crate::webview_mac::Kind::Tab)?,
        };
        let blank = url::Url::parse("about:blank").ok()?;
        let builder = Self::builder(app, label, blank, config, Some(tab_id));
        match window.add_child(builder, LogicalPosition::new(bounds.x, bounds.y), LogicalSize::new(bounds.w.max(1.0), bounds.h.max(1.0))) {
            Ok(wv) => {
                crate::webview_mac::attach(app, tab_id, profile, &wv, Some(parsed.to_string()), crate::webview_mac::Kind::Tab, private);
                Some(wv)
            }
            Err(e) => {
                tracing::error!("failed to create webview for {tab_id}: {e}");
                None
            }
        }
    }

    pub(crate) fn apply_bounds(wv: &Webview, b: Bounds) {
        let _ = wv.set_bounds(tauri::Rect {
            position: LogicalPosition::new(b.x, b.y).into(),
            size: LogicalSize::new(b.w.max(1.0), b.h.max(1.0)).into(),
        });
    }

    /// Unhooks a page webview (answering what it still waits on) and closes it.
    fn close_view(wv: &Webview) {
        crate::webview_mac::detach(wv);
        let _ = wv.close();
    }

    /// Brings native webviews in line with the store.
    pub fn reconcile(app: &Shared) {
        let _perf = crate::perf::span("reconcile");
        struct Want {
            url: String,
            live: bool,
            profile: String,
            media: bool,
            zoom: f64,
            private: bool,
        }
        let (wants, active, auto_pip, text_scale, split): (HashMap<String, Want>, Option<String>, bool, f64, Option<crate::model::Split>) = {
            let s = app.store.lock();
            // The active tab's split partner is on screen beside it.
            let split = s.active_tab_id.as_deref().and_then(|a| s.split_of(a)).cloned();
            let wants = s
                .tabs
                .iter()
                .map(|t| {
                    let want = Want {
                        url: t.url.clone(),
                        live: t.wants_webview(),
                        profile: s.space_profile(&t.space_id),
                        media: t.has_active_media,
                        private: t.private,
                        // Zoom is per site; other pages keep the tab's own.
                        zoom: if crate::settings::zoom_site(&t.url).is_some() {
                            s.settings.zoom_for(&t.url)
                        } else {
                            t.zoom_factor.filter(|z| z.is_finite() && *z > 0.0).unwrap_or(s.settings.text_scale)
                        },
                    };
                    (t.id.clone(), want)
                })
                .collect();
            (wants, s.active_tab_id.clone(), s.settings.auto_picture_in_picture, s.settings.text_scale, split)
        };
        // Both panes need a page: a split whose partner has none shows as a single tab.
        let split = split.filter(|sp| [&sp.left, &sp.right].iter().all(|id| wants.get(*id).is_some_and(|w| w.live && !is_internal_url(&w.url))));
        let on_screen = |id: &str| Some(id) == active.as_deref() || split.as_ref().is_some_and(|s| s.has(id));
        let mut host = app.host.lock();

        // Close webviews whose tab is gone, asleep, unloaded, on an internal
        // page, or moved to another profile (which needs another data store).
        let stale: Vec<String> = host
            .views
            .iter()
            .filter(|(id, v)| wants.get(*id).map(|w| !w.live || w.profile != v.profile).unwrap_or(true))
            .map(|(id, _)| id.clone())
            .collect();
        // Pages in windows of their own that go too: their window closes.
        let mut windows_to_close = Vec::new();
        for id in stale {
            if let Some(v) = host.views.remove(&id) {
                Self::close_view(&v.webview);
                if v.detached {
                    windows_to_close.push(id.clone());
                }
            }
            if !wants.contains_key(&id) {
                app.previews.remove(&id);
            }
        }
        // Private tabs share an in-memory store per profile; once none of a
        // profile's private pages is open, its store (and all it held) goes.
        let private_profiles: std::collections::HashSet<String> =
            wants.values().filter(|w| w.private && w.live).map(|w| w.profile.clone()).collect();
        crate::webview_mac::release_private_stores(&app.handle, private_profiles);

        let previous = host.active.clone();
        host.active = active.clone();
        let visible = host.visible;
        let bounds = host.bounds;
        let mut previews_to_capture = Vec::new();
        let mut to_suspend = Vec::new();
        // A page that comes on screen takes keyboard focus, as in other
        // browsers. Without this, closing the focused tab left focus nowhere
        // and shortcuts typed right after (Command-T) were lost.
        let mut focus_page: Option<Webview> = None;
        let active_shown_before = active.as_ref().and_then(|id| host.views.get(id)).is_some_and(|v| v.shown);
        let fullscreen = host.html_fullscreen;
        host.split = split.clone();
        let pane = |id: &str| -> Option<Bounds> {
            let b = bounds?;
            Some(match &split {
                Some(sp) if sp.has(id) => pane_bounds(b, sp.ratio, sp.left == id, text_scale),
                _ => b,
            })
        };
        // The page with keyboard focus is leaving the screen and no page takes
        // focus after it (a switch that keeps focus in the UI, or one to a
        // page still to be built): the UI takes it first. Otherwise focus
        // stayed in the hidden page, and keys and touchpad swipes went
        // nowhere until the window was clicked.
        if visible {
            let focused = app.focused_page.lock().clone();
            let leaving = focused.as_deref().is_some_and(|f| !on_screen(f) && host.views.get(f).is_some_and(|v| v.shown && !v.detached));
            let next_takes_focus = !host.keep_ui_focus && active.as_ref().is_some_and(|a| host.views.contains_key(a));
            if leaving && !next_takes_focus {
                if let Some(main) = app.handle.get_webview("main") {
                    let _ = main.set_focus();
                }
            }
        }
        for (id, v) in host.views.iter_mut() {
            if v.detached {
                continue;
            }
            let should_show = visible && on_screen(id);
            // Leaving a playing video for another tab: keep watching it in a
            // mini player (also when the page was under a dialog at the time,
            // e.g. switching from the command bar). Not when UI only covers
            // the page for a moment: the tab is still the one in use.
            if auto_pip && previous.as_ref() == Some(id) && active.as_ref() != Some(id) && !should_show && wants.get(id).map(|w| w.media).unwrap_or(false) {
                Self::pip_on_leave(app, id.clone(), v.webview.clone());
            }
            if should_show {
                if let (Some(b), false) = (pane(id), fullscreen) {
                    Self::apply_bounds(&v.webview, b);
                }
                if !v.shown {
                    // Not when the page only comes back from under UI that
                    // covered it (menus, previews): moving focus then blurs the
                    // UI, which closes the auto-hide sidebar.
                    if (previous.as_ref() != Some(id) || v.hidden_at.is_none()) && active.as_ref() == Some(id) {
                        focus_page = Some(v.webview.clone());
                    }
                    crate::webview_mac::set_suspended(&v.webview, false);
                    let _ = v.webview.show();
                    v.shown = true;
                    crate::extensions::webkit_tabs::tab_activated(app, id);
                    v.hidden_at = None;
                    // Returning to a tab brings its mini-player video back into the page.
                    crate::webview_mac::media_eval(&v.webview, id.clone(), PIP_EXIT_JS.to_string(), false, |_| {});
                }
            } else if v.shown {
                // Snapshot the page we're leaving for hover previews.
                if previous.as_ref() == Some(id) {
                    previews_to_capture.push((id.clone(), v.webview.clone()));
                }
                let _ = v.webview.hide();
                v.shown = false;
                v.hidden_at = Some(std::time::Instant::now());
                to_suspend.push(id.clone());
            }
        }
        // Switched to a tab without a page on screen (new tab page, or a page
        // still being built): the UI takes focus so its shortcuts work.
        let focus_ui = visible && focus_page.is_none() && !active_shown_before && previous != active;
        // The UI asked to keep focus (see `keep_ui_focus`): pages come on
        // screen without taking it (including one still being built for the
        // switch, shown by a later pass) until the UI calls `focus_page`.
        if host.keep_ui_focus {
            focus_page = None;
        }
        Self::sync_clip_pages(&host, text_scale);
        drop(host);
        if !windows_to_close.is_empty() {
            crate::detached::page_gone(app, windows_to_close);
        }
        if let Some(wv) = focus_page {
            let _ = wv.set_focus();
        } else if focus_ui {
            if let Some(main) = app.handle.get_webview("main") {
                let _ = main.set_focus();
            }
        }
        for id in to_suspend {
            Self::schedule_suspend(app, id);
        }
        for (id, wv) in previews_to_capture {
            let app = app.clone();
            crate::webview_mac::capture(&wv, move |img| {
                if let Some(img) = img {
                    app.previews.put(&id, img);
                }
            });
        }

        // Build the active tab's webview last, once the page being left is
        // hidden: building takes a while (a slept tab waking up) and the old
        // page must not stay on screen under the new tab's address meanwhile.
        // The split partner's page too (a woken tab, or one restored at start).
        let partner = split.as_ref().zip(active.as_ref()).and_then(|(sp, a)| sp.partner(a).map(str::to_string));
        for id in active.iter().chain(partner.iter()) {
            if let Some(w) = wants.get(id) {
                if w.live && !is_internal_url(&w.url) {
                    Self::create_active(app, id, w.url.clone(), w.profile.clone(), w.zoom, w.private);
                }
            }
        }
    }

    /// Puts a playing video of a tab just left into the mini player
    /// (picture in picture). WebKit now and then refuses while the page is
    /// being hidden: then it tries once more, if the tab is still in the
    /// background.
    fn pip_on_leave(app: &Shared, id: String, wv: Webview) {
        let app = app.clone();
        crate::webview_mac::media_eval(&wv.clone(), id.clone(), pip_enter_js(), true, move |ok| {
            if ok == Some(serde_json::Value::Bool(true)) {
                return;
            }
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(350)).await;
                if app.store.lock().active_tab_id.as_deref() != Some(id.as_str()) {
                    crate::webview_mac::media_eval(&wv, id, pip_enter_js(), true, |_| {});
                }
            });
        });
    }

    /// Builds the active tab's webview, then reconciles again to show it. The
    /// host lock is released while it is built, so another reconcile may be
    /// building it already.
    fn create_active(app: &Shared, id: &str, url: String, profile: String, zoom: f64, private: bool) {
        let mut host = app.host.lock();
        if host.views.contains_key(id) || host.creating.contains(id) {
            return;
        }
        let created = match host.prepare(app, id, &url) {
            Some(p) => {
                host.creating.insert(id.to_string());
                drop(host);
                let wv = Self::build(app, id, &profile, private, p);
                host = app.host.lock();
                host.creating.remove(id);
                wv
            }
            None => None,
        };
        let Some(wv) = created else {
            if host.create_failures < 20 {
                host.create_failures += 1;
                drop(host);
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                    let _ = tauri::async_runtime::spawn_blocking(move || Self::reconcile(&app)).await;
                });
            }
            return;
        };
        host.create_failures = 0;
        let _ = wv.hide();
        if (zoom - 1.0).abs() > 0.001 {
            crate::webview_mac::set_zoom(&wv, zoom);
        }
        crate::clip_mac::register(&wv);
        let extension_page = is_extension_page(&url);
        if let Some(old) = host.views.insert(id.to_string(), TabView { webview: wv, profile, shown: false, hidden_at: None, extension_page, detached: false }) {
            Self::close_view(&old.webview);
        }
        // The state was read before the lock was released for the build: a
        // reconcile that ran meanwhile (tab closed, slept or switched away)
        // could not see this view. Start over from fresh state, which shows
        // it (or closes it); the view exists now, so nothing is built again.
        drop(host);
        Self::reconcile(app);
    }

    /// Suspends a hidden tab after a while, unless it is playing media
    /// (music, calls). WebKit already throttles hidden pages' timers and
    /// painting; suspending stops their scripts until the tab is shown
    /// again. Showing the tab in between cancels it.
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
                    .filter(|v| !v.shown && !v.detached && v.hidden_at.is_some_and(|t| t.elapsed() >= after))
                    .map(|v| v.webview.clone())
            };
            let low = crate::app::memory_load_percent().is_some_and(|l| l >= LOW_MEMORY_LOAD);
            let (trim_after, suspend_after) = if low { (TRIM_AFTER_LOW, SUSPEND_AFTER_LOW) } else { (TRIM_AFTER, SUSPEND_AFTER) };
            let slack = std::time::Duration::from_millis(500);
            tokio::time::sleep(trim_after + slack).await;
            if still_hidden(trim_after).is_none() {
                return;
            }
            tokio::time::sleep(suspend_after - trim_after).await;
            if let Some(wv) = still_hidden(suspend_after) {
                crate::webview_mac::set_suspended(&wv, true);
            }
        });
    }

    /// Navigates a tab's existing webview. Returns false if it has none yet
    /// (`reconcile` will create it with the tab's URL).
    pub fn navigate(app: &Shared, tab_id: &str, url: &str) -> bool {
        let mut host = app.host.lock();
        let Some(v) = host.views.get(tab_id) else { return false };
        if url::Url::parse(url).is_err() {
            return false;
        }
        // From an extension's page to a website or back: a view of the other
        // kind is built (with the tab's new URL).
        if v.extension_page != is_extension_page(url) {
            if let Some(old) = host.views.remove(tab_id) {
                Self::close_view(&old.webview);
            }
            drop(host);
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || Self::reconcile(&app));
            return false;
        }
        crate::webview_mac::load(&v.webview, url);
        true
    }

    pub fn webview(app: &Shared, tab_id: &str) -> Option<Webview> {
        app.host.lock().views.get(tab_id).map(|v| v.webview.clone())
    }

    pub fn active_webview(app: &Shared) -> Option<Webview> {
        let id = app.store.lock().active_tab_id.clone()?;
        Self::webview(app, &id)
    }

    pub fn set_bounds(app: &Shared, b: Bounds) {
        let z = app.store.lock().settings.text_scale;
        let mut host = app.host.lock();
        host.bounds = Some(b);
        let first = host.views.is_empty();
        let split = host.split.clone();
        // Always apply (the UI already dedupes): this also repairs a native
        // view that ended up somewhere else, e.g. right after creation. Not
        // during HTML fullscreen, when WebKit has the page in its own window.
        if !host.html_fullscreen {
            for (id, v) in host.views.iter().filter(|(_, v)| v.shown && !v.detached) {
                let r = match &split {
                    Some(sp) if sp.has(id) => pane_bounds(b, sp.ratio, &sp.left == id, z),
                    _ => b,
                };
                Self::apply_bounds(&v.webview, r);
            }
        }
        drop(host);
        // Hole positions are relative to the page, which just moved.
        Self::apply_clips(app);
        if first {
            Self::reconcile(app);
        }
    }

    /// HTML fullscreen started or ended (see `set_bounds`); on the way out
    /// the page goes back to where the UI wants it.
    pub fn set_html_fullscreen(app: &Shared, fs: bool) {
        let (views, bounds) = {
            let mut host = app.host.lock();
            host.html_fullscreen = fs;
            (host.views.values().filter(|v| v.shown).map(|v| v.webview.clone()).collect::<Vec<_>>(), host.bounds)
        };
        if !fs {
            if let Some(b) = bounds {
                let app = app.clone();
                // After WebKit has put the view back in the window.
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                    if !app.host.lock().html_fullscreen {
                        for wv in &views {
                            Self::apply_bounds(wv, b);
                        }
                    }
                });
            }
        }
    }

    /// Tells the clipper which page views are on screen and where the page
    /// area is (holes are in window coordinates).
    fn sync_clip_pages(host: &TabHost, z: f64) -> u64 {
        let b = host.bounds.unwrap_or(Bounds { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
        let pages = host
            .views
            .iter()
            .filter(|(_, v)| v.shown)
            .map(|(id, v)| {
                let r = match &host.split {
                    Some(sp) if sp.has(id) => pane_bounds(b, sp.ratio, &sp.left == id, z),
                    _ => b,
                };
                (v.webview.label().to_string(), (r.x, r.y))
            })
            .collect();
        host.clipper.set_pages(pages, z)
    }

    /// Applies the current holes to every page view on screen.
    pub fn apply_clips(app: &Shared) {
        let z = app.store.lock().settings.text_scale;
        let host = app.host.lock();
        Self::sync_clip_pages(&host, z);
    }

    /// Sets, animates or removes the hole owned by `id`. With `from` and
    /// `ms`, the hole moves from `from` to `rect` along the `ease` curve (the
    /// same curve as the CSS transition of the UI it uncovers); `remove`
    /// drops it once there.
    /// `lead_ms` runs the hole ahead of (or, negative, behind) the UI's own
    /// animation so the page never covers the moving edge.
    /// Returns a sequence number to wait for with `wait_clip`.
    #[allow(clippy::too_many_arguments)]
    pub fn set_hole(app: &Shared, id: String, rect: Hole, from: Option<Hole>, ms: u32, ease: [f64; 4], remove: bool, lead_ms: f64) -> u64 {
        let clipper = app.host.lock().clipper.clone();
        clipper.set_hole(id, rect, from, ms, ease, remove, lead_ms)
    }

    /// Blocks until hole change `seq` is on screen (or `timeout` passes).
    pub fn wait_clip(app: &Shared, seq: u64, timeout: std::time::Duration) {
        let clipper = app.host.lock().clipper.clone();
        clipper.wait_applied(seq, timeout);
    }

    /// Recreates page webviews when third-party cookie blocking is switched:
    /// the data stores take the new policy, and open pages reload once (as
    /// on Windows, where it is a browser argument).
    pub fn sync_cookie_policy(app: &Shared, block: bool) {
        {
            let mut host = app.host.lock();
            if host.block_3pc == block {
                return;
            }
            host.block_3pc = block;
            host.create_failures = 0;
            for (_, v) in host.views.drain() {
                Self::close_view(&v.webview);
            }
            if let Some((wv, _)) = host.assistant.take() {
                Self::close_view(&wv);
            }
        }
        crate::webview_mac::set_cookie_policy(&app.handle, block);
        Self::reconcile(app);
    }

    /// Recreates page webviews when the VPN changes how they must connect
    /// (direct or through the tunnel proxy): the data stores take the new
    /// proxy setup, and open pages reload once, so no connection made the
    /// old way lingers.
    pub fn sync_network(app: &Shared) {
        let want = (app.vpn.needs_proxy(), app.vpn.webrtc_guard_active());
        let port = {
            let mut host = app.host.lock();
            if host.net == want {
                return;
            }
            host.net = want;
            host.create_failures = 0;
            for (_, v) in host.views.drain() {
                Self::close_view(&v.webview);
            }
            if let Some((wv, _)) = host.assistant.take() {
                Self::close_view(&wv);
            }
            host.net_options().proxy_port
        };
        crate::webview_mac::apply_network(&app.handle, port);
        Self::reconcile(app);
    }

    /// Snapshots a tab for hover previews shortly after its page loads, while
    /// it is on screen (a hidden page can't be captured).
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
            crate::webview_mac::capture(&wv, move |img| {
                if let Some(img) = img {
                    app2.previews.put(&tab_id, img);
                }
            });
        });
    }

    /// Applies the dark-PDF setting to every open page (no-op on non-PDFs).
    pub fn apply_pdf_dark(app: &Shared) {
        let views: Vec<(String, Webview)> = app.host.lock().views.iter().map(|(id, v)| (id.clone(), v.webview.clone())).collect();
        for (tab_id, wv) in views {
            crate::webview_mac::apply_pdf_dark(&wv, app.pdf_dark_for(&tab_id));
        }
    }

    /// The UI is done keeping focus (see `keep_ui_focus`): pages take it as
    /// usual again, and with `focus` the active page, if on screen, gets it now.
    pub fn focus_page(app: &Shared, focus: bool) {
        let active = app.store.lock().active_tab_id.clone();
        let page = {
            let mut host = app.host.lock();
            host.keep_ui_focus = false;
            let visible = host.visible;
            active.as_ref().and_then(|id| host.views.get(id)).filter(|v| visible && v.shown).map(|v| v.webview.clone())
        };
        if let Some(wv) = page.filter(|_| focus) {
            let _ = wv.set_focus();
        }
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
        crate::detached::close_all(app);
        let mut host = app.host.lock();
        for (_, v) in host.views.drain() {
            Self::close_view(&v.webview);
        }
        if let Some((wv, _)) = host.assistant.take() {
            Self::close_view(&wv);
        }
    }

    // ------------------------------------------------------------- assistant

    pub fn assistant_open(app: &Shared, url: &str, b: Bounds) -> Result<(), String> {
        let parsed = url::Url::parse(url).map_err(|e| e.to_string())?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err("unsupported url".into());
        }
        let profile = app.store.lock().active_profile_id();
        // Read the open view's page outside the host lock: `url()` waits on
        // the main thread, which may itself be waiting for that lock.
        let open = app.host.lock().assistant.as_ref().map(|(w, _)| w.clone());
        if let Some(wv) = open {
            let current = wv.url().ok();
            let mut host = app.host.lock();
            if let Some(u) = &current {
                host.assistant_remember(&profile, u);
            }
            // Reopening the same assistant: leave its page as it is.
            if current.as_ref().and_then(|u| u.host_str()) != parsed.host_str() {
                let target = host.assistant_resume(&profile, parsed);
                crate::webview_mac::load(&wv, target.as_str());
            }
            Self::apply_bounds(&wv, b);
            let _ = wv.show();
            host.assistant = Some((wv, b));
            return Ok(());
        }
        let window = app.handle.get_window("main").ok_or("no window")?;
        let (target, net) = {
            let host = app.host.lock();
            (host.assistant_resume(&profile, parsed), host.net_options())
        };
        let label = format!("assistant-{}", LABEL_SEQ.fetch_add(1, Ordering::Relaxed));
        // Never hold the host lock across `add_child` (see `TabHost::build`).
        let config = crate::webview_mac::configuration(app, &profile, false, net, crate::webview_mac::Kind::Assistant).ok_or("no main thread")?;
        let blank = url::Url::parse("about:blank").map_err(|e| e.to_string())?;
        let builder = Self::builder(app, label, blank, config, None);
        let wv = window
            .add_child(builder, LogicalPosition::new(b.x, b.y), LogicalSize::new(b.w.max(1.0), b.h.max(1.0)))
            .map_err(|e| e.to_string())?;
        crate::webview_mac::attach(app, "assistant", &profile, &wv, Some(target.to_string()), crate::webview_mac::Kind::Assistant, false);
        if let Some((old, _)) = app.host.lock().assistant.replace((wv, b)) {
            Self::close_view(&old);
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
        let Some((wv, _)) = app.host.lock().assistant.take() else { return };
        if let Ok(u) = wv.url() {
            let profile = app.store.lock().active_profile_id();
            app.host.lock().assistant_remember(&profile, &u);
        }
        Self::close_view(&wv);
    }

    /// Notes the page an assistant is on, for `assistant_resume`.
    fn assistant_remember(&mut self, profile: &str, u: &url::Url) {
        if matches!(u.scheme(), "http" | "https") {
            if let Some(h) = u.host_str() {
                self.assistant_last.insert(format!("{profile} {h}"), u.clone());
            }
        }
    }

    /// The assistant's last page in this profile, else `url` (its home).
    fn assistant_resume(&self, profile: &str, url: url::Url) -> url::Url {
        url.host_str()
            .and_then(|h| self.assistant_last.get(&format!("{profile} {h}")))
            .cloned()
            .unwrap_or(url)
    }

    pub fn assistant_webview(app: &Shared) -> Option<Webview> {
        app.host.lock().assistant.as_ref().map(|(w, _)| w.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bezier_ends_and_middle() {
        let ease = [0.25, 0.1, 0.25, 1.0];
        assert!((cubic_bezier(ease, 0.0)).abs() < 1e-6);
        assert!((cubic_bezier(ease, 1.0) - 1.0).abs() < 1e-6);
        let mid = cubic_bezier(ease, 0.5);
        assert!(mid > 0.5 && mid < 0.9, "{mid}");
        let lin = cubic_bezier([0.0, 0.0, 1.0, 1.0], 0.3);
        assert!((lin - 0.3).abs() < 1e-3, "{lin}");
    }

    #[test]
    fn holes_interpolate() {
        let a = Hole { x: 0.0, y: 0.0, w: 10.0, h: 10.0, r: 0.0 };
        let b = Hole { x: 10.0, y: 20.0, w: 30.0, h: 40.0, r: 8.0 };
        let m = Hole::lerp(a, b, 0.5);
        assert_eq!(m, Hole { x: 5.0, y: 10.0, w: 20.0, h: 25.0, r: 4.0 });
    }

    #[test]
    fn media_scripts_reach_webkit_pip() {
        assert!(pip_enter_js().contains("webkitSetPresentationMode('picture-in-picture')"));
        assert!(pip_enter_js().contains("requestPictureInPicture"));
        assert!(PIP_EXIT_JS.contains("webkitSetPresentationMode('inline')"));
        assert!(media_state_js().contains("webkitPresentationMode"));
        assert_eq!(sanitize("tab:1/x"), "tab_1_x");
    }
}
