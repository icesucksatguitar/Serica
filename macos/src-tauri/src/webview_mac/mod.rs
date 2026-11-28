//! WebKit (`WKWebView`) integration for tab webviews (macOS).
//!
//! The macOS counterpart of the Windows build's `webview_win.rs`. Tabs are
//! Tauri child webviews (wry's `WKWebView`); Serica builds each tab's
//! `WKWebViewConfiguration` itself (per-profile website data store, proxy,
//! user scripts, content rule lists, the `serica` message handler) and then
//! takes over the view's navigation and UI delegates. Wry's own delegates are
//! kept and called where Tauri relies on them (script queueing, page load
//! events, popups, the file picker).
//!
//! Everything here runs on the main thread. Handlers touch the store only
//! briefly (through `App`) and never take the host lock.

#![allow(non_snake_case)]
#![allow(clippy::too_many_arguments)]

use crate::app::{ui_emit, Shared, TabEvent};
use crate::mac_util::{js_string, ns, nsurl, on_main, post_main, responds, to_json, url_string, MainBox};
use crate::shields::hide_css;
use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, Bool, NSObject, ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, AnyThread, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::NSEvent;
use objc2_foundation::{
    ns_string, NSArray, NSDictionary, NSError, NSHTTPURLResponse, NSKeyValueChangeKey, NSKeyValueObservingOptions, NSNumber, NSObjectNSKeyValueCoding,
    NSObjectNSKeyValueObserverRegistration, NSObjectProtocol, NSString, NSURLAuthenticationChallenge, NSURLCredential, NSURLRequest, NSURLResponse,
    NSURLSessionAuthChallengeDisposition, NSURL, NSUUID,
};
use objc2_web_kit::{
    WKContentWorld, WKDownload, WKFrameInfo, WKMediaCaptureType, WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate,
    WKNavigationResponse, WKNavigationResponsePolicy, WKNavigationType, WKPermissionDecision, WKScriptMessage, WKScriptMessageHandlerWithReply,
    WKSecurityOrigin, WKUIDelegate, WKUserContentController, WKUserScript, WKUserScriptInjectionTime, WKWebView, WKWebViewConfiguration,
    WKWebpagePreferences, WKWebsiteDataStore, WKWindowFeatures,
};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;
use tauri::Webview;

// ------------------------------------------------------------------ context

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Tab,
    /// The AI assistant drawer: permissions, dialogs and Focus Mode only.
    Assistant,
}

/// What the page reported under the pointer at its last right-click.
#[derive(Clone, Debug, Default)]
struct CtxInfo {
    link: Option<String>,
    image: Option<String>,
    selection: Option<String>,
    editable: bool,
    page: String,
    /// Where the click was, in the top document's CSS px (top frame only).
    point: Option<(f64, f64)>,
}

#[derive(Default)]
struct State {
    /// URL of the current (or pending) top-level document: the "source" for adblock.
    top_url: String,
    /// Hosts whose cosmetic CSS / scriptlets are registered as user scripts
    /// (each checks its own hostname, so same-site navigations never race).
    cosmetic_hosts: HashSet<String>,
    /// Hosts whose scripts include scriptlets (must run before page scripts).
    scriptlet_hosts: HashSet<String>,
    /// Hosts this tab already reloaded once because the scriptlets missed the first load.
    reloaded_for: HashSet<String>,
    exceptions: HashSet<String>,
    generichide: bool,
    /// The webview starts on about:blank while its first page's scripts are
    /// registered; that blank page's events are not reported.
    booting: bool,
    /// A page Serica loads itself in place of the one asked for (the Focus
    /// block page, the error page): its navigation must not be checked again.
    allow_once: Option<String>,
    /// The scripts the controller held right after creation (Tauri's and
    /// Serica's own), put back when the per-host scripts are reset.
    base_scripts: Vec<Retained<WKUserScript>>,
    /// Frames from other sites that play media (the mini player reaches them).
    media_frames: Vec<Retained<WKFrameInfo>>,
    /// Frames that currently play, keyed by "origin|url".
    playing_frames: HashSet<String>,
    top_playing: bool,
    /// `_isPlayingAudio` could be observed: it reports audio for the page.
    native_audio: bool,
    last_ctx: Option<CtxInfo>,
    is_pdf: bool,
    crash_reloads: u32,
    /// The error page's one-time token for "Continue anyway", and its host.
    cert_nonce: Option<(String, String)>,
    /// The last find query (Command-G repeats it).
    find_query: Option<String>,
    /// The page whose source the tab shows (`view-source:`), if any.
    view_source: Option<String>,
    /// Blocked loads of the last moment: URL, rule list and when. WebKit
    /// reports a load once per list that matched it (the rules are split in
    /// chunks).
    recent_blocks: Vec<(String, String, std::time::Instant)>,
    /// HTML fullscreen (WebKit shows it in its own window).
    fullscreen: bool,
}

pub struct Ctx {
    app: Shared,
    pub tab_id: String,
    pub kind: Kind,
    pub profile: String,
    webview: Weak<WKWebView>,
    st: RefCell<State>,
    nav: RefCell<Option<Retained<SericaNavDelegate>>>,
    ui: RefCell<Option<Retained<SericaUIDelegate>>>,
    observer: RefCell<Option<Retained<SericaObserver>>>,
    observed: RefCell<Vec<&'static str>>,
}

thread_local! {
    /// Live tab contexts by WKWebView address. Main thread only.
    static CTXS: RefCell<HashMap<usize, Rc<Ctx>>> = RefCell::new(HashMap::new());
    /// Website data stores by profile (and `private:<profile>`).
    static STORES: RefCell<HashMap<String, Retained<WKWebsiteDataStore>>> = RefCell::new(HashMap::new());
    static WORLD: RefCell<Option<Retained<WKContentWorld>>> = const { RefCell::new(None) };
    static HANDLER: RefCell<Option<Retained<SericaScriptHandler>>> = const { RefCell::new(None) };
    /// Network setup applied to the data stores: (tunnel proxy port).
    static NET: Cell<Option<u16>> = const { Cell::new(None) };
    /// Hosts the user chose to visit despite a certificate error (this session).
    static CERT_EXCEPTIONS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn key(wv: &WKWebView) -> usize {
    wv as *const WKWebView as usize
}

fn ctx_of(wv: &WKWebView) -> Option<Rc<Ctx>> {
    CTXS.with(|c| c.borrow().get(&key(wv)).cloned())
}

/// The context of a tab's WKWebView, by tab id.
fn ctx_by_tab(tab_id: &str) -> Option<Rc<Ctx>> {
    CTXS.with(|c| c.borrow().values().find(|x| x.tab_id == tab_id && x.kind == Kind::Tab).cloned())
}

impl Ctx {
    fn webview(&self) -> Option<Retained<WKWebView>> {
        self.webview.load()
    }
}

/// The Serica content world: Serica's scripts and its message handler live
/// here, out of the page's reach.
fn world() -> Retained<WKContentWorld> {
    WORLD.with(|w| {
        w.borrow_mut()
            .get_or_insert_with(|| unsafe { WKContentWorld::worldWithName(&ns("serica"), MainThreadMarker::new().expect("main thread")) })
            .clone()
    })
}

fn page_world() -> Retained<WKContentWorld> {
    unsafe { WKContentWorld::pageWorld(MainThreadMarker::new().expect("main thread")) }
}

/// Runs `f` with the WKWebView of a Tauri webview, on the main thread.
pub fn with_wk(wv: &Webview, f: impl FnOnce(&WKWebView) + Send + 'static) {
    let _ = wv.with_webview(move |pw| {
        let ptr = pw.inner() as *const WKWebView;
        if ptr.is_null() {
            return;
        }
        // SAFETY: wry hands out its live WKWebView on the main thread.
        f(unsafe { &*ptr });
    });
}

// ------------------------------------------------------------------ data stores & network

/// The tunnel proxy pages use, if any (see `vpn.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Net {
    pub proxy_port: Option<u16>,
    pub webrtc_guard: bool,
}

/// A stable data store identifier for a profile (UUID v5 of its id).
pub fn store_uuid(profile: &str) -> uuid::Uuid {
    const NS: uuid::Uuid = uuid::uuid!("6f1d2a4c-58b3-4f7e-9d2a-5e7c1b3a9f40");
    uuid::Uuid::new_v5(&NS, format!("serica-profile:{profile}").as_bytes())
}

#[allow(non_camel_case_types)]
type nw_object = *mut NSObject;
#[link(name = "Network", kind = "framework")]
extern "C" {
    fn nw_endpoint_create_host(host: *const std::ffi::c_char, port: *const std::ffi::c_char) -> nw_object;
    fn nw_proxy_config_create_http_connect(endpoint: nw_object, tls_options: nw_object) -> nw_object;
    fn nw_proxy_config_add_excluded_domain(config: nw_object, domain: *const std::ffi::c_char);
}

/// Points a data store at the local tunnel proxy (`None`: direct). Public
/// API since macOS 14 (`WKWebsiteDataStore.proxyConfigurations`).
fn apply_proxy(store: &WKWebsiteDataStore, port: Option<u16>) {
    unsafe {
        let key = ns_string!("proxyConfigurations");
        if !responds(store, sel!(setProxyConfigurations:)) {
            tracing::warn!("this macOS has no per-webview proxy support; the privacy tunnel can't route pages");
            return;
        }
        match port {
            Some(p) => {
                let host = c"127.0.0.1";
                let port = std::ffi::CString::new(p.to_string()).unwrap_or_default();
                let endpoint = nw_endpoint_create_host(host.as_ptr(), port.as_ptr());
                if endpoint.is_null() {
                    return;
                }
                let config = nw_proxy_config_create_http_connect(endpoint, std::ptr::null_mut());
                // The endpoint was created +1; the config keeps its own reference.
                let _ = Retained::from_raw(endpoint);
                let Some(config) = Retained::from_raw(config) else { return };
                // Loopback stays direct, as with Chromium's --proxy-server.
                for d in [c"localhost", c"127.0.0.1", c"::1"] {
                    nw_proxy_config_add_excluded_domain(Retained::as_ptr(&config) as nw_object, d.as_ptr());
                }
                let list: Retained<NSArray<NSObject>> = NSArray::from_retained_slice(&[config]);
                store.setValue_forKey(Some(&list), key);
            }
            None => {
                let empty: Retained<NSArray<NSObject>> = NSArray::new();
                store.setValue_forKey(Some(&empty), key);
            }
        }
    }
}

/// The website data store of a profile (persistent, by identifier), or the
/// profile's shared in-memory store for private tabs.
fn data_store(profile: &str, private: bool) -> Retained<WKWebsiteDataStore> {
    let mtm = MainThreadMarker::new().expect("main thread");
    let key = if private { format!("private:{profile}") } else { profile.to_string() };
    if let Some(s) = STORES.with(|s| s.borrow().get(&key).cloned()) {
        return s;
    }
    let store = unsafe {
        if private {
            WKWebsiteDataStore::nonPersistentDataStore(mtm)
        } else {
            let id = NSUUID::from_bytes(*store_uuid(profile).as_bytes());
            WKWebsiteDataStore::dataStoreForIdentifier(&id, mtm)
        }
    };
    apply_proxy(&store, NET.with(|n| n.get()));
    STORES.with(|s| s.borrow_mut().insert(key, store.clone()));
    store
}

/// New tunnel setup: every store routes through the proxy (or not) from now
/// on; open pages are recreated by the caller (as on Windows).
pub fn apply_network(handle: &tauri::AppHandle, port: Option<u16>) {
    let _ = on_main(handle, Duration::from_secs(3), move || {
        NET.with(|n| n.set(port));
        STORES.with(|s| {
            for st in s.borrow().values() {
                apply_proxy(st, port);
            }
        });
    });
}

/// Private tabs share one in-memory store per profile; it is let go once no
/// private page of that profile is open, which forgets everything in it.
pub fn release_private_stores(handle: &tauri::AppHandle, keep: HashSet<String>) {
    post_main(handle, move || {
        STORES.with(|s| s.borrow_mut().retain(|k, _| k.strip_prefix("private:").map(|p| keep.contains(p)).unwrap_or(true)));
    });
}

/// Deletes a removed profile's cookies and site data.
pub fn remove_profile_data(handle: &tauri::AppHandle, profile: &str) {
    let profile = profile.to_string();
    post_main(handle, move || unsafe {
        STORES.with(|s| s.borrow_mut().remove(&profile));
        let mtm = MainThreadMarker::new().expect("main thread");
        let id = NSUUID::from_bytes(*store_uuid(&profile).as_bytes());
        let done = RcBlock::new(|err: *mut NSError| {
            if !err.is_null() {
                tracing::warn!("could not remove a profile's data store: {}", (*err).localizedDescription());
            }
        });
        WKWebsiteDataStore::removeDataStoreForIdentifier_completionHandler(&id, &done, mtm);
    });
}

/// Third-party cookies: WebKit's Intelligent Tracking Prevention blocks them
/// by default. "Allow" turns tracking prevention off for the profile's store
/// (so embedded sites get their cookies again, as in Chromium); "block"
/// keeps it on.
fn apply_cookie_policy(store: &WKWebsiteDataStore, block_3pc: bool) {
    unsafe {
        if responds(store, sel!(_setResourceLoadStatisticsEnabled:)) {
            let _: () = msg_send![store, _setResourceLoadStatisticsEnabled: Bool::new(block_3pc)];
        }
        if responds(store, sel!(_setThirdPartyCookieBlockingMode:)) {
            // _WKThirdPartyCookieBlockingMode: 0 = all, 1 = all except between same-site
            // (when ITP is on), 2 = only for ITP-classified domains.
            let mode: usize = if block_3pc { 0 } else { 2 };
            let done = RcBlock::new(|| {});
            let _: () = msg_send![store, _setThirdPartyCookieBlockingMode: mode, completionHandler: &*done];
        }
    }
}

thread_local! {
    static BLOCK_3PC: Cell<bool> = const { Cell::new(false) };
}

pub fn set_cookie_policy(handle: &tauri::AppHandle, block_3pc: bool) {
    let _ = on_main(handle, Duration::from_secs(3), move || {
        BLOCK_3PC.with(|b| b.set(block_3pc));
        STORES.with(|s| {
            for st in s.borrow().values() {
                apply_cookie_policy(st, block_3pc);
            }
        });
    });
}

// ------------------------------------------------------------------ configuration

/// The Safari version that matches this Mac's WebKit, for the user agent.
fn safari_version() -> String {
    static V: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        let from_plist = plist::Value::from_file("/Applications/Safari.app/Contents/Info.plist")
            .ok()
            .and_then(|v| v.as_dictionary().and_then(|d| d.get("CFBundleShortVersionString")).and_then(|s| s.as_string().map(str::to_string)));
        from_plist.unwrap_or_else(|| {
            let (major, minor, _) = crate::mac_util::os_version();
            match major {
                14 => format!("17.{minor}"),
                15 => format!("18.{minor}"),
                m if m >= 26 => format!("{m}.{minor}"),
                _ => "17.0".to_string(),
            }
        })
    })
    .clone()
}

/// What a page user script needs to know.
struct Script<'a> {
    source: &'a str,
    all_frames: bool,
    page_world: bool,
}

fn user_script(s: &Script) -> Retained<WKUserScript> {
    let mtm = MainThreadMarker::new().expect("main thread");
    let world = if s.page_world { page_world() } else { world() };
    unsafe {
        WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
            mtm.alloc::<WKUserScript>(),
            &ns(s.source),
            WKUserScriptInjectionTime::AtDocumentStart,
            !s.all_frames,
            &world,
        )
    }
}

/// Builds a page's configuration on the main thread (WebKit objects live
/// there). `None` if the main thread did not answer (the app is quitting).
pub fn configuration(app: &Shared, profile: &str, private: bool, net: Net, kind: Kind) -> Option<MainBox<Retained<WKWebViewConfiguration>>> {
    let (app2, profile) = (app.clone(), profile.to_string());
    on_main(&app.handle, Duration::from_secs(5), move || MainBox(build_configuration(&app2, &profile, private, net, kind)))
}

fn build_configuration(app: &Shared, profile: &str, private: bool, net: Net, kind: Kind) -> Retained<WKWebViewConfiguration> {
    let mtm = MainThreadMarker::new().expect("main thread");
    unsafe {
        let config = WKWebViewConfiguration::new(mtm);
        let store = data_store(profile, private);
        apply_cookie_policy(&store, BLOCK_3PC.with(|b| b.get()));
        config.setWebsiteDataStore(&store);
        let ucc = WKUserContentController::new(mtm);
        config.setUserContentController(&ucc);
        // Looks like the Safari of this Mac: sites (Google sign-in above all)
        // refuse or degrade unknown WebKit browsers.
        config.setApplicationNameForUserAgent(Some(&ns(&format!("Version/{} Safari/605.1.15", safari_version()))));
        let prefs = config.preferences();
        let yes = NSNumber::numberWithBool(true);
        prefs.setValue_forKey(Some(&yes), ns_string!("developerExtrasEnabled"));
        prefs.setValue_forKey(Some(&yes), ns_string!("allowsPictureInPictureMediaPlayback"));
        prefs.setValue_forKey(Some(&yes), ns_string!("tabFocusesLinks"));
        if responds(&*prefs, sel!(setElementFullscreenEnabled:)) {
            prefs.setElementFullscreenEnabled(true);
        } else {
            prefs.setValue_forKey(Some(&yes), ns_string!("fullScreenEnabled"));
        }
        // Hidden tabs are throttled (timers, rendering) by WebKit itself.
        if responds(&*prefs, sel!(setInactiveSchedulingPolicy:)) {
            prefs.setInactiveSchedulingPolicy(objc2_web_kit::WKInactiveSchedulingPolicy::Throttle);
        }
        prefs.setJavaScriptCanOpenWindowsAutomatically(false);

        // Serica's scripts (see mac_scripts.rs). The WebRTC guard and
        // per-site scriptlets patch the page's own objects: page world.
        let page = crate::mac_scripts::page_script();
        let frame = crate::mac_scripts::frame_script();
        let cert = crate::mac_scripts::cert_bridge_script();
        let mut scripts: Vec<Script> = vec![Script { source: &frame, all_frames: true, page_world: false }];
        if kind == Kind::Tab {
            scripts.push(Script { source: &page, all_frames: false, page_world: false });
            scripts.push(Script { source: crate::extensions::WEBSTORE_SCRIPT, all_frames: false, page_world: false });
            scripts.push(Script { source: crate::mac_scripts::PDF_DARK_SCRIPT, all_frames: false, page_world: false });
            scripts.push(Script { source: crate::mac_scripts::FIND_SCRIPT, all_frames: false, page_world: false });
            scripts.push(Script { source: &cert, all_frames: false, page_world: false });
        }
        if net.webrtc_guard {
            scripts.push(Script { source: crate::vpn::WEBRTC_GUARD_JS, all_frames: true, page_world: true });
        }
        for s in &scripts {
            ucc.addUserScript(&user_script(s));
        }
        crate::passwords::install(&ucc, kind == Kind::Tab && !private);
        if kind == Kind::Tab {
            // Extensions: WebKit's extension runtime, or their content scripts.
            crate::extensions::webkit::configure(app, &config, &ucc);
            // Shields and Focus Mode request blocking (WKContentRuleList).
            crate::content_rules::attach(&ucc);
        } else {
            crate::content_rules::attach_focus_only(&ucc);
        }
        let handler = HANDLER.with(|h| h.borrow_mut().get_or_insert_with(|| SericaScriptHandler::new(mtm)).clone());
        ucc.addScriptMessageHandlerWithReply_contentWorld_name(ProtocolObject::from_ref(&*handler), &world(), &ns("serica"));
        config
    }
}

// ------------------------------------------------------------------ parts

mod delegates;
mod dialogs;
mod messages;
mod nav;
mod popup;
pub mod api;
pub mod menu;

use delegates::{SericaNavDelegate, SericaObserver, SericaScriptHandler, SericaUIDelegate};
pub use api::*;

/// Loads a URL in a page view the way the address bar does.
pub fn nav_load(wv: &WKWebView, url: &str) {
    nav::load_url(wv, url);
}
pub use dialogs::{answer_dialog, answer_permission};
pub use menu::finish_page_menu;
