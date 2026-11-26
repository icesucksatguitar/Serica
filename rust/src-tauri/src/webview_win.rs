//! WebView2-specific integration for tab webviews (Windows).
//!
//! Everything here runs on the UI thread. Handlers only touch the store
//! briefly (via `App`) and never wait on other threads.

#![allow(clippy::missing_safety_doc)]

use crate::app::{ui_emit, Shared, Shortcut, TabEvent};
use crate::shields::hide_css;
use base64::Engine as _;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use tauri::Webview;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::core::{Interface, HSTRING, PWSTR};
use windows::Win32::System::Com::{CoTaskMemFree, IStream, STREAM_SEEK_SET};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT};

/// Per-webview state kept on the UI thread.
#[derive(Default)]
struct Ctx {
    /// URL of the current (or pending) top-level document: the "source" for adblock.
    top_url: String,
    /// Cosmetic CSS/scriptlet scripts registered per host. They are kept (each
    /// checks its own hostname) so same-site navigations never race a re-add.
    cosmetic_scripts: HashMap<String, Option<String>>,
    /// Hosts whose scripts include scriptlets (must run before page scripts).
    scriptlet_hosts: HashSet<String>,
    /// Hosts this tab already reloaded once because the scriptlets missed the first load.
    reloaded_for: HashSet<String>,
    exceptions: HashSet<String>,
    generichide: bool,
    /// The webview starts on about:blank while its first page's adblock
    /// scripts are registered; that blank page's events are not reported.
    booting: bool,
}

fn take(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe {
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

fn key_down(vk: u16) -> bool {
    unsafe { GetKeyState(vk as i32) < 0 }
}

fn js_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Maps a WebView2 resource context to adblock-rust request types.
fn request_type(ctx: COREWEBVIEW2_WEB_RESOURCE_CONTEXT, is_top: bool) -> &'static str {
    match ctx {
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT => {
            if is_top {
                "document"
            } else {
                "sub_frame"
            }
        }
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_STYLESHEET => "stylesheet",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE => "image",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA => "media",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FONT => "font",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT => "script",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST => "xhr",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH => "fetch",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_WEBSOCKET => "websocket",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_PING => "ping",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_CSP_VIOLATION_REPORT => "csp_report",
        _ => "other",
    }
}

fn shortcut_for(vk: u32, ctrl: bool, shift: bool, alt: bool) -> Option<Shortcut> {
    const VK_TAB: u32 = 0x09;
    const VK_ESCAPE: u32 = 0x1B;
    const VK_PRIOR: u32 = 0x21;
    const VK_NEXT: u32 = 0x22;
    const VK_HOME: u32 = 0x24;
    const VK_F6: u32 = 0x75;
    const VK_OEM_COMMA: u32 = 0xBC;
    if ctrl && !alt {
        return match vk {
            0x54 if shift => Some(Shortcut::ReopenTab), // T
            0x4E if shift => Some(Shortcut::NewPrivateTab), // N
            0x54 => Some(Shortcut::NewTab),
            0x4C | 0x4B => Some(Shortcut::OpenCommandBar), // L, K
            0x57 | 0x73 => Some(Shortcut::CloseTab),        // W, F4
            0x45 | 0x53 => Some(Shortcut::ToggleSidebar),   // E, S
            0x46 if !shift => Some(Shortcut::Find),         // F
            0x48 => Some(Shortcut::History),                // H
            0x4F if !shift => Some(Shortcut::OpenFile),     // O
            0x4A => Some(Shortcut::Downloads),              // J
            VK_OEM_COMMA => Some(Shortcut::Settings),
            VK_TAB => Some(Shortcut::CycleTab(if shift { -1 } else { 1 })),
            VK_NEXT => Some(Shortcut::CycleTab(1)),
            VK_PRIOR => Some(Shortcut::CycleTab(-1)),
            0x31..=0x39 => Some(Shortcut::SpaceIndex((vk - 0x30) as usize)),
            _ => None,
        };
    }
    if alt && !ctrl && vk == VK_HOME {
        return Some(Shortcut::Home);
    }
    match vk {
        VK_F6 => Some(Shortcut::OpenCommandBar),
        VK_ESCAPE => Some(Shortcut::Escape),
        _ => None,
    }
}

/// Installs all native handlers on a freshly created tab webview.
/// Hooks a new tab webview. With `first_url`, the webview was created on
/// about:blank: the site's cosmetic filters and scriptlets are registered
/// first and the page is loaded after, so they run on its first load (no
/// reload to apply them).
pub fn attach(app: &Shared, tab_id: &str, profile: &str, wv: &Webview, first_url: Option<String>) {
    let app = app.clone();
    let tab_id = tab_id.to_string();
    let profile = profile.to_string();
    let _ = wv.with_webview(move |pw| unsafe {
        let controller = pw.controller();
        let env = pw.environment();
        if app.extensions.claim_sync(&profile) {
            if let Ok(core) = controller.CoreWebView2() {
                crate::extensions::native::sync(&app, &core);
            }
        }
        if let Err(e) = install(app, tab_id, profile, controller, env, first_url) {
            tracing::error!("webview2 hook install failed: {e}");
        }
    });
}

unsafe fn install(
    app: Shared,
    tab_id: String,
    profile: String,
    controller: ICoreWebView2Controller,
    env: ICoreWebView2Environment,
    first_url: Option<String>,
) -> windows::core::Result<()> {
    let core = controller.CoreWebView2()?;
    let ctx = Rc::new(RefCell::new(Ctx { booting: first_url.is_some(), ..Ctx::default() }));
    let mut token = Default::default();

    if let Ok(settings) = core.Settings() {
        let _ = settings.SetAreDevToolsEnabled(true);
        let _ = settings.SetIsZoomControlEnabled(true);
        let _ = settings.SetIsStatusBarEnabled(true);
        let _ = settings.SetAreDefaultContextMenusEnabled(true);
        // The engine's own password manager and form autofill, as in Edge:
        // offers to save a password on sign-in and fills it in next time.
        // Saved per profile, encrypted by Windows (DPAPI).
        if let Ok(s4) = settings.cast::<ICoreWebView2Settings4>() {
            let _ = s4.SetIsPasswordAutosaveEnabled(true);
            let _ = s4.SetIsGeneralAutofillEnabled(true);
        }
    }

    if let Err(e) = install_context_menu(&app, &tab_id, &core, &controller) {
        tracing::warn!("context menu hook install failed: {e}");
    }

    if let Err(e) = crate::downloads::native::install(&app, &tab_id, &core) {
        tracing::warn!("download hook install failed: {e}");
    }

    // ---- navigation lifecycle
    {
        let (app, id, ctx, core2) = (app.clone(), tab_id.clone(), ctx.clone(), core.clone());
        core.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut uri = PWSTR::null();
                args.Uri(&mut uri)?;
                let uri = take(uri);
                if ctx.borrow().booting && uri == "about:blank" {
                    return Ok(());
                }
                ctx.borrow_mut().top_url = uri.clone();
                install_cosmetics(&app, &core2, &ctx, &uri, None);
                app.tab_event(&id, TabEvent::NavigationStarting(uri));
                Ok(())
            })),
            &mut token,
        )?;
    }
    {
        let (app, id, ctx) = (app.clone(), tab_id.clone(), ctx.clone());
        core.add_SourceChanged(
            &SourceChangedEventHandler::create(Box::new(move |sender, args| {
                let Some(sender) = sender else { return Ok(()) };
                let mut new_doc = Default::default();
                if let Some(a) = args.as_ref() {
                    let _ = a.IsNewDocument(&mut new_doc);
                }
                let same_document = !new_doc.as_bool();
                let mut uri = PWSTR::null();
                sender.Source(&mut uri)?;
                let uri = take(uri);
                ctx.borrow_mut().top_url = uri.clone();
                // A single-page app moving to a blocked path makes no request:
                // load it for real so the request pipeline answers it.
                if same_document {
                    if let Some(hit) = app.focus.check(&uri) {
                        focus_recheck(&sender, &uri, &hit, false);
                    }
                }
                app.tab_event(&id, TabEvent::UrlChanged(uri));
                Ok(())
            })),
            &mut token,
        )?;
    }
    {
        let (app, id) = (app.clone(), tab_id.clone());
        core.add_HistoryChanged(
            &HistoryChangedEventHandler::create(Box::new(move |sender, _| {
                let Some(sender) = sender else { return Ok(()) };
                let (mut back, mut fwd) = (Default::default(), Default::default());
                sender.CanGoBack(&mut back)?;
                sender.CanGoForward(&mut fwd)?;
                app.tab_event(&id, TabEvent::History { back: back.as_bool(), forward: fwd.as_bool() });
                Ok(())
            })),
            &mut token,
        )?;
    }
    {
        let (ctx, app, id) = (ctx.clone(), app.clone(), tab_id.clone());
        core.add_ContentLoading(
            &ContentLoadingEventHandler::create(Box::new(move |sender, args| {
                let (Some(sender), Some(args)) = (sender, args) else { return Ok(()) };
                let mut error_page = Default::default();
                args.IsErrorPage(&mut error_page)?;
                if !error_page.as_bool() {
                    verify_scriptlets(&sender, &ctx);
                    if app.pdf_dark_for(&id) {
                        let js = crate::tabhost::pdf_dark_js(true);
                        let _ = sender.ExecuteScript(&HSTRING::from(js), None);
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    {
        let (app, id, boot_ctx) = (app.clone(), tab_id.clone(), ctx.clone());
        core.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |sender, args| {
                let (Some(sender), Some(args)) = (sender, args) else { return Ok(()) };
                let mut ok = Default::default();
                args.IsSuccess(&mut ok)?;
                let mut uri = PWSTR::null();
                sender.Source(&mut uri)?;
                let url = take(uri);
                if url == "about:blank" && boot_ctx.borrow().booting {
                    return Ok(());
                }
                if !ok.as_bool() {
                    let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                    if args.WebErrorStatus(&mut status).is_ok() {
                        show_error_page(&sender, status, &url);
                    }
                }
                if let Some(hit) = app.focus.check(&url) {
                    focus_recheck(&sender, &url, &hit, true);
                }
                app.tab_event(&id, TabEvent::Loaded { url, success: ok.as_bool() });
                read_theme_color(&app, &id, &sender);
                crate::tabhost::TabHost::capture_after_load(&app, id.clone());
                Ok(())
            })),
            &mut token,
        )?;
    }
    {
        let (app, id) = (app.clone(), tab_id.clone());
        core.add_DocumentTitleChanged(
            &DocumentTitleChangedEventHandler::create(Box::new(move |sender, _| {
                let Some(sender) = sender else { return Ok(()) };
                let mut t = PWSTR::null();
                sender.DocumentTitle(&mut t)?;
                let t = take(t);
                if t != "about:blank" {
                    app.tab_event(&id, TabEvent::Title(t));
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    if let Ok(core15) = core.cast::<ICoreWebView2_15>() {
        let (app, id) = (app.clone(), tab_id.clone());
        core15.add_FaviconChanged(
            &FaviconChangedEventHandler::create(Box::new(move |sender, _| {
                let Some(sender) = sender else { return Ok(()) };
                if let Ok(c) = sender.cast::<ICoreWebView2_15>() {
                    let mut f = PWSTR::null();
                    c.FaviconUri(&mut f)?;
                    let f = take(f);
                    if !f.is_empty() {
                        app.tab_event(&id, TabEvent::Favicon(f));
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    if let Ok(core8) = core.cast::<ICoreWebView2_8>() {
        let (app, id) = (app.clone(), tab_id.clone());
        core8.add_IsDocumentPlayingAudioChanged(
            &IsDocumentPlayingAudioChangedEventHandler::create(Box::new(move |sender, _| {
                let Some(sender) = sender else { return Ok(()) };
                if let Ok(c) = sender.cast::<ICoreWebView2_8>() {
                    let mut playing = Default::default();
                    c.IsDocumentPlayingAudio(&mut playing)?;
                    app.tab_event(&id, TabEvent::Audio(playing.as_bool()));
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    install_dialogs(&app, &tab_id, &core)?;
    if let Err(e) = install_permissions(&app, &tab_id, &core) {
        tracing::warn!("permission hook install failed: {e}");
    }
    {
        let app = app.clone();
        core.add_ContainsFullScreenElementChanged(
            &ContainsFullScreenElementChangedEventHandler::create(Box::new(move |sender, _| {
                let Some(sender) = sender else { return Ok(()) };
                let mut fs = Default::default();
                sender.ContainsFullScreenElement(&mut fs)?;
                let fs = fs.as_bool();
                use tauri::Manager;
                if let Some(w) = app.handle.get_window("main") {
                    let _ = w.set_fullscreen(fs);
                }
                ui_emit(&app.handle, "serica:html-fullscreen", fs);
                Ok(())
            })),
            &mut token,
        )?;
    }

    // ---- browser shortcuts while a page has focus
    {
        let app = app.clone();
        controller.add_AcceleratorKeyPressed(
            &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
                args.KeyEventKind(&mut kind)?;
                if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN {
                    return Ok(());
                }
                let mut status = Default::default();
                args.PhysicalKeyStatus(&mut status)?;
                let mut vk = 0u32;
                args.VirtualKey(&mut vk)?;
                let sc = shortcut_for(vk, key_down(VK_CONTROL.0), key_down(VK_SHIFT.0), key_down(VK_MENU.0));
                if let Some(sc) = sc {
                    // Escape still reaches the page (closing its own dialogs).
                    if !matches!(sc, Shortcut::Escape) {
                        args.SetHandled(true)?;
                    }
                    if status.WasKeyDown.as_bool() && !matches!(sc, Shortcut::CycleTab(_)) {
                        return Ok(());
                    }
                    app.shortcut(sc);
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    {
        let (app, id) = (app.clone(), tab_id.clone());
        controller.add_ZoomFactorChanged(
            &ZoomFactorChangedEventHandler::create(Box::new(move |sender, _| {
                let Some(sender) = sender else { return Ok(()) };
                let mut z = 1.0f64;
                sender.ZoomFactor(&mut z)?;
                app.tab_event(&id, TabEvent::Zoom(z));
                Ok(())
            })),
            &mut token,
        )?;
    }

    // ---- Brave adblock: network requests
    let filter = HSTRING::from("*");
    match core.cast::<ICoreWebView2_22>() {
        Ok(c22) => c22.AddWebResourceRequestedFilterWithRequestSourceKinds(
            &filter,
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
        )?,
        Err(_) => core.AddWebResourceRequestedFilter(&filter, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?,
    }
    {
        let (app, id, ctx, env) = (app.clone(), tab_id.clone(), ctx.clone(), env.clone());
        core.add_WebResourceRequested(
            &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let req = args.Request()?;
                let mut uri = PWSTR::null();
                req.Uri(&mut uri)?;
                let uri = take(uri);
                // Leave Tauri's own protocols alone.
                if uri.starts_with("http://ipc.localhost") || uri.starts_with("http://tauri.localhost") || uri.starts_with("tauri:") || uri.starts_with("chrome-extension:") {
                    return Ok(());
                }
                let mut rc = COREWEBVIEW2_WEB_RESOURCE_CONTEXT::default();
                args.ResourceContext(&mut rc)?;
                // Focus Mode: a blocked page is answered with the block page
                // before anything is sent to the site; whole blocked sites
                // also lose the requests other pages make to them.
                if let Some(hit) = app.focus.check(&uri) {
                    if rc == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT {
                        app.focus_stats.blocked(&uri);
                        args.SetResponse(&focus_response(&env, &hit, &uri)?)?;
                        return Ok(());
                    }
                    if hit.whole_site {
                        args.SetResponse(&env.CreateWebResourceResponse(None, 403, &HSTRING::from("Blocked by Focus Mode"), &HSTRING::from(""))?)?;
                        return Ok(());
                    }
                }
                let source = ctx.borrow().top_url.clone();
                let rtype = request_type(rc, uri == source);
                if rtype == "document" {
                    return Ok(());
                }
                let Some(d) = app.shields.check(&uri, &source, rtype) else { return Ok(()) };
                let response = match d.redirect.as_deref().and_then(decode_data_url) {
                    Some((mime, body)) => {
                        let stream = windows::Win32::UI::Shell::SHCreateMemStream(Some(&body));
                        env.CreateWebResourceResponse(stream.as_ref(), 200, &HSTRING::from("OK"), &HSTRING::from(format!("Content-Type: {mime}\r\nAccess-Control-Allow-Origin: *")))?
                    }
                    None => env.CreateWebResourceResponse(None, 403, &HSTRING::from("Blocked by Serica Shields"), &HSTRING::from(""))?,
                };
                args.SetResponse(&response)?;
                let item = app.shields.record_block(&id, &uri, &d, rtype);
                app.on_blocked(item);
                Ok(())
            })),
            &mut token,
        )?;
    }

    // ---- Brave adblock: generic cosmetic filtering (page reports classes/ids)
    {
        let (app, ctx, id) = (app.clone(), ctx.clone(), tab_id.clone());
        core.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(move |sender, args| {
                let (Some(sender), Some(args)) = (sender, args) else { return Ok(()) };
                let mut msg = PWSTR::null();
                if args.TryGetWebMessageAsString(&mut msg).is_err() {
                    return Ok(());
                }
                let msg = take(msg);
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&msg) else { return Ok(()) };
                if v.get("__serica").is_none() {
                    return Ok(());
                }
                if v["t"] == "swipe-nav" {
                    let mut can = Default::default();
                    if v["dir"] == "back" {
                        if sender.CanGoBack(&mut can).is_ok() && can.as_bool() {
                            let _ = sender.GoBack();
                        }
                    } else if sender.CanGoForward(&mut can).is_ok() && can.as_bool() {
                        let _ = sender.GoForward();
                    }
                    return Ok(());
                }
                if v["t"] == "ext-install" || v["t"] == "ext-query" {
                    let mut src = PWSTR::null();
                    args.Source(&mut src)?;
                    let from_store = url::Url::parse(&take(src)).ok().and_then(|u| u.host_str().map(|h| h == "chromewebstore.google.com")).unwrap_or(false);
                    let Some(ext_id) = v["id"].as_str().filter(|s| crate::extensions::is_extension_id(s)) else { return Ok(()) };
                    if !from_store {
                        return Ok(());
                    }
                    if v["t"] == "ext-query" {
                        if app.extensions.is_installed(ext_id) {
                            let js = crate::extensions::store_state_js(ext_id, "installed", None);
                            let _ = sender.ExecuteScript(&HSTRING::from(js), None);
                        }
                    } else {
                        crate::commands::spawn_store_install(&app, &id, ext_id);
                    }
                    return Ok(());
                }
                if v["t"] != "cosmetic" {
                    return Ok(());
                }
                let c = ctx.borrow();
                if c.generichide || !app.shields.active_for(&c.top_url) {
                    return Ok(());
                }
                let list = |k: &str| -> Vec<String> {
                    v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).take(5000).collect()).unwrap_or_default()
                };
                let selectors = app.shields.class_id_selectors(&list("classes"), &list("ids"), &c.exceptions);
                drop(c);
                if !selectors.is_empty() {
                    let css = hide_css(selectors);
                    let js = format!(
                        "(()=>{{const s=document.createElement('style');s.textContent={};(document.head||document.documentElement).appendChild(s);}})();",
                        js_string(&css)
                    );
                    let _ = sender.ExecuteScript(&HSTRING::from(js), None);
                }
                Ok(())
            })),
            &mut token,
        )?;
    }

    // ---- native find-in-page (results drive the UI's find bar)
    if let Ok(c28) = core.cast::<ICoreWebView2_28>() {
        if let Ok(find) = c28.Find() {
            let report = {
                let (app, id) = (app.clone(), tab_id.clone());
                move |f: &ICoreWebView2Find| {
                    let (mut count, mut active) = (0i32, 0i32);
                    let _ = f.MatchCount(&mut count);
                    let _ = f.ActiveMatchIndex(&mut active);
                    ui_emit(
                        &app.handle,
                        "serica:find-result",
                        serde_json::json!({ "tabId": id, "totalMatches": count.max(0), "activeMatch": if count > 0 { active + 1 } else { 0 } }),
                    );
                }
            };
            let r1 = report.clone();
            find.add_MatchCountChanged(
                &FindMatchCountChangedEventHandler::create(Box::new(move |f, _| {
                    if let Some(f) = f {
                        r1(&f);
                    }
                    Ok(())
                })),
                &mut token,
            )?;
            find.add_ActiveMatchIndexChanged(
                &FindActiveMatchIndexChangedEventHandler::create(Box::new(move |f, _| {
                    if let Some(f) = f {
                        report(&f);
                    }
                    Ok(())
                })),
                &mut token,
            )?;
        }
    }

    // Load the first page last, once every handler above is in place.
    if let Some(url) = first_url {
        let (app2, core2, ctx2) = (app.clone(), core.clone(), ctx.clone());
        let load = move || {
            let (core3, ctx3) = (core2.clone(), ctx2.clone());
            install_cosmetics(
                &app2,
                &core2,
                &ctx2,
                &url.clone(),
                Some(Box::new(move || {
                    ctx3.borrow_mut().booting = false;
                    let _ = core3.Navigate(&HSTRING::from(url));
                })),
            );
        };
        // "Clear cookies when Serica closes": also done before the first page
        // of a session loads, in case Serica could not do it while closing
        // (closed with Alt+F4, shut down, crashed).
        // Only before the profile's first page this session: turning the
        // setting on later must not sign the user out of the session.
        let first_of_session = app.claim_cookie_sweep(&profile);
        let s = app.settings();
        if s.clear_cookies_on_exit && first_of_session {
            let keep = s.cookie_keep_sites.clone();
            sweep(&core, move |domain| !cookie_in_sites(domain, &keep), move |n| {
                if n > 0 {
                    tracing::info!("cleared {n} cookies at start");
                }
                load();
            });
        } else {
            load();
        }
    }
    Ok(())
}

/// The block page as a response to a document request.
unsafe fn focus_response(env: &ICoreWebView2Environment, hit: &crate::focus::Hit, uri: &str) -> windows::core::Result<ICoreWebView2WebResourceResponse> {
    let html = crate::focus::block_page(hit, uri);
    let stream = windows::Win32::UI::Shell::SHCreateMemStream(Some(html.as_bytes()));
    env.CreateWebResourceResponse(
        stream.as_ref(),
        200,
        &HSTRING::from("OK"),
        &HSTRING::from("Content-Type: text/html; charset=utf-8

Cache-Control: no-store"),
    )
}

/// A blocked URL is on screen: unless it already shows the block page, load
/// it again through the request pipeline (`hard`: replace the document with
/// the block page directly, for content that never makes a request, like
/// pages from the back/forward cache or a service worker).
unsafe fn focus_recheck(core: &ICoreWebView2, url: &str, hit: &crate::focus::Hit, hard: bool) {
    let (core2, url) = (core.clone(), url.to_string());
    let page = hard.then(|| crate::focus::block_page(hit, &url));
    let _ = core.ExecuteScript(
        &HSTRING::from("window.__sericaFocusBlock === true"),
        &ExecuteScriptCompletedHandler::create(Box::new(move |hr, result| {
            if hr.is_ok() && result.trim() == "true" {
                return Ok(());
            }
            if let Some(page) = &page {
                let _ = core2.NavigateToString(&HSTRING::from(page.as_str()));
            } else {
                let _ = core2.Navigate(&HSTRING::from(url.as_str()));
            }
            Ok(())
        })),
    );
}

/// Registers this navigation's cosmetic CSS and scriptlets to run at document
/// creation, replacing the previous navigation's script.
/// Keep at most this many per-host cosmetic scripts per tab.
const MAX_COSMETIC_SCRIPTS: usize = 48;

unsafe fn install_cosmetics(
    app: &Shared,
    core: &ICoreWebView2,
    ctx: &Rc<RefCell<Ctx>>,
    url: &str,
    done: Option<Box<dyn FnOnce()>>,
) {
    // Runs `done` now (nothing to wait for) or once the script is registered.
    let finish_now = |done: Option<Box<dyn FnOnce()>>| {
        if let Some(f) = done {
            f();
        }
    };
    let res = app.shields.cosmetic(url);
    let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    {
        let mut c = ctx.borrow_mut();
        c.exceptions = res.as_ref().map(|r| r.exceptions.clone()).unwrap_or_default();
        c.generichide = res.as_ref().map(|r| r.generichide).unwrap_or(false);
    }
    let Some(res) = res else { return finish_now(done) };
    if host.is_empty() || (res.hide_selectors.is_empty() && res.injected_script.trim().is_empty()) {
        return finish_now(done);
    }
    if ctx.borrow().cosmetic_scripts.contains_key(&host) {
        return finish_now(done); // already registered for this host (or registering)
    }
    if ctx.borrow().cosmetic_scripts.len() >= MAX_COSMETIC_SCRIPTS {
        let old: Vec<String> = ctx.borrow_mut().cosmetic_scripts.drain().filter_map(|(_, id)| id).collect();
        for id in old {
            let _ = core.RemoveScriptToExecuteOnDocumentCreated(&HSTRING::from(id));
        }
        ctx.borrow_mut().scriptlet_hosts.clear();
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
    let host_key = host.clone();
    {
        let mut c = ctx.borrow_mut();
        c.cosmetic_scripts.insert(host.clone(), None);
        if has_scriptlets {
            c.scriptlet_hosts.insert(host.clone());
        }
    }
    let ctx2 = ctx.clone();
    // Shared so `done` still runs if registration fails before its handler.
    let done = Rc::new(std::cell::Cell::new(done));
    let done2 = done.clone();
    let added = core.AddScriptToExecuteOnDocumentCreated(
        &HSTRING::from(script),
        &AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(move |hr, id| {
            {
                let mut c = ctx2.borrow_mut();
                if hr.is_ok() {
                    c.cosmetic_scripts.insert(host.clone(), Some(id));
                } else {
                    c.cosmetic_scripts.remove(&host);
                    c.scriptlet_hosts.remove(&host);
                }
            }
            if let Some(f) = done2.take() {
                f();
            }
            Ok(())
        })),
    );
    if added.is_err() {
        // The completion handler will not run: carry on without the filters.
        tracing::warn!("could not register cosmetic filters");
        {
            let mut c = ctx.borrow_mut();
            c.cosmetic_scripts.remove(&host_key);
            c.scriptlet_hosts.remove(&host_key);
        }
        if let Some(f) = done.take() {
            f();
        }
    }
}

/// When a page starts loading: if its site needs scriptlets (YouTube ads, etc.)
/// but they were registered too late for this document, reload once so they run.
unsafe fn verify_scriptlets(core: &ICoreWebView2, ctx: &Rc<RefCell<Ctx>>) {
    let mut uri = PWSTR::null();
    if core.Source(&mut uri).is_err() {
        return;
    }
    let url = take(uri);
    let Some(host) = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_string)) else { return };
    {
        let c = ctx.borrow();
        if !c.scriptlet_hosts.contains(&host) || c.reloaded_for.contains(&host) {
            return;
        }
    }
    let (ctx2, core2) = (ctx.clone(), core.clone());
    let check = format!("window.__sericaCosmetic === {}", js_string(&host));
    let _ = core.ExecuteScript(
        &HSTRING::from(check),
        &ExecuteScriptCompletedHandler::create(Box::new(move |hr, result| {
            if hr.is_ok() && result.trim() == "false" && ctx2.borrow_mut().reloaded_for.insert(host.clone()) {
                tracing::info!("scriptlets missed the first load of {host}; reloading once");
                let _ = core2.Reload();
            }
            Ok(())
        })),
    );
}

unsafe fn read_theme_color(app: &Shared, tab_id: &str, core: &ICoreWebView2) {
    const JS: &str = r#"(() => { try { const ms = Array.from(document.querySelectorAll('meta[name="theme-color" i]')); const m = ms.find((el) => !el.media || matchMedia(el.media).matches); const v = m && m.getAttribute('content'); return typeof v === 'string' ? v.trim() : null; } catch (e) { return null; } })()"#;
    let (app, id) = (app.clone(), tab_id.to_string());
    let _ = core.ExecuteScript(
        &HSTRING::from(JS),
        &ExecuteScriptCompletedHandler::create(Box::new(move |hr, result| {
            if hr.is_ok() {
                let color = serde_json::from_str::<serde_json::Value>(&result).ok().and_then(|v| v.as_str().map(str::to_string));
                ui_emit(&app.handle, "serica:page-color", serde_json::json!({ "tabId": id, "themeColor": color }));
            }
            Ok(())
        })),
    );
}

fn decode_data_url(u: &str) -> Option<(String, Vec<u8>)> {
    let rest = u.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let mime = meta.split(';').next().filter(|m| !m.is_empty()).unwrap_or("text/plain").to_string();
    let body = if meta.ends_with(";base64") {
        base64::engine::general_purpose::STANDARD.decode(data).ok()?
    } else {
        url::form_urlencoded::parse(data.as_bytes()).map(|(k, _)| k.into_owned()).collect::<String>().into_bytes()
    };
    Some((mime, body))
}

unsafe fn read_stream(stream: &IStream) -> Vec<u8> {
    let _ = stream.Seek(0, STREAM_SEEK_SET, None);
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let mut read = 0u32;
        let hr = stream.Read(buf.as_mut_ptr() as *mut _, buf.len() as u32, Some(&mut read));
        if hr.is_err() || read == 0 {
            break;
        }
        out.extend_from_slice(&buf[..read as usize]);
    }
    out
}

/// Captures the webview's current frame as a JPEG data URL.
pub fn capture(wv: &Webview, done: impl FnOnce(Option<String>) + Send + 'static) {
    let done = std::sync::Mutex::new(Some(done));
    let r = wv.with_webview(move |pw| unsafe {
        let finish = move |v: Option<String>| {
            if let Some(f) = done.lock().ok().and_then(|mut d| d.take()) {
                f(v);
            }
        };
        let Ok(core) = pw.controller().CoreWebView2() else { return finish(None) };
        let Ok(stream) = windows::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal(Default::default(), true) else {
            return finish(None);
        };
        let s2 = stream.clone();
        let finish = std::rc::Rc::new(RefCell::new(Some(finish)));
        let f2 = finish.clone();
        let started = core.CapturePreview(
            COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_JPEG,
            &stream,
            &CapturePreviewCompletedHandler::create(Box::new(move |hr| {
                let out = if hr.is_ok() {
                    let bytes = read_stream(&s2);
                    (!bytes.is_empty()).then(|| format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
                } else {
                    None
                };
                if let Some(f) = f2.borrow_mut().take() {
                    f(out);
                }
                Ok(())
            })),
        );
        if started.is_err() {
            if let Some(f) = finish.borrow_mut().take() {
                f(None);
            }
        }
    });
    if r.is_err() {
        tracing::warn!("capture dispatch failed");
    }
}

/// Starts, continues or stops native find-in-page.
pub fn find(wv: &Webview, query: Option<String>, forward: bool, next: bool) {
    let _ = wv.with_webview(move |pw| unsafe {
        let Ok(core) = pw.controller().CoreWebView2() else { return };
        let Ok(c28) = core.cast::<ICoreWebView2_28>() else { return };
        let Ok(find) = c28.Find() else { return };
        match query {
            None => {
                let _ = find.Stop();
            }
            Some(_) if next => {
                let _ = if forward { find.FindNext() } else { find.FindPrevious() };
            }
            Some(q) => {
                let Ok(env15) = pw.environment().cast::<ICoreWebView2Environment15>() else { return };
                let Ok(opts) = env15.CreateFindOptions() else { return };
                let _ = opts.SetFindTerm(&HSTRING::from(q));
                let _ = opts.SetSuppressDefaultFindDialog(true);
                let _ = opts.SetShouldHighlightAllMatches(true);
                let _ = find.Start(&opts, &FindStartCompletedHandler::create(Box::new(|_| Ok(()))));
            }
        }
    });
}

/// Runs `js` in the page and passes the JSON-encoded result to `done`.
pub fn eval(wv: &Webview, js: String, done: impl FnOnce(Option<serde_json::Value>) + Send + 'static) {
    let done = std::sync::Mutex::new(Some(done));
    let _ = wv.with_webview(move |pw| unsafe {
        let finish = move |v| {
            if let Some(f) = done.lock().ok().and_then(|mut d| d.take()) {
                f(v);
            }
        };
        let Ok(core) = pw.controller().CoreWebView2() else { return finish(None) };
        let finish = Rc::new(RefCell::new(Some(finish)));
        let f2 = finish.clone();
        let r = core.ExecuteScript(
            &HSTRING::from(js),
            &ExecuteScriptCompletedHandler::create(Box::new(move |hr, result| {
                let v = if hr.is_ok() { serde_json::from_str(&result).ok() } else { None };
                if let Some(f) = f2.borrow_mut().take() {
                    f(v);
                }
                Ok(())
            })),
        );
        if r.is_err() {
            if let Some(f) = finish.borrow_mut().take() {
                f(None);
            }
        }
    });
}

/// Navigation helpers that need the native object.
pub fn nav(wv: &Webview, action: &'static str) {
    let _ = wv.with_webview(move |pw| unsafe {
        let Ok(core) = pw.controller().CoreWebView2() else { return };
        let _ = match action {
            "back" => core.GoBack(),
            "forward" => core.GoForward(),
            "stop" => core.Stop(),
            "reload" => core.Reload(),
            "hard-reload" => core
                .CallDevToolsProtocolMethod(&HSTRING::from("Page.reload"), &HSTRING::from(r#"{"ignoreCache":true}"#), None),
            _ => Ok(()),
        };
    });
}

pub fn set_zoom(wv: &Webview, factor: f64) {
    let _ = wv.with_webview(move |pw| unsafe {
        let _ = pw.controller().SetZoomFactor(factor);
    });
}

/// Stock WebView2 menu entries that don't fit a browser: Edge-only features
/// and "new window" (Serica opens tabs).
const HIDDEN_MENU_ITEMS: &[&str] =
    &["openLinkInNewWindow", "openLinkInProfile", "createQrCode", "copyLinkToHighlight", "share", "webCapture", "readAloud", "translate"];

fn search_engine_label(id: &str) -> &'static str {
    match id {
        "duckduckgo" => "DuckDuckGo",
        "bing" => "Bing",
        "brave" => "Brave",
        _ => "Google",
    }
}

/// A page context menu shown by the UI and waiting for the user's pick.
/// COM objects live on the UI thread only, so this map is thread-local.
struct PendingMenu {
    args: ICoreWebView2ContextMenuRequestedEventArgs,
    deferral: ICoreWebView2Deferral,
    controller: ICoreWebView2Controller,
    tab_id: String,
    /// Serica's own entries: (url, open in foreground).
    custom: Vec<(String, bool)>,
}

thread_local! {
    static PENDING_MENUS: RefCell<HashMap<u64, PendingMenu>> = RefCell::new(HashMap::new());
}
static NEXT_MENU_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Strips Windows menu mnemonics ("Cu&t" -> "Cut", "&&" -> "&").
fn menu_label(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                out.push('&');
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The engine's menu items as JSON for the UI, without Edge-only entries.
unsafe fn native_items(items: &ICoreWebView2ContextMenuItemCollection, editable: bool) -> windows::core::Result<Vec<serde_json::Value>> {
    let mut out = Vec::new();
    let mut n = 0u32;
    items.Count(&mut n)?;
    for i in 0..n {
        let item = items.GetValueAtIndex(i)?;
        let mut p = PWSTR::null();
        item.Name(&mut p)?;
        let name = take(p);
        if HIDDEN_MENU_ITEMS.contains(&name.as_str()) || (name == "other" && !editable) {
            continue;
        }
        let mut kind = COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND::default();
        item.Kind(&mut kind)?;
        if kind == COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR {
            out.push(serde_json::json!({ "type": "separator" }));
            continue;
        }
        let mut p = PWSTR::null();
        item.Label(&mut p)?;
        let label = menu_label(&take(p));
        let mut p = PWSTR::null();
        let _ = item.ShortcutKeyDescription(&mut p);
        let shortcut = take(p);
        let (mut enabled, mut checked) = (Default::default(), Default::default());
        item.IsEnabled(&mut enabled)?;
        item.IsChecked(&mut checked)?;
        let mut id = 0i32;
        item.CommandId(&mut id)?;
        let submenu = kind == COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SUBMENU;
        let children = if submenu { native_items(&item.Children()?, editable)? } else { Vec::new() };
        if submenu && children.is_empty() {
            continue;
        }
        out.push(serde_json::json!({
            "type": if submenu { "submenu" } else { "command" },
            "id": id,
            "name": name,
            "label": label,
            "shortcut": shortcut,
            "enabled": enabled.as_bool(),
            "checked": checked.as_bool(),
            "children": children,
        }));
    }
    // No doubled, leading or trailing separators after the removals.
    let mut clean: Vec<serde_json::Value> = Vec::with_capacity(out.len());
    for v in out {
        let sep = v["type"] == "separator";
        if sep && clean.last().map_or(true, |l| l["type"] == "separator") {
            continue;
        }
        clean.push(v);
    }
    if clean.last().is_some_and(|l| l["type"] == "separator") {
        clean.pop();
    }
    Ok(clean)
}

/// Shows Serica's own menu for a right-click in a page. The engine's menu is
/// suppressed (`Handled`); its items (copy, paste, spelling, inspect...) are
/// sent to the UI, which draws the menu and reports the pick through
/// `finish_page_menu`. A native pick runs as the engine's own command, so
/// editing, spelling and clipboard behave exactly as in the stock menu.
unsafe fn install_context_menu(
    app: &Shared,
    tab_id: &str,
    core: &ICoreWebView2,
    controller: &ICoreWebView2Controller,
) -> windows::core::Result<()> {
    let c11 = core.cast::<ICoreWebView2_11>()?;
    let (app, tab_id, controller) = (app.clone(), tab_id.to_string(), controller.clone());
    let mut token = Default::default();
    c11.add_ContextMenuRequested(
        &ContextMenuRequestedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let items = args.MenuItems()?;
            let target = args.ContextMenuTarget()?;
            let mut editable = Default::default();
            target.IsEditable(&mut editable)?;
            let editable = editable.as_bool();

            let mut has = Default::default();
            let mut p = PWSTR::null();
            target.HasLinkUri(&mut has)?;
            let link = if has.as_bool() && target.LinkUri(&mut p).is_ok() { Some(take(p)) } else { None };
            target.HasSelection(&mut has)?;
            let mut p = PWSTR::null();
            let selection = if has.as_bool() && target.SelectionText(&mut p).is_ok() {
                Some(take(p).split_whitespace().collect::<Vec<_>>().join(" ")).filter(|s| !s.is_empty())
            } else {
                None
            };
            let mut kind = COREWEBVIEW2_CONTEXT_MENU_TARGET_KIND::default();
            target.Kind(&mut kind)?;
            target.HasSourceUri(&mut has)?;
            let mut p = PWSTR::null();
            let image = if has.as_bool() && kind == COREWEBVIEW2_CONTEXT_MENU_TARGET_KIND_IMAGE && target.SourceUri(&mut p).is_ok() {
                Some(take(p))
            } else {
                None
            };
            let mut p = PWSTR::null();
            target.PageUri(&mut p)?;
            let page = take(p);

            // Serica's entries: (label, url, foreground).
            let mut top: Vec<(String, String, bool)> = Vec::new();
            if let Some(l) = link.as_ref().filter(|l| l.starts_with("http") || l.starts_with("file:")) {
                top.push(("Open link in new tab".into(), l.clone(), true));
                top.push(("Open link in background tab".into(), l.clone(), false));
            }
            if let Some(src) = image.as_ref().filter(|s| s.starts_with("http")) {
                top.push(("Open image in new tab".into(), src.clone(), true));
            }
            if let Some(sel) = selection.as_ref().filter(|_| !editable) {
                let settings = app.settings();
                let short: String = if sel.chars().count() > 32 { sel.chars().take(30).collect::<String>() + "\u{2026}" } else { sel.clone() };
                let q: String = url::form_urlencoded::byte_serialize(sel.as_bytes()).collect();
                top.push((
                    format!("Search {} for \u{201c}{short}\u{201d}", search_engine_label(&settings.search_engine)),
                    format!("{}{q}", settings.search_prefix()),
                    true,
                ));
            }
            let source = (link.is_none() && image.is_none() && selection.is_none() && !editable && page.starts_with("http"))
                .then(|| ("View page source".to_string(), format!("view-source:{page}"), true));

            let mut list: Vec<serde_json::Value> = Vec::new();
            let mut custom: Vec<(String, bool)> = Vec::new();
            for (label, url, fg) in &top {
                list.push(serde_json::json!({ "type": "custom", "index": custom.len(), "label": label, "name": "serica" }));
                custom.push((url.clone(), *fg));
            }
            let native = native_items(&items, editable)?;
            if !list.is_empty() && !native.is_empty() {
                list.push(serde_json::json!({ "type": "separator" }));
            }
            let inspect_at = native.iter().position(|v| v["name"] == "inspectElement");
            for (i, v) in native.into_iter().enumerate() {
                if Some(i) == inspect_at {
                    if let Some((label, url, fg)) = &source {
                        list.push(serde_json::json!({ "type": "custom", "index": custom.len(), "label": label, "name": "viewSource" }));
                        custom.push((url.clone(), *fg));
                    }
                }
                list.push(v);
            }
            if list.is_empty() {
                return Ok(()); // nothing to offer: leave it to the engine
            }

            // Where the click was, in physical pixels of the window's client area.
            let mut pt = windows::Win32::Foundation::POINT::default();
            args.Location(&mut pt)?;
            // `Location` is in DIPs, `Bounds` in physical pixels.
            let mut bounds = windows::Win32::Foundation::RECT::default();
            controller.Bounds(&mut bounds)?;
            let mut scale = 1.0f64;
            if let Ok(c3) = controller.cast::<ICoreWebView2Controller3>() {
                let _ = c3.RasterizationScale(&mut scale);
            }

            // One menu at a time: a new request ends any earlier one unpicked.
            PENDING_MENUS.with(|m| {
                for (_, old) in m.borrow_mut().drain() {
                    let _ = old.deferral.Complete();
                }
            });
            let deferral = args.GetDeferral()?;
            args.SetHandled(true)?;
            let id = NEXT_MENU_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            PENDING_MENUS.with(|m| {
                m.borrow_mut().insert(
                    id,
                    PendingMenu { args: args.clone(), deferral, controller: controller.clone(), tab_id: tab_id.clone(), custom },
                )
            });
            let context = if editable {
                "editable"
            } else if link.is_some() {
                "link"
            } else if image.is_some() {
                "image"
            } else if selection.is_some() {
                "selection"
            } else {
                "page"
            };
            ui_emit(
                &app.handle,
                "serica:page-context-menu",
                serde_json::json!({
                    "id": id,
                    "tabId": tab_id,
                    "x": bounds.left as f64 + pt.x as f64 * scale,
                    "y": bounds.top as f64 + pt.y as f64 * scale,
                    "context": context,
                    "link": link,
                    "image": image,
                    "selection": selection,
                    "items": list,
                }),
            );
            // The menu takes keyboard focus (arrows, Enter, Escape); a click
            // back into the page then blurs the UI, which closes it.
            if let Some(main) = tauri::Manager::get_webview(&app.handle, "main") {
                let _ = main.set_focus();
            }
            Ok(())
        })),
        &mut token,
    )?;
    Ok(())
}

/// Ends a page menu: runs the picked engine command (`command`) or Serica
/// entry (`custom`), or nothing when the menu was dismissed. UI thread only.
/// `refocus` hands keyboard focus back to the page (a pick, or Escape).
pub fn finish_page_menu(app: &Shared, id: u64, command: Option<i32>, custom: Option<usize>, refocus: bool) {
    let Some(menu) = PENDING_MENUS.with(|m| m.borrow_mut().remove(&id)) else { return };
    unsafe {
        if command.is_some() || refocus {
            // The UI had focus while its menu was open; commands such as
            // paste act on the page's focused field.
            let _ = menu.controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        }
        if let Some(cmd) = command {
            let _ = menu.args.SetSelectedCommandId(cmd);
        }
        let _ = menu.deferral.Complete();
    }
    if let Some((url, foreground)) = custom.and_then(|i| menu.custom.get(i).cloned()) {
        let tab_id = menu.tab_id;
        app.mutate(move |s| {
            let space = s.tab(&tab_id).map(|t| t.space_id.clone());
            s.create_tab(Some(url), space, foreground, Some(&tab_id));
        });
    }
}

/// Hidden tabs ask WebView2 to keep a low memory footprint (it trims caches
/// and swaps out what it can); visible tabs go back to normal.
pub fn set_background(wv: &Webview, background: bool) {
    let _ = wv.with_webview(move |pw| unsafe {
        if let Ok(c19) = pw.controller().CoreWebView2().and_then(|c| c.cast::<ICoreWebView2_19>()) {
            let level = if background { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW } else { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL };
            let _ = c19.SetMemoryUsageTargetLevel(level);
        }
    });
}

/// Freezes a hidden tab's renderer (timers, scripts) until it is shown again.
/// WebView2 refuses while the page plays audio or holds other live activity.
pub fn try_suspend(wv: &Webview) {
    let _ = wv.with_webview(move |pw| unsafe {
        if let Ok(c3) = pw.controller().CoreWebView2().and_then(|c| c.cast::<ICoreWebView2_3>()) {
            let _ = c3.TrySuspend(&TrySuspendCompletedHandler::create(Box::new(|_, _| Ok(()))));
        }
    });
}

// ------------------------------------------------------------------ cookies
//
// Cookies belong to a WebView2 profile (one per Serica profile), so any page
// webview of that profile reaches all of them. Calls run on the UI thread and
// report through `done`.

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

/// Reads every cookie of the profile, then runs `f` with them.
unsafe fn with_cookies(
    core: &ICoreWebView2,
    f: impl FnOnce(&ICoreWebView2CookieManager, Vec<(ICoreWebView2Cookie, String)>) + 'static,
) -> windows::core::Result<()> {
    let mgr = core.cast::<ICoreWebView2_2>()?.CookieManager()?;
    let m2 = mgr.clone();
    let handler = GetCookiesCompletedHandler::create(Box::new(move |_, list| {
        let mut out = Vec::new();
        if let Some(list) = list {
            let mut n = 0u32;
            list.Count(&mut n)?;
            for i in 0..n {
                if let Ok(c) = list.GetValueAtIndex(i) {
                    let mut d = PWSTR::null();
                    if c.Domain(&mut d).is_ok() {
                        out.push((c, take(d)));
                    }
                }
            }
        }
        f(&m2, out);
        Ok(())
    }));
    // An empty URI lists every cookie of the profile.
    mgr.GetCookies(&HSTRING::new(), &handler)
}

/// Sites with cookies in `wv`'s profile and how many each has, most first.
pub fn cookie_sites(wv: &Webview, done: impl FnOnce(Vec<(String, usize)>) + Send + 'static) {
    let _ = wv.with_webview(move |pw| unsafe {
        let Ok(core) = pw.controller().CoreWebView2() else { return done(Vec::new()) };
        let done = std::cell::Cell::new(Some(done));
        let r = with_cookies(&core, {
            let done = done.take();
            move |_, cookies| {
                let mut by_site: HashMap<String, usize> = HashMap::new();
                for (_, domain) in &cookies {
                    *by_site.entry(cookie_site(domain)).or_default() += 1;
                }
                let mut v: Vec<(String, usize)> = by_site.into_iter().collect();
                v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                if let Some(done) = done {
                    done(v);
                }
            }
        });
        if r.is_err() {
            if let Some(done) = done.take() {
                done(Vec::new());
            }
        }
    });
}

/// Deletes the cookies of `wv`'s profile that `delete` picks (by domain);
/// reports how many went.
pub fn delete_cookies(wv: &Webview, delete: impl Fn(&str) -> bool + Send + 'static, done: impl FnOnce(usize) + Send + 'static) {
    let _ = wv.with_webview(move |pw| unsafe {
        let Ok(core) = pw.controller().CoreWebView2() else { return done(0) };
        sweep(&core, delete, done);
    });
}

/// Deletes the cookies `delete` picks, on the UI thread, then runs `done`.
unsafe fn sweep(core: &ICoreWebView2, delete: impl Fn(&str) -> bool + 'static, done: impl FnOnce(usize) + 'static) {
    let done = Rc::new(RefCell::new(Some(done)));
    let d2 = done.clone();
    let r = with_cookies(core, move |mgr, cookies| {
        let mut n = 0;
        for (c, domain) in &cookies {
            if delete(domain) && mgr.DeleteCookie(c).is_ok() {
                n += 1;
            }
        }
        if let Some(done) = d2.borrow_mut().take() {
            done(n);
        }
    });
    if r.is_err() {
        if let Some(done) = done.borrow_mut().take() {
            done(0);
        }
    }
}

// ------------------------------------------------------------------ page dialogs

/// A page dialog (alert/confirm/prompt/"Leave site?") or an "Open in app?"
/// request waiting for the user's answer in Serica's own UI.
enum Pending {
    Script(ICoreWebView2ScriptDialogOpeningEventArgs, ICoreWebView2Deferral),
    External(String, ICoreWebView2Deferral),
}

thread_local! {
    static DIALOGS: RefCell<HashMap<u64, Pending>> = RefCell::new(HashMap::new());
}
static NEXT_DIALOG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn origin_of(uri: &str) -> String {
    url::Url::parse(uri)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string()))
        .unwrap_or_else(|| uri.to_string())
}

/// Replaces WebView2's built-in dialogs with events the UI renders in Serica's style.
unsafe fn install_dialogs(app: &Shared, tab_id: &str, core: &ICoreWebView2) -> windows::core::Result<()> {
    let mut token = Default::default();
    if let Ok(settings) = core.Settings() {
        let _ = settings.SetAreDefaultScriptDialogsEnabled(false);
    }
    {
        let (app, id) = (app.clone(), tab_id.to_string());
        core.add_ScriptDialogOpening(
            &ScriptDialogOpeningEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_SCRIPT_DIALOG_KIND::default();
                args.Kind(&mut kind)?;
                let kind = match kind {
                    COREWEBVIEW2_SCRIPT_DIALOG_KIND_CONFIRM => "confirm",
                    COREWEBVIEW2_SCRIPT_DIALOG_KIND_PROMPT => "prompt",
                    COREWEBVIEW2_SCRIPT_DIALOG_KIND_BEFOREUNLOAD => "beforeunload",
                    _ => "alert",
                };
                let (mut uri, mut msg, mut def) = (PWSTR::null(), PWSTR::null(), PWSTR::null());
                args.Uri(&mut uri)?;
                args.Message(&mut msg)?;
                args.DefaultText(&mut def)?;
                let (uri, msg, def) = (take(uri), take(msg), take(def));
                let deferral = args.GetDeferral()?;
                let dialog_id = NEXT_DIALOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                DIALOGS.with(|d| d.borrow_mut().insert(dialog_id, Pending::Script(args.clone(), deferral)));
                ui_emit(
                    &app.handle,
                    "serica:page-dialog",
                    serde_json::json!({
                        "id": dialog_id, "tabId": id, "kind": kind,
                        "origin": origin_of(&uri),
                        "message": msg.chars().take(2000).collect::<String>(),
                        "defaultText": def,
                    }),
                );
                Ok(())
            })),
            &mut token,
        )?;
    }
    if let Ok(core18) = core.cast::<ICoreWebView2_18>() {
        let (app, id) = (app.clone(), tab_id.to_string());
        core18.add_LaunchingExternalUriScheme(
            &LaunchingExternalUriSchemeEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                // Serica asks (and launches) itself; WebView2's own prompt never shows.
                args.SetCancel(true)?;
                let (mut uri, mut from) = (PWSTR::null(), PWSTR::null());
                args.Uri(&mut uri)?;
                args.InitiatingOrigin(&mut from)?;
                let (uri, from) = (take(uri), take(from));
                let scheme = uri.split(':').next().unwrap_or_default().to_ascii_lowercase();
                if scheme.is_empty() || matches!(scheme.as_str(), "javascript" | "file" | "data" | "vbscript") {
                    return Ok(());
                }
                let deferral = args.GetDeferral()?;
                let dialog_id = NEXT_DIALOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                DIALOGS.with(|d| d.borrow_mut().insert(dialog_id, Pending::External(uri.clone(), deferral)));
                ui_emit(
                    &app.handle,
                    "serica:page-dialog",
                    serde_json::json!({
                        "id": dialog_id, "tabId": id, "kind": "external",
                        "origin": origin_of(&from),
                        "message": uri.chars().take(300).collect::<String>(),
                        "scheme": scheme,
                    }),
                );
                Ok(())
            })),
            &mut token,
        )?;
    }
    Ok(())
}

// ------------------------------------------------------------------ site permissions

/// Serica's name for a WebView2 permission kind; `None` for kinds the engine
/// decides on its own (autoplay follows Chromium's policy).
fn permission_name(kind: COREWEBVIEW2_PERMISSION_KIND) -> Option<&'static str> {
    Some(match kind {
        COREWEBVIEW2_PERMISSION_KIND_CAMERA => "camera",
        COREWEBVIEW2_PERMISSION_KIND_MICROPHONE => "microphone",
        COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION => "location",
        COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS => "notifications",
        COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ => "clipboard",
        COREWEBVIEW2_PERMISSION_KIND_MULTIPLE_AUTOMATIC_DOWNLOADS => "downloads",
        COREWEBVIEW2_PERMISSION_KIND_FILE_READ_WRITE => "files",
        COREWEBVIEW2_PERMISSION_KIND_OTHER_SENSORS => "sensors",
        COREWEBVIEW2_PERMISSION_KIND_LOCAL_FONTS => "fonts",
        COREWEBVIEW2_PERMISSION_KIND_MIDI_SYSTEM_EXCLUSIVE_MESSAGES => "midi",
        COREWEBVIEW2_PERMISSION_KIND_WINDOW_MANAGEMENT => "windows",
        _ => return None,
    })
}

/// `https://host[:port]` of a page, the unit permissions are kept for.
pub fn permission_origin(uri: &str) -> Option<String> {
    let u = url::Url::parse(uri).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    Some(u.origin().ascii_serialization())
}

struct PendingPermission {
    args: ICoreWebView2PermissionRequestedEventArgs,
    deferral: ICoreWebView2Deferral,
    origin: String,
    kind: &'static str,
}

thread_local! {
    static PERMISSIONS: RefCell<HashMap<u64, PendingPermission>> = RefCell::new(HashMap::new());
}
static NEXT_PERMISSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Asks for camera, microphone, location and the other permissions in
/// Serica's own prompt instead of the engine's (Edge-styled) one. Answers
/// kept as "always" live in Serica's settings, not in the WebView2 profile,
/// so they can be listed and revoked in Settings › Privacy.
unsafe fn install_permissions(app: &Shared, tab_id: &str, core: &ICoreWebView2) -> windows::core::Result<()> {
    let mut token = Default::default();
    let (app, id) = (app.clone(), tab_id.to_string());
    core.add_PermissionRequested(
        &PermissionRequestedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
            args.PermissionKind(&mut kind)?;
            let Some(name) = permission_name(kind) else { return Ok(()) };
            let mut uri = PWSTR::null();
            args.Uri(&mut uri)?;
            let uri = take(uri);
            // Never let the engine remember an answer behind Serica's back.
            if let Ok(a3) = args.cast::<ICoreWebView2PermissionRequestedEventArgs3>() {
                let _ = a3.SetSavesInProfile(false);
            }
            let Some(origin) = permission_origin(&uri) else {
                // Not a web origin (file:, data:): nothing to ask about.
                args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                return Ok(());
            };
            if let Some(allow) = app.settings().permission(&origin, name) {
                args.SetState(if allow { COREWEBVIEW2_PERMISSION_STATE_ALLOW } else { COREWEBVIEW2_PERMISSION_STATE_DENY })?;
                return Ok(());
            }
            let mut user = Default::default();
            let _ = args.IsUserInitiated(&mut user);
            let deferral = args.GetDeferral()?;
            let request_id = NEXT_PERMISSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            PERMISSIONS.with(|p| p.borrow_mut().insert(request_id, PendingPermission { args: args.clone(), deferral, origin: origin.clone(), kind: name }));
            ui_emit(
                &app.handle,
                "serica:permission-request",
                serde_json::json!({
                    "id": request_id,
                    "tabId": id,
                    "origin": origin,
                    "host": origin_of(&uri),
                    "kind": name,
                    "userInitiated": user.as_bool(),
                }),
            );
            Ok(())
        })),
        &mut token,
    )?;
    Ok(())
}

/// Answers a permission request: "allow" / "block" (remembered for the
/// site), "allow-once", or "deny" (this time only, also for a dismissed
/// prompt). Must run on the UI thread.
pub fn answer_permission(app: &Shared, id: u64, decision: &str) {
    let Some(p) = PERMISSIONS.with(|m| m.borrow_mut().remove(&id)) else { return };
    let allow = matches!(decision, "allow" | "allow-once");
    unsafe {
        let _ = p.args.SetState(if allow { COREWEBVIEW2_PERMISSION_STATE_ALLOW } else { COREWEBVIEW2_PERMISSION_STATE_DENY });
        let _ = p.deferral.Complete();
    }
    if matches!(decision, "allow" | "block") {
        let changed = app.store.lock().settings.remember_permission(&p.origin, p.kind, Some(decision == "allow"));
        if changed {
            app.notify(crate::app::Notice::Settings);
        }
    }
}

/// Permissions and Focus Mode for a webview that isn't a tab (the assistant drawer).
pub fn attach_permissions(app: &Shared, owner: &str, wv: &Webview) {
    let (app, owner) = (app.clone(), owner.to_string());
    let _ = wv.with_webview(move |pw| unsafe {
        let env = pw.environment();
        let Ok(core) = pw.controller().CoreWebView2() else { return };
        if let Err(e) = install_permissions(&app, &owner, &core) {
            tracing::warn!("permission hook install failed: {e}");
        }
        let filter = HSTRING::from("*");
        if core.AddWebResourceRequestedFilter(&filter, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT).is_err() {
            return;
        }
        let mut token = Default::default();
        let app2 = app.clone();
        let _ = core.add_WebResourceRequested(
            &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut uri = PWSTR::null();
                args.Request()?.Uri(&mut uri)?;
                let uri = take(uri);
                if let Some(hit) = app2.focus.check(&uri) {
                    app2.focus_stats.blocked(&uri);
                    args.SetResponse(&focus_response(&env, &hit, &uri)?)?;
                }
                Ok(())
            })),
            &mut token,
        );
    });
}

/// Answers a dialog raised by `install_dialogs`. Must run on the UI thread.
pub fn answer_dialog(id: u64, accept: bool, text: Option<String>) {
    let Some(p) = DIALOGS.with(|d| d.borrow_mut().remove(&id)) else { return };
    unsafe {
        match p {
            Pending::Script(args, deferral) => {
                if accept {
                    if let Some(t) = text {
                        let _ = args.SetResultText(&HSTRING::from(t));
                    }
                    let _ = args.Accept();
                }
                let _ = deferral.Complete();
            }
            Pending::External(uri, deferral) => {
                let _ = deferral.Complete();
                if accept {
                    let _ = open::that_detached(uri);
                }
            }
        }
    }
}

// ------------------------------------------------------------------ error page

/// Replaces WebView2's "Hmmm… can't reach this page" with Serica's own page.
/// The error document stays in place, so the address bar, back/forward and
/// reload keep working; it retries by itself when the connection returns.
unsafe fn show_error_page(core: &ICoreWebView2, status: COREWEBVIEW2_WEB_ERROR_STATUS, url: &str) {
    let (kind, title, body) = match status {
        COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED => (
            "offline",
            "You're offline",
            "Serica can't reach the internet right now. Check your Wi-Fi or network cable — this page will reload as soon as you're back online.",
        ),
        COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED => (
            "dns",
            "This site can't be found",
            "The address might be mistyped, or the site might no longer exist. If you're offline, the page will reload once you reconnect.",
        ),
        COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT => (
            "timeout",
            "This site took too long to respond",
            "The server may be busy or your connection may be slow. Try again in a moment.",
        ),
        COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT
        | COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED
        | COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET
        | COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE => (
            "unreachable",
            "This site can't be reached",
            "The connection was interrupted or refused. Check your connection and try again.",
        ),
        // Certificate pages carry WebView2's own "continue anyway" flow; other
        // statuses (cancelled navigations, downloads) are not network failures.
        _ => return,
    };
    let data = serde_json::json!({ "kind": kind, "title": title, "body": body, "url": url, "host": origin_of(url) });
    let js = ERROR_PAGE_JS.replace("__DATA__", &data.to_string());
    let _ = core.ExecuteScript(&HSTRING::from(js), &ExecuteScriptCompletedHandler::create(Box::new(|_, _| Ok(()))));
}

const ERROR_PAGE_JS: &str = r#"(() => {
  if (!location.href.startsWith('chrome-error://')) return;
  const d = __DATA__;
  const dark = matchMedia('(prefers-color-scheme: dark)').matches;
  const icons = {
    offline: '<path d="M2 8.8a15 15 0 0 1 4.2-2.6"/><path d="M10.7 5.1A15 15 0 0 1 22 8.8"/><path d="M5 12.6a10 10 0 0 1 5.2-2.7"/><path d="M16.8 11.2A10 10 0 0 1 19 12.6"/><path d="M8.5 16.4a5 5 0 0 1 7 0"/><path d="M12 20h.01"/><path d="m2 2 20 20"/>',
    dns: '<circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/><path d="M8.5 8.5l5 5M13.5 8.5l-5 5"/>',
    timeout: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
    unreachable: '<circle cx="12" cy="12" r="9"/><path d="M12 8v5"/><path d="M12 16h.01"/>',
  };
  document.documentElement.innerHTML = `<head><meta charset="utf-8"><meta name="color-scheme" content="light dark"><title></title><style>
    :root { --bg: ${dark ? '#111113' : '#f6f6f7'}; --card: ${dark ? '#1b1b1e' : '#ffffff'}; --fg: ${dark ? '#ececef' : '#17171a'};
            --muted: ${dark ? '#9b9ba3' : '#6b6b73'}; --line: ${dark ? '#2a2a2f' : '#e6e6ea'}; --accent: ${dark ? '#ececef' : '#17171a'}; --on-accent: ${dark ? '#111113' : '#ffffff'}; }
    * { box-sizing: border-box; margin: 0; }
    html, body { height: 100%; }
    body { background: var(--bg); color: var(--fg); font: 14px/1.55 "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;
           display: grid; place-items: center; padding: 24px; -webkit-font-smoothing: antialiased; }
    main { width: min(440px, 100%); text-align: left; animation: in .35s cubic-bezier(.2,.8,.2,1); }
    @keyframes in { from { opacity: 0; transform: translateY(6px); } }
    .icon { width: 52px; height: 52px; border-radius: 14px; background: var(--card); border: 1px solid var(--line);
            display: grid; place-items: center; margin-bottom: 22px; }
    .icon svg { width: 24px; height: 24px; stroke: var(--fg); fill: none; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
    h1 { font: 600 22px/1.25 "Segoe UI Variable Display", "Segoe UI", system-ui, sans-serif; letter-spacing: -.01em; margin-bottom: 8px; }
    p { color: var(--muted); }
    .host { display: inline-block; margin: 14px 0 26px; padding: 3px 10px; border-radius: 999px; background: var(--card);
            border: 1px solid var(--line); color: var(--muted); font-size: 12.5px; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .row { display: flex; align-items: center; gap: 14px; }
    button { font: 600 13px "Segoe UI", system-ui, sans-serif; border: 0; border-radius: 10px; padding: 9px 18px; cursor: pointer;
             background: var(--accent); color: var(--on-accent); transition: opacity .15s, transform .1s; }
    button:hover { opacity: .88; } button:active { transform: scale(.98); }
    .status { font-size: 12.5px; color: var(--muted); display: flex; align-items: center; gap: 7px; }
    .dot { width: 7px; height: 7px; border-radius: 50%; background: #d9534f; }
    .dot.on { background: #3fb950; }
    .code { margin-top: 34px; font: 11.5px ui-monospace, "Cascadia Mono", Consolas, monospace; color: var(--muted); opacity: .7; }
  </style></head><body><main>
    <div class="icon"><svg viewBox="0 0 24 24">${icons[d.kind] || icons.unreachable}</svg></div>
    <h1></h1><p class="body"></p><div class="host"></div>
    <div class="row"><button id="retry">Try again</button><span class="status"><span class="dot"></span><span class="st"></span></span></div>
    <div class="code"></div>
  </main></body>`;
  document.title = d.host || d.title;
  document.querySelector('h1').textContent = d.title;
  document.querySelector('.body').textContent = d.body;
  document.querySelector('.host').textContent = d.host || d.url;
  document.querySelector('.code').textContent = { offline: 'ERR_INTERNET_DISCONNECTED', dns: 'ERR_NAME_NOT_RESOLVED', timeout: 'ERR_TIMED_OUT' }[d.kind] || 'ERR_CONNECTION_FAILED';
  const retry = () => location.replace(d.url);
  document.getElementById('retry').onclick = retry;
  const status = () => {
    const on = navigator.onLine;
    document.querySelector('.dot').classList.toggle('on', on);
    document.querySelector('.st').textContent = on ? 'Connected' : 'No internet connection';
  };
  status();
  addEventListener('offline', status);
  addEventListener('online', () => { status(); setTimeout(retry, 600); });
})()"#;

pub fn set_muted(wv: &Webview, muted: bool) {
    let _ = wv.with_webview(move |pw| unsafe {
        if let Ok(c8) = pw.controller().CoreWebView2().and_then(|c| c.cast::<ICoreWebView2_8>()) {
            let _ = c8.SetIsMuted(muted);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_url_decoding() {
        let (m, b) = decode_data_url("data:application/javascript;base64,KGZ1bmN0aW9uKCl7fSkoKQ==").unwrap();
        assert_eq!(m, "application/javascript");
        assert_eq!(b, b"(function(){})()");
        let (m, b) = decode_data_url("data:text/plain,").unwrap();
        assert_eq!(m, "text/plain");
        assert!(b.is_empty());
    }

    #[test]
    fn shortcuts() {
        assert!(matches!(shortcut_for(0x54, true, false, false), Some(Shortcut::NewTab)));
        assert!(matches!(shortcut_for(0x54, true, true, false), Some(Shortcut::ReopenTab)));
        assert!(matches!(shortcut_for(0x45, true, false, false), Some(Shortcut::ToggleSidebar)));
        assert!(matches!(shortcut_for(0x53, true, false, false), Some(Shortcut::ToggleSidebar)));
        assert!(matches!(shortcut_for(0x33, true, false, false), Some(Shortcut::SpaceIndex(3))));
        assert!(matches!(shortcut_for(0x09, true, true, false), Some(Shortcut::CycleTab(-1))));
        assert!(shortcut_for(0x41, true, false, false).is_none()); // Ctrl+A stays with the page
        assert!(matches!(shortcut_for(0x4F, true, false, false), Some(Shortcut::OpenFile)));
        assert!(shortcut_for(0x54, false, false, false).is_none());
    }
}

/// UI rectangles to cut out of a page view: logical px relative to the page,
/// as (left, top, right, bottom), and the page's logical width.
#[derive(Clone, Default)]
pub struct Clip {
    /// (left, top, right, bottom, corner radius)
    pub holes: Vec<(f64, f64, f64, f64, f64)>,
    pub page_width: f64,
}

/// Cuts the clip's holes out of a page webview's window region. The page keeps
/// its size and position (no reflow); the UI underneath shows through the
/// holes and gets their clicks. No holes removes the region.
pub fn set_clip(wv: &Webview, clip: Clip) {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, SetWindowRgn, HGDIOBJ, RGN_DIFF};
    use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetClientRect};
    let _ = wv.with_webview(move |pw| unsafe {
        let mut host = HWND::default();
        if pw.controller().ParentWindow(&mut host).is_err() || host.0.is_null() {
            return;
        }
        // Only ever clip the page's own container, never the app window.
        let mut class = [0u16; 64];
        let n = GetClassNameW(host, &mut class) as usize;
        if String::from_utf16_lossy(&class[..n]) != "WRY_WEBVIEW" {
            return;
        }
        if clip.holes.is_empty() {
            let _ = SetWindowRgn(host, None, true);
            return;
        }
        let mut rc = RECT::default();
        if GetClientRect(host, &mut rc).is_err() || rc.right <= 0 || clip.page_width <= 0.0 {
            return;
        }
        // Physical px per logical px, from the page window's own size.
        let sf = rc.right as f64 / clip.page_width;
        let full = CreateRectRgn(0, 0, rc.right, rc.bottom);
        for (l, t, r, b, radius) in clip.holes {
            let (l, t, r, b) = ((l * sf).floor() as i32, (t * sf).floor() as i32, (r * sf).ceil() as i32, (b * sf).ceil() as i32);
            if r <= l || b <= t {
                continue;
            }
            let d = (radius * sf * 2.0).round() as i32;
            // Round-rect regions exclude their right and bottom edges.
            let hole = if d > 1 { CreateRoundRectRgn(l, t, r + 1, b + 1, d, d) } else { CreateRectRgn(l, t, r, b) };
            CombineRgn(Some(full), Some(full), Some(hole), RGN_DIFF);
            let _ = DeleteObject(HGDIOBJ(hole.0));
        }
        // The system owns the region from here on.
        let _ = SetWindowRgn(host, Some(full), true);
    });
}
