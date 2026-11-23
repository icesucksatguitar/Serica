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
            0x54 => Some(Shortcut::NewTab),
            0x4C | 0x4B => Some(Shortcut::OpenCommandBar), // L, K
            0x57 | 0x73 => Some(Shortcut::CloseTab),        // W, F4
            0x45 | 0x53 => Some(Shortcut::ToggleSidebar),   // E, S
            0x46 if !shift => Some(Shortcut::Find),         // F
            0x48 => Some(Shortcut::History),                // H
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
pub fn attach(app: &Shared, tab_id: &str, profile: &str, wv: &Webview) {
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
        if let Err(e) = install(app, tab_id, controller, env) {
            tracing::error!("webview2 hook install failed: {e}");
        }
    });
}

unsafe fn install(app: Shared, tab_id: String, controller: ICoreWebView2Controller, env: ICoreWebView2Environment) -> windows::core::Result<()> {
    let core = controller.CoreWebView2()?;
    let ctx = Rc::new(RefCell::new(Ctx::default()));
    let mut token = Default::default();

    if let Ok(settings) = core.Settings() {
        let _ = settings.SetAreDevToolsEnabled(true);
        let _ = settings.SetIsZoomControlEnabled(true);
        let _ = settings.SetIsStatusBarEnabled(true);
        let _ = settings.SetAreDefaultContextMenusEnabled(true);
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
                ctx.borrow_mut().top_url = uri.clone();
                install_cosmetics(&app, &core2, &ctx, &uri);
                app.tab_event(&id, TabEvent::NavigationStarting(uri));
                Ok(())
            })),
            &mut token,
        )?;
    }
    {
        let (app, id, ctx) = (app.clone(), tab_id.clone(), ctx.clone());
        core.add_SourceChanged(
            &SourceChangedEventHandler::create(Box::new(move |sender, _| {
                let Some(sender) = sender else { return Ok(()) };
                let mut uri = PWSTR::null();
                sender.Source(&mut uri)?;
                let uri = take(uri);
                ctx.borrow_mut().top_url = uri.clone();
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
        let ctx = ctx.clone();
        core.add_ContentLoading(
            &ContentLoadingEventHandler::create(Box::new(move |sender, args| {
                let (Some(sender), Some(args)) = (sender, args) else { return Ok(()) };
                let mut error_page = Default::default();
                args.IsErrorPage(&mut error_page)?;
                if !error_page.as_bool() {
                    verify_scriptlets(&sender, &ctx);
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    {
        let (app, id) = (app.clone(), tab_id.clone());
        core.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |sender, args| {
                let (Some(sender), Some(args)) = (sender, args) else { return Ok(()) };
                let mut ok = Default::default();
                args.IsSuccess(&mut ok)?;
                let mut uri = PWSTR::null();
                sender.Source(&mut uri)?;
                let url = take(uri);
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
                app.tab_event(&id, TabEvent::Title(take(t)));
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
    Ok(())
}

/// Registers this navigation's cosmetic CSS and scriptlets to run at document
/// creation, replacing the previous navigation's script.
/// Keep at most this many per-host cosmetic scripts per tab.
const MAX_COSMETIC_SCRIPTS: usize = 48;

unsafe fn install_cosmetics(app: &Shared, core: &ICoreWebView2, ctx: &Rc<RefCell<Ctx>>, url: &str) {
    let res = app.shields.cosmetic(url);
    let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    {
        let mut c = ctx.borrow_mut();
        c.exceptions = res.as_ref().map(|r| r.exceptions.clone()).unwrap_or_default();
        c.generichide = res.as_ref().map(|r| r.generichide).unwrap_or(false);
    }
    let Some(res) = res else { return };
    if host.is_empty() || (res.hide_selectors.is_empty() && res.injected_script.trim().is_empty()) {
        return;
    }
    if ctx.borrow().cosmetic_scripts.contains_key(&host) {
        return; // already registered for this host (or registering)
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
    {
        let mut c = ctx.borrow_mut();
        c.cosmetic_scripts.insert(host.clone(), None);
        if has_scriptlets {
            c.scriptlet_hosts.insert(host.clone());
        }
    }
    let ctx2 = ctx.clone();
    let _ = core.AddScriptToExecuteOnDocumentCreated(
        &HSTRING::from(script),
        &AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(move |hr, id| {
            let mut c = ctx2.borrow_mut();
            if hr.is_ok() {
                c.cosmetic_scripts.insert(host, Some(id));
            } else {
                c.cosmetic_scripts.remove(&host);
                c.scriptlet_hosts.remove(&host);
            }
            Ok(())
        })),
    );
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
        assert!(shortcut_for(0x54, false, false, false).is_none());
    }
}
