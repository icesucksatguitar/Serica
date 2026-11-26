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

/// Hidden tabs drop to WebView2's low-memory level after `TRIM_AFTER`, and
/// are frozen after `SUSPEND_AFTER`, unless they play media. With memory to
/// spare they stay fully live much longer, so switching back is instant
/// (a trimmed tab repaints from scratch, a frozen one wakes its scripts);
/// memory grows linearly with the tabs used recently, and Memory saver
/// still frees tabs unused for longer.
const TRIM_AFTER: std::time::Duration = std::time::Duration::from_secs(10 * 60);
const SUSPEND_AFTER: std::time::Duration = std::time::Duration::from_secs(20 * 60);
/// The same steps while system memory runs low (`LOW_MEMORY_LOAD`% in use).
const TRIM_AFTER_LOW: std::time::Duration = std::time::Duration::from_secs(30);
const SUSPEND_AFTER_LOW: std::time::Duration = std::time::Duration::from_secs(5 * 60);
const LOW_MEMORY_LOAD: u32 = 75;

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
    /// Applies the holes to the page windows (off the main thread).
    #[cfg(windows)]
    pub clipper: std::sync::Arc<crate::clip_win::Clipper>,
    pub bounds: Option<Bounds>,
    /// False while a UI overlay covers the viewport.
    pub visible: bool,
    pub active: Option<String>,
    pub assistant: Option<(Webview, Bounds)>,
    data_root: PathBuf,
    proxy_port: Option<u16>,
    /// Network setup the current page webviews were created with:
    /// (routed through the local tunnel proxy, WebRTC guard script).
    net: (bool, bool),
    /// Third-party cookies blocked in the current page webviews (a browser
    /// argument, fixed per WebView2 environment).
    block_3pc: bool,
    /// Failed creations in a row (a profile's browser process may still be
    /// shutting down after a network change); creation is retried a few times.
    create_failures: u32,
    /// Tabs whose webview is being built right now (outside the lock).
    creating: std::collections::HashSet<String>,
    /// Profiles whose pages are kept closed for now (their password store is
    /// being written by an import).
    paused: std::collections::HashSet<String>,
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
  // Scrolling settled: the app refreshes its capture of the page, which it
  // draws under panels (sidebar, top bar) sliding over the page.
  let settle = 0;
  addEventListener('scroll', () => {
    clearTimeout(settle);
    settle = setTimeout(() => { if (document.visibilityState === 'visible') post({ t: 'settled' }); }, 300);
  }, { passive: true, capture: true });
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

/// Dark mode for PDFs. WebView2's PDF viewer is a sealed frame that page
/// scripts and Dark Reader can't reach, so a click-through overlay below the
/// viewer's toolbar recolors what is behind it with an SVG filter:
///
/// - Neutral and pastel pixels (paper, text, rules, grey diagrams, light
///   highlights) are inverted with their hue kept, onto a soft dark page:
///   white paper becomes #1c1c1c, black text #e3e3e3.
/// - Vivid pixels (chart colors, colored figures) keep their own color, so a
///   yellow bar stays yellow instead of turning brown. In between, the mix
///   follows the color's strength.
/// - The viewer's own dark backdrop stays dark (told apart from #333 text by
///   shape: an erode + dilate keeps only wide areas).
///
/// Only acts on a top-level PDF document. `pdf_dark_js(on)` switches it.
pub const PDF_DARK_SCRIPT: &str = r#"(() => {
  if (window.top !== window || window.__sericaPdfDark) return;
  let on = false;
  const TOOLBAR = 41;
  // The viewer's backdrop is #333 (0.2). Text can be #333 too, so the backdrop
  // is told apart by shape: only #333 areas wider than a few px survive an
  // erode + dilate (text strokes do not), and those keep their own color.
  const BG_TABLE = Array.from({ length: 101 }, (_, i) => (i >= 19 && i <= 21 ? 1 : 0)).join(' ');
  const FILTER =
    '<svg width="0" height="0" style="position:absolute"><filter id="serica-pdf-dark-f" color-interpolation-filters="sRGB" x="0" y="0" width="100%" height="100%">'
    // Inverted luminance with hues kept, squeezed onto a soft dark range.
    + '<feColorMatrix in="SourceGraphic" type="matrix" values="-1 0 0 0 1 0 -1 0 0 1 0 0 -1 0 1 0 0 0 1 0" result="neg"/>'
    + '<feColorMatrix in="neg" type="hueRotate" values="180" result="negHue"/>'
    + '<feComponentTransfer in="negHue" result="dark">'
    + ['R', 'G', 'B'].map((c) => '<feFunc' + c + ' type="linear" slope="0.78" intercept="0.11"/>').join('')
    + '</feComponentTransfer>'
    // Color mask: (|R-G| + |G-B| + |B-R|) / 2 per pixel (the table below
    // yields 2|d|). Greys and pastel highlights (up to ~0.55, e.g. #ffffc0)
    // are inverted like paper, so their text stays readable; vivid colors
    // (0.8 and up) are kept as they are.
    + '<feColorMatrix in="SourceGraphic" type="matrix" values="1 -1 0 0 0.5 0 1 -1 0 0.5 -1 0 1 0 0.5 0 0 0 0 1" result="diff"/>'
    + '<feComponentTransfer in="diff" result="absdiff">'
    + ['R', 'G', 'B'].map((c) => '<feFunc' + c + ' type="table" tableValues="1 0 1"/>').join('')
    + '</feComponentTransfer>'
    + '<feColorMatrix in="absdiff" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0.5 0.5 0.5 0 0" result="satRaw"/>'
    + '<feComponentTransfer in="satRaw" result="mask"><feFuncA type="table" tableValues="0 0 0 0 0 0 0.3 0.65 1 1 1"/></feComponentTransfer>'
    // Colored parts from the original, the rest from the dark version.
    + '<feComposite in="SourceGraphic" in2="mask" operator="in" result="keep"/>'
    + '<feComposite in="dark" in2="mask" operator="out" result="inv"/>'
    + '<feComposite in="keep" in2="inv" operator="arithmetic" k1="0" k2="1" k3="1" k4="0" result="mixed"/>'
    // The viewer's own backdrop stays as it is.
    + '<feColorMatrix in="SourceGraphic" type="matrix" values="0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0.299 0.587 0.114 0 0" result="lum"/>'
    + '<feComponentTransfer in="lum" result="bgRaw"><feFuncA type="table" tableValues="' + BG_TABLE + '"/></feComponentTransfer>'
    + '<feMorphology in="bgRaw" operator="erode" radius="4" result="bgCore"/>'
    + '<feMorphology in="bgCore" operator="dilate" radius="4" result="bgMask"/>'
    + '<feComposite in="SourceGraphic" in2="bgMask" operator="in" result="bgKeep"/>'
    + '<feComposite in="mixed" in2="bgMask" operator="out" result="page"/>'
    + '<feComposite in="bgKeep" in2="page" operator="arithmetic" k1="0" k2="1" k3="1" k4="0"/>'
    + '</filter></svg>';
  const apply = () => {
    if (document.contentType !== 'application/pdf') return;
    const root = document.body || document.documentElement;
    if (!root) { document.addEventListener('DOMContentLoaded', apply, { once: true }); return; }
    let el = document.getElementById('serica-pdf-dark');
    if (!on) { if (el) el.remove(); return; }
    if (el) return;
    el = document.createElement('serica-pdf-dark');
    el.id = 'serica-pdf-dark';
    el.innerHTML = FILTER;
    root.appendChild(el);
    // The veil refers to the filter only once the filter is in the document:
    // a reference made in the same frame is not resolved and does nothing.
    const holder = el;
    requestAnimationFrame(() => {
      if (!on || !holder.isConnected) return;
      const veil = document.createElement('div');
      veil.style.cssText = 'position:fixed;left:0;right:0;top:' + TOOLBAR + 'px;bottom:0;pointer-events:none;z-index:2147483647;backdrop-filter:url(#serica-pdf-dark-f);';
      holder.appendChild(veil);
    });
  };
  Object.defineProperty(window, '__sericaPdfDark', { value: (v) => { on = !!v; apply(); } });
  Object.defineProperty(window, '__sericaPdfDarkOn', { get: () => on });
})();"#;

pub fn pdf_dark_js(on: bool) -> String {
    format!("window.__sericaPdfDark && window.__sericaPdfDark({on})")
}

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
    pub fn new(data_root: PathBuf, proxy_port: Option<u16>, net: (bool, bool), block_3pc: bool) -> Self {
        Self {
            views: HashMap::new(),
            #[cfg(windows)]
            clipper: crate::clip_win::Clipper::new(),
            bounds: None,
            visible: true,
            active: None,
            assistant: None,
            data_root,
            proxy_port,
            net,
            block_3pc,
            create_failures: 0,
            creating: std::collections::HashSet::new(),
            paused: std::collections::HashSet::new(),
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
        // Thin overlay scrollbars on pages (shown while scrolling), as in Edge
        // and Chrome on Windows 11, instead of the classic always-on track.
        args.push_str(" --enable-features=OverlayScrollbar,msOverlayScrollbarWinStyle:scrollbar_mode/full_mode,msOverlayScrollbarWinStyleAnimation");
        // Pages connect directly unless the VPN needs the tunnel: a proxy turns
        // off HTTP/3 and Chromium's own DNS and connection racing.
        if self.block_3pc {
            args.push_str(" --test-third-party-cookie-phaseout");
        }
        if let (true, Some(p)) = (self.net.0, self.proxy_port) {
            args.push_str(&format!(" --proxy-server=http://127.0.0.1:{p}"));
        }
        args
    }

    fn builder(&self, app: &Shared, label: String, url: url::Url, profile: &str, tab_id: Option<&str>) -> WebviewBuilder<tauri::Wry> {
        let mut b = WebviewBuilder::new(label, WebviewUrl::External(url))
            .data_directory(self.profile_dir(profile))
            .additional_browser_args(&self.browser_args())
            // Tauri's file-drop handler turns WebView2's own drop support off
            // and only reports file paths to the app: pages then could not take
            // dropped files (uploads), links or text, and HTML5 drag and drop
            // inside them broke. Pages handle drops themselves, as in Edge.
            .disable_drag_drop_handler()
            .background_color(tauri::webview::Color(255, 255, 255, 255))
            .browser_extensions_enabled(true)
            .initialization_script(PAGE_SCRIPT)
            .initialization_script(crate::extensions::WEBSTORE_SCRIPT)
            .initialization_script(SWIPE_SCRIPT)
            .initialization_script(PDF_DARK_SCRIPT);
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
    fn prepare(&self, app: &Shared, tab_id: &str, url: &str, profile: &str, private: bool) -> Option<PendingView> {
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
        // A private tab is an InPrivate page: its cookies, storage and cache
        // live in memory and are gone once the last private page closes.
        let builder = self.builder(app, label, start, profile, Some(tab_id)).incognito(private);
        Some(PendingView { window, builder, bounds, url: parsed })
    }

    /// Creates the webview. `add_child` blocks until the main thread has
    /// built it, and the main thread takes the host lock in its own callbacks
    /// (preview captures finishing, page events), so this must run without
    /// the host lock held, or both threads wait on each other forever.
    fn build(app: &Shared, tab_id: &str, profile: &str, p: PendingView) -> Option<Webview> {
        let PendingView { window, builder, bounds, url: parsed } = p;
        let _perf = crate::perf::span("build_webview(any thread)");
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
        let _perf = crate::perf::span("reconcile");
        struct Want {
            url: String,
            live: bool,
            profile: String,
            media: bool,
            zoom: f64,
            private: bool,
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
            (wants, s.active_tab_id.clone(), s.settings.auto_picture_in_picture, s.settings.text_scale)
        };
        let mut host = app.host.lock();

        // Close webviews whose tab is gone, asleep, unloaded, on an internal
        // page, or moved to another profile (which needs another data dir).
        let stale: Vec<String> = host
            .views
            .iter()
            .filter(|(id, v)| host.paused.contains(&v.profile) || wants.get(*id).map(|w| !w.live || w.profile != v.profile).unwrap_or(true))
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            if let Some(v) = host.views.remove(&id) {
                #[cfg(windows)]
                host.clipper.forget(v.webview.label());
                let _ = v.webview.close();
            }
            if !wants.contains_key(&id) {
                app.previews.remove(&id);
            }
        }

        let previous = host.active.clone();
        host.active = active.clone();
        let visible = host.visible;
        let bounds = host.bounds;
        let mut previews_to_capture = Vec::new();
        let mut to_suspend = Vec::new();
        // A page that comes on screen takes keyboard focus, as in other
        // browsers. Without this, closing the focused tab left focus nowhere
        // and shortcuts typed right after (Ctrl+T) were lost.
        let mut focus_page: Option<Webview> = None;
        let active_shown_before = active.as_ref().and_then(|id| host.views.get(id)).is_some_and(|v| v.shown);
        for (id, v) in host.views.iter_mut() {
            let should_show = visible && Some(id) == active.as_ref();
            if should_show {
                if let Some(b) = bounds {
                    Self::apply_bounds(&v.webview, b);
                }
                if !v.shown {
                    // Not when the page only comes back from under UI that
                    // covered it (menus, previews): moving focus then blurs the
                    // UI, which closes the auto-hide sidebar.
                    if previous.as_ref() != Some(id) || v.hidden_at.is_none() {
                        focus_page = Some(v.webview.clone());
                    }
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
        // Switched to a tab without a page on screen (new tab page, or a page
        // still being built): the UI takes focus so its shortcuts work.
        let focus_ui = visible && focus_page.is_none() && !active_shown_before && previous != active;
        #[cfg(windows)]
        Self::sync_clip_pages(&host, text_scale);
        #[cfg(not(windows))]
        let _ = text_scale;
        drop(host);
        if let Some(wv) = focus_page {
            let _ = wv.set_focus();
        } else if focus_ui {
            if let Some(main) = app.handle.get_webview("main") {
                let _ = main.set_focus();
            }
        }
        #[cfg(windows)]
        for id in to_suspend {
            Self::schedule_suspend(app, id);
        }
        #[cfg(not(windows))]
        let _ = to_suspend;
        #[cfg(windows)]
        for (id, wv) in previews_to_capture {
            let app = app.clone();
            crate::webview_win::capture(&wv, move |img| {
                if let Some(img) = img {
                    app.previews.put(&id, img);
                }
            });
        }
        #[cfg(not(windows))]
        let _ = previews_to_capture;

        // Build the active tab's webview last, once the page being left is
        // hidden: building takes a while (a slept tab waking up) and the old
        // page must not stay on screen under the new tab's address meanwhile.
        if let Some(id) = &active {
            if let Some(w) = wants.get(id) {
                if w.live && !is_internal_url(&w.url) && !app.host.lock().paused.contains(&w.profile) {
                    Self::create_active(app, id, w.url.clone(), w.profile.clone(), w.zoom, w.private);
                }
            }
        }
    }

    /// Builds the active tab's webview, then reconciles again to show it. The
    /// host lock is released while it is built, so another reconcile may be
    /// building it already.
    fn create_active(app: &Shared, id: &str, url: String, profile: String, zoom: f64, private: bool) {
        let mut host = app.host.lock();
        if host.views.contains_key(id) || host.creating.contains(id) {
            return;
        }
        let created = match host.prepare(app, id, &url, &profile, private) {
            Some(p) => {
                host.creating.insert(id.to_string());
                drop(host);
                let wv = Self::build(app, id, &profile, p);
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
        #[cfg(windows)]
        if (zoom - 1.0).abs() > 0.001 {
            crate::webview_win::set_zoom(&wv, zoom);
        }
        #[cfg(not(windows))]
        let _ = zoom;
        #[cfg(windows)]
        crate::webview_win::register_clip_window(&wv, host.clipper.clone());
        if let Some(old) = host.views.insert(id.to_string(), TabView { webview: wv, profile, shown: false, hidden_at: None }) {
            let _ = old.webview.close();
        }
        // The state was read before the lock was released for the build: a
        // reconcile that ran meanwhile (tab closed, slept or switched away)
        // could not see this view. Start over from fresh state, which shows
        // it (or closes it); the view exists now, so nothing is built again.
        drop(host);
        Self::reconcile(app);
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
            let low = crate::app::memory_load_percent().is_some_and(|l| l >= LOW_MEMORY_LOAD);
            let (trim_after, suspend_after) = if low { (TRIM_AFTER_LOW, SUSPEND_AFTER_LOW) } else { (TRIM_AFTER, SUSPEND_AFTER) };
            let slack = std::time::Duration::from_millis(500);
            tokio::time::sleep(trim_after + slack).await;
            let Some(wv) = still_hidden(trim_after) else { return };
            crate::webview_win::set_background(&wv, true);
            tokio::time::sleep(suspend_after - trim_after).await;
            if let Some(wv) = still_hidden(suspend_after) {
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

    /// Tells the clip worker which page windows are on screen and where the
    /// page area is (holes are in window coordinates).
    #[cfg(windows)]
    fn sync_clip_pages(host: &TabHost, z: f64) -> u64 {
        let pages = host.views.values().filter(|v| v.shown).map(|v| v.webview.label().to_string()).collect();
        let origin = host.bounds.map(|b| (b.x, b.y)).unwrap_or((0.0, 0.0));
        host.clipper.set_pages(pages, origin, z)
    }

    /// Applies the current holes to every page view on screen.
    pub fn apply_clips(app: &Shared) {
        #[cfg(windows)]
        {
            let z = app.store.lock().settings.text_scale;
            let host = app.host.lock();
            Self::sync_clip_pages(&host, z);
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
    /// Returns a sequence number to wait for with `wait_clip` (0 off Windows).
    #[allow(clippy::too_many_arguments)]
    pub fn set_hole(app: &Shared, id: String, rect: Hole, from: Option<Hole>, ms: u32, ease: [f64; 4], remove: bool, lead_ms: f64) -> u64 {
        #[cfg(windows)]
        {
            let clipper = app.host.lock().clipper.clone();
            clipper.set_hole(id, rect, from, ms, ease, remove, lead_ms)
        }
        #[cfg(not(windows))]
        {
            let _ = (app, id, rect, from, ms, ease, remove, lead_ms);
            0
        }
    }

    /// Blocks until hole change `seq` is on screen (or `timeout` passes).
    pub fn wait_clip(app: &Shared, seq: u64, timeout: std::time::Duration) {
        #[cfg(windows)]
        {
            let clipper = app.host.lock().clipper.clone();
            clipper.wait_applied(seq, timeout);
        }
        #[cfg(not(windows))]
        let _ = (app, seq, timeout);
    }

    /// Keeps a profile's pages closed (`true`) or lets them back (`false`).
    /// Closing every page of a profile ends its browser process, which
    /// releases files like its password store.
    pub fn pause_profile(app: &Shared, profile: &str, pause: bool) {
        {
            let mut host = app.host.lock();
            if pause {
                host.paused.insert(profile.to_string());
                if app.store.lock().active_profile_id() == profile {
                    if let Some((wv, _)) = host.assistant.take() {
                        let _ = wv.close();
                    }
                }
            } else {
                host.paused.remove(profile);
            }
        }
        Self::reconcile(app);
    }

    /// Opens a profile once, hidden, so WebView2 creates its data (password
    /// store, cookies). The caller closes the returned webview.
    pub fn probe_profile(app: &Shared, profile: &str) -> Option<Webview> {
        let (window, builder) = {
            let host = app.host.lock();
            let window = app.handle.get_window("main")?;
            let label = format!("probe-{}", LABEL_SEQ.fetch_add(1, Ordering::Relaxed));
            let builder = host.builder(app, label, url::Url::parse("about:blank").ok()?, profile, None);
            (window, builder)
        };
        // Never hold the host lock across `add_child` (see `TabHost::build`).
        let wv = window.add_child(builder, LogicalPosition::new(0.0, 0.0), LogicalSize::new(1.0, 1.0)).ok()?;
        let _ = wv.hide();
        Some(wv)
    }

    /// Recreates page webviews when third-party cookie blocking is switched:
    /// it is a browser argument, fixed per WebView2 environment, so open pages
    /// reload once.
    pub fn sync_cookie_policy(app: &Shared, block: bool) {
        {
            let mut host = app.host.lock();
            if host.block_3pc == block {
                return;
            }
            host.block_3pc = block;
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

    /// A page webview of `profile`, if one is open (cookies are per profile).
    pub fn profile_webview(app: &Shared, profile: &str) -> Option<Webview> {
        app.host.lock().views.values().find(|v| v.profile == profile).map(|v| v.webview.clone())
    }

    /// Every profile with an open page webview, with one of its webviews.
    pub fn profile_webviews(app: &Shared) -> Vec<(String, Webview)> {
        let host = app.host.lock();
        let mut seen = std::collections::HashSet::new();
        host.views.values().filter(|v| seen.insert(v.profile.clone())).map(|v| (v.profile.clone(), v.webview.clone())).collect()
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
                    app2.previews.put(&tab_id, img);
                }
            });
        });
    }

    /// Applies the dark-PDF setting to every open page (no-op on non-PDFs).
    pub fn apply_pdf_dark(app: &Shared) {
        for (tab_id, v) in app.host.lock().views.iter() {
            let _ = v.webview.eval(&pdf_dark_js(app.pdf_dark_for(tab_id)));
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
        #[cfg(windows)]
        crate::webview_win::attach_permissions(app, "assistant", &wv);
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
