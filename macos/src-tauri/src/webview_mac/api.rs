//! What the rest of the backend calls on tab webviews (the macOS
//! counterparts of `webview_win`'s functions).

use super::*;
use objc2_foundation::NSHTTPCookie;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

// ------------------------------------------------------------------ attach / detach

/// Hooks a new page webview. It was created on about:blank, so every
/// handler is in place (and the first page's filters registered) before the
/// real page `first_url` loads.
pub fn attach(app: &Shared, tab_id: &str, profile: &str, wv: &Webview, first_url: Option<String>, kind: Kind, private: bool) {
    let (app, tab_id, profile) = (app.clone(), tab_id.to_string(), profile.to_string());
    let _ = wv.with_webview(move |pw| {
        let ptr = pw.inner() as *const WKWebView;
        if ptr.is_null() {
            return;
        }
        // SAFETY: wry's live WKWebView, on the main thread.
        let wk = unsafe { &*ptr };
        install(app, tab_id, profile, wk, first_url, kind, private);
    });
}

fn install(app: Shared, tab_id: String, profile: String, wk: &WKWebView, first_url: Option<String>, kind: Kind, private: bool) {
    let mtm = MainThreadMarker::new().expect("main thread");
    unsafe {
        let ucc = wk.configuration().userContentController();
        let base = ucc.userScripts().to_vec();
        let ctx = Rc::new(Ctx {
            app: app.clone(),
            tab_id: tab_id.clone(),
            kind,
            profile: profile.clone(),
            webview: Weak::from(wk),
            st: RefCell::new(State { booting: first_url.is_some(), base_scripts: base, ..Default::default() }),
            nav: RefCell::new(None),
            ui: RefCell::new(None),
            observer: RefCell::new(None),
            observed: RefCell::new(Vec::new()),
        });
        let nav = SericaNavDelegate::new(mtm, wk.navigationDelegate());
        wk.setNavigationDelegate(Some(ProtocolObject::from_ref(&*nav)));
        let ui = SericaUIDelegate::new(mtm, wk.UIDelegate());
        wk.setUIDelegate(Some(ProtocolObject::from_ref(&*ui)));
        *ctx.nav.borrow_mut() = Some(nav);
        *ctx.ui.borrow_mut() = Some(ui);
        // Serica's view subclass first: key-value observing subclasses the
        // instance too, and must do so on top of it.
        crate::mac_tabview::adopt(wk);
        let obs = SericaObserver::new(mtm);
        *ctx.observer.borrow_mut() = Some(obs.clone());
        super::delegates::observe_all(&ctx, wk, &obs);
        // Two-finger swipes go back and forward, as in Safari; pinch zooms.
        wk.setAllowsBackForwardNavigationGestures(kind == Kind::Tab);
        wk.setAllowsMagnification(kind == Kind::Tab);
        if responds(wk, sel!(setInspectable:)) {
            wk.setInspectable(true);
        }
        if std::env::var_os("SERICA_TEST_MUTE").is_some() {
            mute(wk, true);
        }
        CTXS.with(|c| c.borrow_mut().insert(key(wk), ctx.clone()));
        if kind == Kind::Tab {
            crate::extensions::webkit_tabs::tab_opened(&tab_id, private);
        }

        let Some(url) = first_url else { return };
        let weak = Weak::from(wk);
        let start = {
            let ctx = ctx.clone();
            move || {
                ctx.st.borrow_mut().booting = false;
                if let Some(wk) = weak.load() {
                    super::nav::load_url(&wk, &url);
                }
            }
        };
        // "Clear cookies when Serica closes": also done before the profile's
        // first page of a session, in case it could not be done at exit.
        let first_of_session = !private && app.claim_cookie_sweep(&profile);
        let s = app.settings();
        if s.clear_cookies_on_exit && first_of_session {
            let keep = s.cookie_keep_sites.clone();
            sweep_main(&profile, move |domain| !cookie_in_sites(domain, &keep), move |n| {
                if n > 0 {
                    tracing::info!("cleared {n} cookies at start");
                }
                start();
            });
        } else {
            start();
        }
    }
}

/// Unhooks a page webview that is about to close: its observers go, and
/// whatever it still waits on is answered (dialogs, permissions).
pub fn detach(wv: &Webview) {
    with_wk(wv, |wk| {
        let Some(ctx) = CTXS.with(|c| c.borrow_mut().remove(&key(wk))) else { return };
        if ctx.kind == Kind::Tab {
            crate::extensions::webkit_tabs::tab_closed(&ctx.tab_id);
        }
        super::delegates::unobserve_all(&ctx, wk);
        super::dialogs::fail_pending(Some(&ctx.tab_id));
        super::menu::forget(&ctx.tab_id);
        crate::clip_mac::forget(wk);
    });
}

/// A live page view to run work in: the tab's own, else any tab's.
pub fn any_tab_view(tab_id: &str) -> Option<Retained<WKWebView>> {
    ctx_by_tab(tab_id)
        .and_then(|c| c.webview())
        .or_else(|| CTXS.with(|c| c.borrow().values().filter(|x| x.kind == Kind::Tab).find_map(|x| x.webview())))
}

/// The page view of exactly this tab.
pub fn tab_view(tab_id: &str) -> Option<Retained<WKWebView>> {
    ctx_by_tab(tab_id).and_then(|c| c.webview())
}

/// Answers everything still pending in every page (Serica is quitting).
pub fn fail_all_pending() {
    super::dialogs::fail_pending(None);
}

// ------------------------------------------------------------------ scripts

/// Runs `js` in Serica's content world of the page's top frame.
pub fn eval_world(wk: &WKWebView, js: &str) {
    unsafe {
        wk.evaluateJavaScript_inFrame_inContentWorld_completionHandler(&ns(js), None, &world(), None);
    }
}

fn eval_world_cb(wk: &WKWebView, js: &str, done: impl Fn(serde_json::Value) + 'static) {
    let blk = RcBlock::new(move |result: *mut AnyObject, _err: *mut NSError| done(to_json(unsafe { result.as_ref() })));
    unsafe {
        wk.evaluateJavaScript_inFrame_inContentWorld_completionHandler(&ns(js), None, &world(), Some(&blk));
    }
}

/// `eval_world` for a Tauri webview.
pub fn eval_serica(wv: &Webview, js: String) {
    with_wk(wv, move |wk| eval_world(wk, &js));
}

pub fn apply_pdf_dark(wv: &Webview, on: bool) {
    eval_serica(wv, crate::mac_scripts::pdf_dark_js(on));
}

// ------------------------------------------------------------------ capture

/// Captures the page as it is on screen, as JPEG bytes (Retina resolution).
/// The encoding runs off the main thread.
pub fn capture(wv: &Webview, done: impl FnOnce(Option<Vec<u8>>) + Send + 'static) {
    let done: Arc<Mutex<Option<Box<dyn FnOnce(Option<Vec<u8>>) + Send>>>> = Arc::new(Mutex::new(Some(Box::new(done))));
    let d2 = done.clone();
    let r = wv.with_webview(move |pw| unsafe {
        let _perf = crate::perf::span("main.capture_start");
        let finish = move |v: Option<Vec<u8>>| {
            if let Some(f) = d2.lock().ok().and_then(|mut d| d.take()) {
                f(v);
            }
        };
        let ptr = pw.inner() as *const WKWebView;
        if ptr.is_null() {
            return finish(None);
        }
        let wk = &*ptr;
        if wk.isHidden() || wk.window().is_none() {
            return finish(None);
        }
        let mtm = MainThreadMarker::new().expect("main thread");
        let cfg = objc2_web_kit::WKSnapshotConfiguration::new(mtm);
        cfg.setAfterScreenUpdates(false);
        let finish = Arc::new(Mutex::new(Some(finish)));
        let blk = RcBlock::new(move |image: *mut objc2_app_kit::NSImage, _err: *mut NSError| {
            let Some(f) = finish.lock().ok().and_then(|mut f| f.take()) else { return };
            let Some(img) = image.as_ref() else { return f(None) };
            let Some(cg) = img.CGImageForProposedRect_context_hints(std::ptr::null_mut(), None, None) else { return f(None) };
            let cg = MainBox(cg);
            std::thread::spawn(move || {
                let cg = cg;
                f(encode_jpeg(&cg.0));
            });
        });
        wk.takeSnapshotWithConfiguration_completionHandler(Some(&cfg), &blk);
    });
    if r.is_err() {
        if let Some(f) = done.lock().ok().and_then(|mut d| d.take()) {
            f(None);
        }
    }
}

fn encode_jpeg(cg: &objc2_core_graphics::CGImage) -> Option<Vec<u8>> {
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImageCompressionFactor};
    unsafe {
        let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), cg);
        let factor = NSNumber::numberWithDouble(0.82);
        let props = NSDictionary::from_slices(&[NSImageCompressionFactor], &[&*factor as &AnyObject]);
        let data = rep.representationUsingType_properties(NSBitmapImageFileType::JPEG, &props)?;
        let bytes = data.to_vec();
        (!bytes.is_empty()).then_some(bytes)
    }
}

// ------------------------------------------------------------------ find in page

/// Starts (`query`), continues (`next`) or stops (neither) find in page; the
/// count and the active match go to the UI's find bar.
pub fn find(app: &Shared, wv: &Webview, tab_id: &str, query: Option<String>, forward: bool, next: bool) {
    let (app, id) = (app.clone(), tab_id.to_string());
    with_wk(wv, move |wk| {
        let Some(ctx) = ctx_of(wk) else { return };
        let js = if next {
            let q = query.or_else(|| ctx.st.borrow().find_query.clone());
            let Some(q) = q else { return };
            ctx.st.borrow_mut().find_query = Some(q.clone());
            format!(
                "(() => {{ const f = window.__sericaFind; if (!f) return null; const q = {}; return f.query() === q ? f.next({forward}) : f.find(q); }})()",
                js_string(&q)
            )
        } else if let Some(q) = query {
            ctx.st.borrow_mut().find_query = Some(q.clone());
            format!("(() => {{ const f = window.__sericaFind; return f ? f.find({}) : null; }})()", js_string(&q))
        } else {
            ctx.st.borrow_mut().find_query = None;
            eval_world(wk, "window.__sericaFind && window.__sericaFind.clear()");
            return;
        };
        eval_world_cb(wk, &js, move |v| {
            let total = v["total"].as_i64().unwrap_or(0).max(0);
            let active = v["active"].as_i64().unwrap_or(0).max(0);
            ui_emit(&app.handle, "serica:find-result", serde_json::json!({ "tabId": id, "totalMatches": total, "activeMatch": active }));
        });
    });
}

// ------------------------------------------------------------------ media

type MediaDone = Rc<RefCell<Option<Box<dyn FnOnce(Option<serde_json::Value>)>>>>;

/// Runs a media script (see `tabhost::pip_enter_js`, `media_state_js`,
/// `media_action_js`) in the tab's page, then in each frame from another
/// site that plays media, until one returns something other than
/// null/false, and passes that on. WebKit runs these as if the user had just
/// clicked, which picture-in-picture needs.
pub fn media_eval(wv: &Webview, _tab_id: String, js: String, _follow: bool, done: impl FnOnce(Option<serde_json::Value>) + Send + 'static) {
    let done = Arc::new(Mutex::new(Some(done)));
    let d2 = done.clone();
    let r = wv.with_webview(move |pw| {
        let finish: MediaDone = Rc::new(RefCell::new(Some(Box::new(move |v| {
            if let Some(f) = d2.lock().ok().and_then(|mut d| d.take()) {
                f(v);
            }
        }))));
        let ptr = pw.inner() as *const WKWebView;
        if ptr.is_null() {
            if let Some(f) = finish.borrow_mut().take() {
                f(None);
            }
            return;
        }
        let wk: Retained<WKWebView> = unsafe { Message::retain(&*ptr) };
        let frames = Rc::new(ctx_of(&wk).map(|c| c.st.borrow().media_frames.clone()).unwrap_or_default());
        eval_media_at(wk, Rc::new(js), frames, 0, finish);
    });
    if r.is_err() {
        if let Some(f) = done.lock().ok().and_then(|mut d| d.take()) {
            f(None);
        }
    }
}

fn eval_media_at(wk: Retained<WKWebView>, js: Rc<String>, frames: Rc<Vec<Retained<WKFrameInfo>>>, i: usize, finish: MediaDone) {
    let body = format!("return await ({});", js);
    let next = (wk.clone(), js.clone(), frames.clone(), finish.clone());
    let blk = RcBlock::new(move |result: *mut AnyObject, _err: *mut NSError| {
        let value = to_json(unsafe { result.as_ref() });
        let found = !value.is_null() && value != serde_json::Value::Bool(false);
        let (wk, js, frames, finish) = next.clone();
        if found || i >= frames.len() {
            if let Some(f) = finish.borrow_mut().take() {
                f((!value.is_null()).then_some(value));
            }
        } else {
            eval_media_at(wk, js, frames, i + 1, finish);
        }
    });
    let frame = if i == 0 { None } else { frames.get(i - 1).map(|f| &**f) };
    unsafe {
        wk.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(&ns(&body), None, frame, &world(), Some(&blk));
    }
}

fn mute(wk: &WKWebView, muted: bool) {
    unsafe {
        if responds(wk, sel!(_setPageMuted:)) {
            // _WKMediaMutedState: audio muted = 1 << 0.
            let state: usize = if muted { 1 } else { 0 };
            let _: () = msg_send![wk, _setPageMuted: state];
        } else {
            let js = format!("document.querySelectorAll('video,audio').forEach((m) => {{ m.muted = {muted}; }})");
            eval_world(wk, &js);
        }
    }
}

pub fn set_muted(wv: &Webview, muted: bool) {
    with_wk(wv, move |wk| mute(wk, muted));
}

// ------------------------------------------------------------------ navigation, zoom, print

pub fn nav(wv: &Webview, action: &'static str) {
    with_wk(wv, move |wk| unsafe {
        let _ = match action {
            "back" => wk.goBack(),
            "forward" => wk.goForward(),
            "stop" => {
                wk.stopLoading();
                None
            }
            "reload" => wk.reload(),
            "hard-reload" => wk.reloadFromOrigin(),
            _ => None,
        };
    });
}

/// Loads `url` in a page webview (file URLs get read access to their folder).
pub fn load(wv: &Webview, url: &str) {
    let url = url.to_string();
    with_wk(wv, move |wk| super::nav::load_url(wk, &url));
}

pub fn set_zoom(wv: &Webview, factor: f64) {
    with_wk(wv, move |wk| unsafe {
        if responds(wk, sel!(setPageZoom:)) {
            wk.setPageZoom(factor);
        }
    });
}

pub fn print(wv: &Webview) {
    with_wk(wv, |wk| unsafe {
        let Some(window) = wk.window() else { return };
        let info = objc2_app_kit::NSPrintInfo::sharedPrintInfo();
        let op = wk.printOperationWithPrintInfo(&info);
        op.setShowsPrintPanel(true);
        op.setShowsProgressPanel(true);
        op.setCanSpawnSeparateThread(true);
        op.runOperationModalForWindow_delegate_didRunSelector_contextInfo(&window, None, None, std::ptr::null_mut());
    });
}

/// Hidden tabs: WebKit throttles their timers and painting by itself
/// (`inactiveSchedulingPolicy`); a tab hidden long enough is suspended,
/// the counterpart of WebView2's `TrySuspend`.
pub fn set_suspended(wv: &Webview, suspended: bool) {
    with_wk(wv, move |wk| unsafe {
        let prefs = wk.configuration().preferences();
        if responds(&*prefs, sel!(setInactiveSchedulingPolicy:)) {
            prefs.setInactiveSchedulingPolicy(if suspended {
                objc2_web_kit::WKInactiveSchedulingPolicy::Suspend
            } else {
                objc2_web_kit::WKInactiveSchedulingPolicy::Throttle
            });
        }
    });
}

// ------------------------------------------------------------------ cookies
//
// Cookies belong to a profile's website data store, reachable whether or
// not a page of the profile is open.

/// A cookie's site, as shown and matched: its domain without a leading dot
/// or `www.`.
pub fn cookie_site(domain: &str) -> String {
    let d = domain.trim_start_matches('.').to_ascii_lowercase();
    d.strip_prefix("www.").map(str::to_string).unwrap_or(d)
}

/// Whether a cookie for `domain` belongs to one of `sites` (a site keeps its
/// subdomains: `google.com` keeps `accounts.google.com`).
pub fn cookie_in_sites(domain: &str, sites: &[String]) -> bool {
    let d = cookie_site(domain);
    sites.iter().any(|s| {
        let s = cookie_site(s);
        !s.is_empty() && (d == s || d.ends_with(&format!(".{s}")))
    })
}

fn with_cookies(profile: &str, f: impl FnOnce(&objc2_web_kit::WKHTTPCookieStore, Vec<Retained<NSHTTPCookie>>) + 'static) {
    unsafe {
        let store = data_store(profile, false);
        let cs = store.httpCookieStore();
        let cs2 = cs.clone();
        let f = Cell::new(Some(f));
        let blk = RcBlock::new(move |list: NonNull<NSArray<NSHTTPCookie>>| {
            let cookies = list.as_ref().to_vec();
            if let Some(f) = f.take() {
                f(&cs2, cookies);
            }
        });
        cs.getAllCookies(&blk);
    }
}

/// Sites with cookies in a profile and how many each has, most first.
pub fn cookie_sites(handle: &tauri::AppHandle, profile: &str, done: impl FnOnce(Vec<(String, usize)>) + Send + 'static) {
    let profile = profile.to_string();
    post_main(handle, move || {
        with_cookies(&profile, move |_, cookies| {
            let mut by_site: HashMap<String, usize> = HashMap::new();
            for c in &cookies {
                *by_site.entry(cookie_site(&c.domain().to_string())).or_default() += 1;
            }
            let mut v: Vec<(String, usize)> = by_site.into_iter().collect();
            v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            done(v);
        });
    });
}

/// Deletes the cookies `delete` picks (by domain), on the main thread, then runs `done`.
fn sweep_main(profile: &str, delete: impl Fn(&str) -> bool + 'static, done: impl FnOnce(usize) + 'static) {
    with_cookies(profile, move |cs, cookies| {
        let doomed: Vec<Retained<NSHTTPCookie>> = cookies.into_iter().filter(|c| delete(&c.domain().to_string())).collect();
        let n = doomed.len();
        if n == 0 {
            return done(0);
        }
        let left = Rc::new(Cell::new(n));
        let done = Rc::new(Cell::new(Some(done)));
        for c in doomed {
            let (left, done) = (left.clone(), done.clone());
            let blk = RcBlock::new(move || {
                left.set(left.get().saturating_sub(1));
                if left.get() == 0 {
                    if let Some(d) = done.take() {
                        d(n);
                    }
                }
            });
            unsafe { cs.deleteCookie_completionHandler(&c, Some(&blk)) };
        }
    });
}

/// Deletes the cookies of `profile` that `delete` picks; reports how many went.
pub fn delete_cookies(handle: &tauri::AppHandle, profile: &str, delete: impl Fn(&str) -> bool + Send + 'static, done: impl FnOnce(usize) + Send + 'static) {
    let profile = profile.to_string();
    post_main(handle, move || sweep_main(&profile, delete, done));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_sites_match_subdomains() {
        assert_eq!(cookie_site(".www.Example.com"), "example.com");
        assert!(cookie_in_sites("accounts.google.com", &["google.com".into()]));
        assert!(!cookie_in_sites("notgoogle.com", &["google.com".into()]));
        assert!(!cookie_in_sites("a.test", &["".into()]));
    }

    #[test]
    fn profile_store_ids_are_stable_and_distinct() {
        assert_eq!(store_uuid("default"), store_uuid("default"));
        assert_ne!(store_uuid("default"), store_uuid("work"));
        assert_eq!(store_uuid("default").get_version_num(), 5);
    }
}

/// Swaps the extension content scripts of every open tab for the current set
/// (before macOS 15.4, when extensions change); pages pick them up on their
/// next load. Main thread.
pub fn replace_extension_scripts(app: &Shared) {
    let ctxs: Vec<Rc<Ctx>> = CTXS.with(|c| c.borrow().values().filter(|x| x.kind == Kind::Tab).cloned().collect());
    if ctxs.is_empty() {
        return;
    }
    let mark = crate::extensions::webkit::SCRIPT_MARK;
    for ctx in ctxs {
        let Some(wk) = ctx.webview() else { continue };
        unsafe {
            let ucc = wk.configuration().userContentController();
            let mut st = ctx.st.borrow_mut();
            st.base_scripts.retain(|s| !s.source().to_string().starts_with(mark));
            st.base_scripts.extend(crate::extensions::webkit::user_scripts(app));
            ucc.removeAllUserScripts();
            for s in &st.base_scripts {
                ucc.addUserScript(s);
            }
            // Site scripts are put back on the next navigation.
            st.cosmetic_hosts.clear();
            st.scriptlet_hosts.clear();
        }
    }
}
