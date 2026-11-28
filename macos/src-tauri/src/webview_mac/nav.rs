//! Navigation: Focus Mode, external apps, downloads, Shields per page,
//! cosmetic filters, error pages, and the page state the UI shows.

use super::*;
use std::ptr::NonNull;

/// Schemes WebKit loads in the page itself; anything else belongs to an app.
fn is_web_scheme(scheme: &str) -> bool {
    matches!(
        scheme,
        "http" | "https" | "about" | "blob" | "data" | "file" | "javascript" | "view-source" | "sericabg" | "sericacap" | "tauri" | "ipc" | "webkit-extension" | "applewebdata"
    )
}

fn host_of(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default()
}

fn origin_of(uri: &str) -> String {
    url::Url::parse(uri)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string()))
        .unwrap_or_else(|| uri.to_string())
}

fn wv_url(wv: &WKWebView) -> String {
    unsafe { url_string(wv.URL().as_deref()) }
}

/// Tells extensions (WebKit's runtime) a tab changed.
fn tab_changed(ctx: &Ctx, props: objc2_web_kit::WKWebExtensionTabChangedProperties) {
    if ctx.kind == Kind::Tab {
        crate::extensions::webkit_tabs::tab_changed(&ctx.tab_id, props);
    }
}

/// The address the tab shows: `view-source:` while the source viewer is up.
fn shown_url(ctx: &Ctx, url: String) -> String {
    match ctx.st.borrow().view_source.as_deref() {
        Some(v) if v == url => format!("view-source:{url}"),
        _ => url,
    }
}

/// Shows the source of `target` (WebKit has no `view-source:`): Serica's
/// viewer, loaded as `target` itself so it reads the source with the page's
/// cookies.
fn show_source(ctx: &Ctx, wv: &WKWebView, target: &str) {
    ctx.st.borrow_mut().view_source = Some(target.to_string());
    load_in_place(ctx, wv, target, &crate::mac_scripts::view_source_html(target));
    ctx.app.tab_event(&ctx.tab_id, TabEvent::UrlChanged(format!("view-source:{target}")));
}

/// Loads `html` as the response for `url` (the address stays `url`, and
/// reload asks the site again). Its navigation is let through unchecked.
pub(super) fn load_in_place(ctx: &Ctx, wv: &WKWebView, url: &str, html: &str) {
    let Some(u) = nsurl(url) else { return };
    ctx.st.borrow_mut().allow_once = Some(url.to_string());
    unsafe {
        let req = NSURLRequest::requestWithURL(&u);
        let _ = wv.loadSimulatedRequest_responseHTMLString(&req, &ns(html));
    }
}

/// Loads a URL the way the address bar does (file URLs get read access to
/// their folder, which `loadRequest` would not grant).
pub fn load_url(wv: &WKWebView, url: &str) {
    let Some(u) = nsurl(url) else { return };
    unsafe {
        if u.isFileURL() {
            let dir = u.URLByDeletingLastPathComponent().unwrap_or_else(|| u.clone());
            let _ = wv.loadFileURL_allowingReadAccessToURL(&u, &dir);
        } else {
            let _ = wv.loadRequest(&NSURLRequest::requestWithURL(&u));
        }
    }
}

// ------------------------------------------------------------------ policy

pub(super) fn decide_action(
    this: &SericaNavDelegate,
    wv: &WKWebView,
    action: &WKNavigationAction,
    prefs: &WKWebpagePreferences,
    handler: &block2::Block<dyn Fn(WKNavigationActionPolicy, NonNull<WKWebpagePreferences>)>,
) {
    let cancel = || handler.call((WKNavigationActionPolicy::Cancel, NonNull::from(prefs)));
    let Some(ctx) = ctx_of(wv) else {
        return this.forward_decide(wv, action, prefs, handler);
    };
    let (url, main, nav_type, flags, button, download) = unsafe {
        let req = action.request();
        let url = url_string(req.URL().as_deref());
        let main = action.targetFrame().map(|f| f.isMainFrame()).unwrap_or(true);
        let download = responds(action, sel!(shouldPerformDownload)) && action.shouldPerformDownload();
        (url, main, action.navigationType(), action.modifierFlags().0 as u64, action.buttonNumber(), download)
    };
    let scheme = url.split(':').next().unwrap_or_default().to_ascii_lowercase();

    // A page Serica loads itself (block page, error page).
    if main {
        let mut st = ctx.st.borrow_mut();
        if st.allow_once.as_deref() == Some(url.as_str()) {
            st.allow_once = None;
            drop(st);
            return handler.call((WKNavigationActionPolicy::Allow, NonNull::from(prefs)));
        }
    }

    // The source viewer: reloading it shows the source again; going anywhere
    // else leaves it. `view-source:` itself opens it.
    if main {
        let shown = ctx.st.borrow().view_source.clone();
        if let Some(shown) = shown {
            if shown == url && nav_type == WKNavigationType::Reload {
                cancel();
                show_source(&ctx, wv, &url);
                return;
            }
            ctx.st.borrow_mut().view_source = None;
        }
        if scheme == "view-source" {
            cancel();
            let target = url["view-source:".len()..].to_string();
            if target.starts_with("http://") || target.starts_with("https://") {
                if let Some(hit) = ctx.app.focus.check(&target) {
                    ctx.app.focus_stats.blocked(&target);
                    let html = crate::focus::block_page(&hit, &target);
                    load_in_place(&ctx, wv, &target, &html);
                    ctx.app.tab_event(&ctx.tab_id, TabEvent::UrlChanged(target));
                } else {
                    show_source(&ctx, wv, &target);
                }
            }
            return;
        }
    }

    // Serica's own pages never load in a web page; other schemes open an app.
    if scheme == "serica" || scheme == "neutrino" {
        return cancel();
    }
    if !scheme.is_empty() && !is_web_scheme(&scheme) {
        cancel();
        if !matches!(scheme.as_str(), "vbscript") {
            let from = unsafe { action.sourceFrame().map(|f| f.securityOrigin()).map(|o| format!("{}://{}", o.protocol(), o.host())).unwrap_or_default() };
            super::dialogs::external(&ctx, &url, &scheme, &from);
        }
        return;
    }

    // Focus Mode: a blocked page is answered with the block page before
    // anything is sent to the site; whole blocked sites also lose the frames
    // other pages embed from them.
    if let Some(hit) = ctx.app.focus.check(&url) {
        cancel();
        if main {
            ctx.app.focus_stats.blocked(&url);
            let html = crate::focus::block_page(&hit, &url);
            load_in_place(&ctx, wv, &url, &html);
            ctx.app.tab_event(&ctx.tab_id, TabEvent::UrlChanged(url));
        }
        return;
    }

    if ctx.kind == Kind::Tab {
        // Command-click and middle-click open a tab (in the background;
        // Command-Shift-click brings it forward), as Safari does.
        let link = nav_type == WKNavigationType::LinkActivated;
        let cmd = flags & crate::mac_keys::CMD != 0;
        if main && link && (cmd || button == 2) && url.starts_with("http") {
            cancel();
            let foreground = flags & crate::mac_keys::SHIFT != 0;
            let (id, target) = (ctx.tab_id.clone(), url.clone());
            ctx.app.mutate(move |s| {
                if s.tab(&id).is_some_and(|t| t.private) {
                    s.create_private_tab(Some(target));
                    return;
                }
                let space = s.tab(&id).map(|t| t.space_id.clone());
                s.create_tab(Some(target), space, foreground, Some(&id));
            });
            return;
        }
        if download {
            return handler.call((WKNavigationActionPolicy::Download, NonNull::from(prefs)));
        }
        if main && (scheme == "http" || scheme == "https") {
            ctx.st.borrow_mut().top_url = url.clone();
            // Shields for this page: the content rule lists on unless the site
            // is allowed (or Shields are off), and its cosmetic filters.
            let ucc = unsafe { wv.configuration().userContentController() };
            crate::content_rules::set_tab_shields(&ucc, ctx.app.shields.active_for(&url));
            install_cosmetics(&ctx, wv, &url);
        }
    }
    this.forward_decide(wv, action, prefs, handler);
}

/// Whether a response is `Content-Disposition: attachment`.
fn is_attachment(response: &NSURLResponse) -> bool {
    let Some(http) = response.downcast_ref::<NSHTTPURLResponse>() else { return false };
    let headers = http.allHeaderFields();
    headers
        .valueForKey(ns_string!("Content-Disposition"))
        .and_then(|v| v.downcast::<NSString>().ok())
        .is_some_and(|d| d.to_string().split(';').next().is_some_and(|k| k.trim().eq_ignore_ascii_case("attachment")))
}

pub(super) fn decide_response(wv: &WKWebView, response: &WKNavigationResponse, handler: &block2::Block<dyn Fn(WKNavigationResponsePolicy)>) {
    unsafe {
        let r = response.response();
        let main = response.isForMainFrame();
        let mime = r.MIMEType().map(|m| m.to_string()).unwrap_or_default().to_ascii_lowercase();
        if main {
            if let Some(ctx) = ctx_of(wv) {
                ctx.st.borrow_mut().is_pdf = mime == "application/pdf";
            }
        }
        if !response.canShowMIMEType() || is_attachment(&r) {
            return handler.call((WKNavigationResponsePolicy::Download,));
        }
        handler.call((WKNavigationResponsePolicy::Allow,));
    }
}

// ------------------------------------------------------------------ lifecycle

pub(super) fn did_start(wv: &WKWebView, _redirect: bool) {
    let Some(ctx) = ctx_of(wv) else { return };
    let url = wv_url(wv);
    {
        let mut st = ctx.st.borrow_mut();
        if st.booting && (url.is_empty() || url == "about:blank") {
            return;
        }
        st.top_url = url.clone();
        st.media_frames.clear();
        st.playing_frames.clear();
        st.is_pdf = false;
    }
    ctx.app.tab_event(&ctx.tab_id, TabEvent::NavigationStarting(url));
    tab_changed(&ctx, objc2_web_kit::WKWebExtensionTabChangedProperties::Loading);
}

pub(super) fn did_commit(wv: &WKWebView) {
    let Some(ctx) = ctx_of(wv) else { return };
    let url = wv_url(wv);
    if ctx.st.borrow().booting && url == "about:blank" {
        return;
    }
    ctx.st.borrow_mut().top_url = url.clone();
    verify_scriptlets(&ctx, wv, &url);
    if ctx.app.pdf_dark_for(&ctx.tab_id) {
        super::api::eval_world(wv, &crate::mac_scripts::pdf_dark_js(true));
    }
    ctx.app.tab_event(&ctx.tab_id, TabEvent::UrlChanged(shown_url(&ctx, url)));
    history_changed(&ctx, wv);
}

pub(super) fn did_finish(wv: &WKWebView) {
    let Some(ctx) = ctx_of(wv) else { return };
    let url = wv_url(wv);
    if ctx.st.borrow().booting && url == "about:blank" {
        return;
    }
    ctx.st.borrow_mut().crash_reloads = 0;
    // Content from the back/forward cache or a service worker never asks
    // the policy delegate: put the block page there directly.
    if let Some(hit) = ctx.app.focus.check(&url) {
        focus_recheck(&ctx, wv, &url, &hit, true);
    }
    if ctx.app.pdf_dark_for(&ctx.tab_id) {
        super::api::eval_world(wv, &crate::mac_scripts::pdf_dark_js(true));
    }
    ctx.app.tab_event(&ctx.tab_id, TabEvent::Loaded { url: shown_url(&ctx, url), success: true });
    tab_changed(&ctx, objc2_web_kit::WKWebExtensionTabChangedProperties::Loading);
    read_theme_color(&ctx, wv);
    if ctx.kind == Kind::Tab {
        crate::tabhost::TabHost::capture_after_load(&ctx.app, ctx.tab_id.clone());
    }
}

/// A blocked URL is on screen: unless it already shows the block page, load
/// it again through the policy check (`hard`: show the block page directly,
/// for documents that never asked, like pages from the back/forward cache).
fn focus_recheck(ctx: &Rc<Ctx>, wv: &WKWebView, url: &str, hit: &crate::focus::Hit, hard: bool) {
    let page = hard.then(|| crate::focus::block_page(hit, url));
    let (url, weak) = (url.to_string(), Weak::from(wv));
    let ctx2 = ctx.clone();
    let done = RcBlock::new(move |result: *mut AnyObject, _err: *mut NSError| {
        let is_block = unsafe { result.as_ref() }.map(|r| to_json(Some(r)) == serde_json::Value::Bool(true)).unwrap_or(false);
        if is_block {
            return;
        }
        let Some(wv) = weak.load() else { return };
        match &page {
            Some(p) => load_in_place(&ctx2, &wv, &url, p),
            None => load_url(&wv, &url),
        }
    });
    unsafe {
        wv.evaluateJavaScript_completionHandler(&ns("window.__sericaFocusBlock === true"), Some(&done));
    }
}

pub(super) fn did_fail(wv: &WKWebView, error: &NSError, provisional: bool) {
    let Some(ctx) = ctx_of(wv) else { return };
    let (domain, code) = (error.domain().to_string(), error.code());
    let failing = error
        .userInfo()
        .objectForKey(ns_string!("NSErrorFailingURLStringKey"))
        .and_then(|v| v.downcast::<NSString>().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| wv_url(wv));
    let cancelled = (domain == "NSURLErrorDomain" && code == -999) || (domain == "WebKitErrorDomain" && matches!(code, 101 | 102 | 204));
    if domain == "WebKitErrorDomain" && code == 102 {
        // A navigation that became a download: the page stays, so does its address.
        let src = wv_url(wv);
        if !src.is_empty() {
            ctx.app.tab_event(&ctx.tab_id, TabEvent::UrlChanged(src));
        }
    }
    ctx.app.tab_event(&ctx.tab_id, TabEvent::Loaded { url: wv_url(wv), success: false });
    if !cancelled && provisional {
        show_error_page(&ctx, wv, &domain, code, &failing);
    } else if !cancelled && domain == "NSURLErrorDomain" && matches!(code, -1009 | -1005) {
        show_error_page(&ctx, wv, &domain, code, &failing);
    }
}

/// Replaces WebKit's blank failed page with Serica's own (see mac_scripts).
fn show_error_page(ctx: &Ctx, wv: &WKWebView, domain: &str, code: isize, url: &str) {
    if domain != "NSURLErrorDomain" || !url.starts_with("http") {
        return;
    }
    let (kind, title, body, err) = match code {
        -1009 | -1020 | -1018 => (
            "offline",
            "You're offline",
            "Serica can't reach the internet right now. Check your Wi-Fi or network connection — this page will reload as soon as you're back online.",
            "ERR_INTERNET_DISCONNECTED",
        ),
        -1003 | -1006 => (
            "dns",
            "This site can't be found",
            "The address might be mistyped, or the site might no longer exist. If you're offline, the page will reload once you reconnect.",
            "ERR_NAME_NOT_RESOLVED",
        ),
        -1001 => ("timeout", "This site took too long to respond", "The server may be busy or your connection may be slow. Try again in a moment.", "ERR_TIMED_OUT"),
        -1004 | -1005 | -1011 | -1017 | -1200 | -1100 => (
            "unreachable",
            "This site can't be reached",
            "The connection was interrupted or refused. Check your connection and try again.",
            "ERR_CONNECTION_FAILED",
        ),
        -1206..=-1201 => (
            "cert",
            "This connection is not private",
            "The site's certificate isn't trusted, so someone might be pretending to be it to steal your information. Go back unless you know this network and site.",
            "ERR_CERT_INVALID",
        ),
        _ => return,
    };
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    if kind == "cert" {
        ctx.st.borrow_mut().cert_nonce = Some((nonce.clone(), host_of(url)));
    }
    let html = crate::mac_scripts::error_page_html(kind, title, body, url, &origin_of(url), err, &nonce);
    load_in_place(ctx, wv, url, &html);
}

/// `SecTrustRef` (Security framework), only passed through.
#[repr(C)]
pub struct SecTrust {
    _private: [u8; 0],
}

unsafe impl objc2::encode::RefEncode for SecTrust {
    const ENCODING_REF: objc2::encode::Encoding = objc2::encode::Encoding::Pointer(&objc2::encode::Encoding::Struct("__SecTrust", &[]));
}

pub(super) fn auth_challenge(
    wv: &WKWebView,
    challenge: &NSURLAuthenticationChallenge,
    handler: &block2::Block<dyn Fn(NSURLSessionAuthChallengeDisposition, *mut NSURLCredential)>,
) {
    unsafe {
        let space = challenge.protectionSpace();
        let method = space.authenticationMethod().to_string();
        let host = space.host().to_string();
        match method.as_str() {
            "NSURLAuthenticationMethodServerTrust" => {
                let allowed = CERT_EXCEPTIONS.with(|c| c.borrow().contains(&host));
                let trust: *mut SecTrust = if allowed { msg_send![&*space, serverTrust] } else { std::ptr::null_mut() };
                let cred: Option<Retained<NSURLCredential>> = if trust.is_null() { None } else { msg_send![NSURLCredential::class(), credentialForTrust: trust] };
                if let Some(cred) = cred {
                    handler.call((NSURLSessionAuthChallengeDisposition::UseCredential, Retained::as_ptr(&cred) as *mut _));
                } else {
                    handler.call((NSURLSessionAuthChallengeDisposition::PerformDefaultHandling, std::ptr::null_mut()));
                }
            }
            "NSURLAuthenticationMethodHTTPBasic" | "NSURLAuthenticationMethodHTTPDigest" | "NSURLAuthenticationMethodNTLM" => {
                super::dialogs::http_auth(wv, challenge, handler.copy());
            }
            _ => handler.call((NSURLSessionAuthChallengeDisposition::PerformDefaultHandling, std::ptr::null_mut())),
        }
    }
}

pub(super) fn process_terminated(wv: &WKWebView) {
    let Some(ctx) = ctx_of(wv) else { return };
    let tries = {
        let mut st = ctx.st.borrow_mut();
        st.crash_reloads += 1;
        st.crash_reloads
    };
    tracing::warn!("web content process of {} ended", ctx.tab_id);
    ctx.app.tab_event(&ctx.tab_id, TabEvent::Audio(false));
    if tries <= 3 {
        unsafe {
            let _ = wv.reload();
        }
    }
}

pub(super) fn adopt_download(wv: &WKWebView, download: &WKDownload) {
    let Some(ctx) = ctx_of(wv) else { return };
    crate::downloads::native::adopt(&ctx.app, &ctx.tab_id, download);
}

/// A content rule list acted on a request: count what Shields blocked.
pub(super) fn rule_list_action(wv: &WKWebView, identifier: &NSString, action: &AnyObject, url: &NSURL) {
    let Some(ctx) = ctx_of(wv) else { return };
    if !identifier.to_string().starts_with(crate::content_rules::SHIELDS_PREFIX) {
        return;
    }
    let blocked: bool = responds(action, sel!(blockedLoad)) && unsafe { msg_send![action, blockedLoad] };
    if !blocked {
        return;
    }
    let u = url_string(Some(url));
    // The same load, reported again by another list: counted once. (The
    // same list again is the page asking again.)
    let list = identifier.to_string();
    {
        let mut st = ctx.st.borrow_mut();
        let now = std::time::Instant::now();
        st.recent_blocks.retain(|(_, _, at)| now.duration_since(*at) < std::time::Duration::from_millis(250));
        let repeat = st.recent_blocks.iter().any(|(lu, ll, _)| *lu == u && *ll != list);
        if !repeat && st.recent_blocks.len() < 512 {
            st.recent_blocks.push((u.clone(), list, now));
        }
        if repeat {
            return;
        }
    }
    let rtype = crate::content_rules::guess_type(&u);
    // WebKit says only what it blocked; the engine names the filter that matched.
    let top = ctx.st.borrow().top_url.clone();
    let d = ctx.app.shields.check(&u, &top, rtype).unwrap_or(crate::shields::Decision { rule: None, category: crate::shields::categorize(&u) });
    let item = ctx.app.shields.record_block(&ctx.tab_id, &u, &d, rtype);
    ctx.app.on_blocked(item);
}

// ------------------------------------------------------------------ observed state

fn history_changed(ctx: &Ctx, wv: &WKWebView) {
    let (back, forward) = unsafe { (wv.canGoBack(), wv.canGoForward()) };
    ctx.app.tab_event(&ctx.tab_id, TabEvent::History { back, forward });
}

pub(super) fn observed(wv: &WKWebView, key: &str) {
    let Some(ctx) = ctx_of(wv) else { return };
    match key {
        "URL" => {
            let url = wv_url(wv);
            {
                let st = ctx.st.borrow();
                if url.is_empty() || (st.booting && url == "about:blank") {
                    return;
                }
            }
            // A single-page app moving to a blocked path makes no request:
            // load it for real so the policy check answers it.
            if let Some(hit) = ctx.app.focus.check(&url) {
                focus_recheck(&ctx, wv, &url, &hit, false);
            }
            ctx.st.borrow_mut().top_url = url.clone();
            ctx.app.tab_event(&ctx.tab_id, TabEvent::UrlChanged(shown_url(&ctx, url)));
            tab_changed(&ctx, objc2_web_kit::WKWebExtensionTabChangedProperties::URL);
        }
        "title" => {
            let t = unsafe { wv.title() }.map(|t| t.to_string()).unwrap_or_default();
            if t != "about:blank" && !ctx.st.borrow().booting {
                ctx.app.tab_event(&ctx.tab_id, TabEvent::Title(t));
                tab_changed(&ctx, objc2_web_kit::WKWebExtensionTabChangedProperties::Title);
            }
        }
        "canGoBack" | "canGoForward" => history_changed(&ctx, wv),
        "_isPlayingAudio" => {
            let playing: bool = unsafe { msg_send![wv, _isPlayingAudio] };
            ctx.app.tab_event(&ctx.tab_id, TabEvent::Audio(playing));
        }
        "fullscreenState" => {
            // WKFullscreenState: 0 not, 1 entering, 2 in, 3 exiting.
            let state: isize = unsafe { msg_send![wv, fullscreenState] };
            let fs = matches!(state, 1 | 2);
            let changed = {
                let mut st = ctx.st.borrow_mut();
                let c = st.fullscreen != fs;
                st.fullscreen = fs;
                c
            };
            if changed && (state == 1 || state == 0) {
                crate::clip_mac::set_fullscreen(wv, fs);
                crate::tabhost::TabHost::set_html_fullscreen(&ctx.app, fs);
                ui_emit(&ctx.app.handle, "serica:html-fullscreen", fs);
            }
        }
        "themeColor" => {
            let color = unsafe { theme_color_hex(wv) };
            ui_emit(&ctx.app.handle, "serica:page-color", serde_json::json!({ "tabId": ctx.tab_id, "themeColor": color }));
        }
        _ => {}
    }
}

/// `WKWebView.themeColor` (the page's `theme-color`) as `#rrggbb`.
unsafe fn theme_color_hex(wv: &WKWebView) -> Option<String> {
    if !responds(wv, sel!(themeColor)) {
        return None;
    }
    let c: Option<Retained<objc2_app_kit::NSColor>> = msg_send![wv, themeColor];
    let c = c?.colorUsingColorSpace(&objc2_app_kit::NSColorSpace::sRGBColorSpace())?;
    let b = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(format!("#{:02x}{:02x}{:02x}", b(c.redComponent()), b(c.greenComponent()), b(c.blueComponent())))
}

fn read_theme_color(ctx: &Rc<Ctx>, wv: &WKWebView) {
    let (app, id) = (ctx.app.clone(), ctx.tab_id.clone());
    let done = RcBlock::new(move |result: *mut AnyObject, _err: *mut NSError| {
        let v = to_json(unsafe { result.as_ref() });
        let color = v.as_str().map(str::to_string);
        ui_emit(&app.handle, "serica:page-color", serde_json::json!({ "tabId": id, "themeColor": color }));
    });
    unsafe {
        wv.evaluateJavaScript_completionHandler(&ns(crate::mac_scripts::THEME_COLOR_JS), Some(&done));
    }
}

// ------------------------------------------------------------------ cosmetic filters

/// Keep at most this many per-host cosmetic scripts per tab.
const MAX_COSMETIC_SCRIPTS: usize = 48;

/// Registers the cosmetic CSS and scriptlets for `url`'s site to run at
/// document start (page world, top frame), before the page itself loads.
pub(super) fn install_cosmetics(ctx: &Ctx, wv: &WKWebView, url: &str) {
    let res = ctx.app.shields.cosmetic(url);
    let host = host_of(url);
    {
        let mut st = ctx.st.borrow_mut();
        st.exceptions = res.as_ref().map(|r| r.exceptions.clone()).unwrap_or_default();
        st.generichide = res.as_ref().map(|r| r.generichide).unwrap_or(false);
    }
    let Some(res) = res else { return };
    if host.is_empty() || (res.hide_selectors.is_empty() && res.injected_script.trim().is_empty()) {
        return;
    }
    let ucc = unsafe { wv.configuration().userContentController() };
    {
        let mut st = ctx.st.borrow_mut();
        if st.cosmetic_hosts.contains(&host) {
            return;
        }
        if st.cosmetic_hosts.len() >= MAX_COSMETIC_SCRIPTS {
            // WebKit can only drop every user script at once: put back the
            // ones the page always needs.
            unsafe {
                ucc.removeAllUserScripts();
                for s in &st.base_scripts {
                    ucc.addUserScript(s);
                }
            }
            st.cosmetic_hosts.clear();
            st.scriptlet_hosts.clear();
        }
    }
    let has_scriptlets = !res.injected_script.trim().is_empty();
    let css = hide_css(res.hide_selectors.into_iter());
    let script = format!(
        r#"(function(){{
  if (window.top !== window || location.hostname !== {host}) return;
  try {{ Object.defineProperty(window, '__sericaCosmetic', {{ value: {host} }}); }} catch (e) {{}}
  const css = {css};
  if (css) {{
    const add = () => {{ const s = document.createElement('style'); s.textContent = css; (document.head || document.documentElement).appendChild(s); }};
    if (document.documentElement) add(); else document.addEventListener('readystatechange', add, {{ once: true }});
  }}
  try {{
{scriptlets}
  }} catch (e) {{}}
}})();"#,
        host = js_string(&host),
        css = js_string(&css),
        scriptlets = res.injected_script
    );
    unsafe {
        ucc.addUserScript(&user_script(&Script { source: &script, all_frames: false, page_world: true }));
    }
    let mut st = ctx.st.borrow_mut();
    st.cosmetic_hosts.insert(host.clone());
    if has_scriptlets {
        st.scriptlet_hosts.insert(host);
    }
}

/// When a page starts: if its site needs scriptlets (YouTube ads, etc.) but
/// they were registered too late for this document, reload once so they run.
fn verify_scriptlets(ctx: &Rc<Ctx>, wv: &WKWebView, url: &str) {
    let host = host_of(url);
    {
        let st = ctx.st.borrow();
        if host.is_empty() || !st.scriptlet_hosts.contains(&host) || st.reloaded_for.contains(&host) {
            return;
        }
    }
    let (ctx2, weak) = (ctx.clone(), Weak::from(wv));
    let check = format!("window.__sericaCosmetic === {}", js_string(&host));
    let done = RcBlock::new(move |result: *mut AnyObject, err: *mut NSError| {
        if !err.is_null() {
            return;
        }
        let ok = to_json(unsafe { result.as_ref() }) == serde_json::Value::Bool(true);
        if !ok && ctx2.st.borrow_mut().reloaded_for.insert(host.clone()) {
            tracing::info!("scriptlets missed the first load of {host}; reloading once");
            if let Some(wv) = weak.load() {
                unsafe {
                    let _ = wv.reload();
                }
            }
        }
    });
    unsafe {
        wv.evaluateJavaScript_completionHandler(&ns(&check), Some(&done));
    }
}
