//! Runtime verification hooks (Cargo feature `automation`, never in a
//! release build). With `SERICA_AUTOMATION_PORT` set, a test copy listens on
//! 127.0.0.1:<port> for JSON commands (POST, one per request) that drive
//! the app the way a person would, in-process, so no Accessibility or Screen
//! Recording permission is needed:
//!
//! - `{"op":"eval-ui","js":"…"}` / `{"op":"eval-tab","tabId":"…","js":"…"}`:
//!   runs an async function body in the UI or a page; returns its value.
//! - `{"op":"key","key":"c","mods":["cmd"]}`: a key press through
//!   `NSApp.sendEvent` (key equivalents, the menu bar, the responder chain).
//! - `{"op":"mouse","x":…,"y":…,"kind":"click|right|move|down|up","clicks":1}`:
//!   a mouse event at window points (top-left origin).
//! - `{"op":"focus","target":"ui|tab"}`: makes the UI or the active page
//!   the first responder.
//! - `{"op":"window"}`: window frame, scale, full screen, traffic lights and
//!   the content view's subviews (frames in top-left points, masks).
//! - `{"op":"snapshot","path":"/…/x.png"}`: the window as composited: the UI,
//!   each page clipped by its real mask, and the traffic lights.
//! - `{"op":"pasteboard"}` / `{"op":"pasteboard","set":"…"}`.

use crate::app::Shared;
use crate::mac_util::{ns, on_main, to_json};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker, Message};
use objc2_app_kit::{
    NSApplication, NSBitmapImageFileType, NSBitmapImageRep, NSCompositingOperation, NSEvent, NSEventModifierFlags, NSEventType, NSGraphicsContext, NSImage, NSPasteboard,
    NSPasteboardTypeString, NSView, NSWindow, NSWindowButton,
};
use objc2_core_foundation::{CGAffineTransform, CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGContext;
use objc2_foundation::{NSDictionary, NSError, NSPoint, NSRect};
use objc2_quartz_core::CAShapeLayer;
use objc2_web_kit::{WKContentWorld, WKWebView};
use serde_json::{json, Value};
use std::sync::mpsc;
use std::time::Duration;
use tauri::Manager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Starts the endpoint when `SERICA_AUTOMATION_PORT` is set.
pub fn start(app: &Shared) {
    let Some(port) = std::env::var("SERICA_AUTOMATION_PORT").ok().and_then(|p| p.parse::<u16>().ok()) else { return };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("automation: could not listen on {port}: {e}");
                return;
            }
        };
        tracing::warn!("automation: listening on 127.0.0.1:{port} (test builds only)");
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { continue };
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let body = match read_request(&mut sock).await {
                    Some(b) => b,
                    None => return,
                };
                let cmd: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let app2 = app.clone();
                let out = tauri::async_runtime::spawn_blocking(move || run(&app2, &cmd)).await.unwrap_or_else(|e| json!({ "error": e.to_string() }));
                let text = out.to_string();
                let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len());
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
}

async fn read_request(sock: &mut tokio::net::TcpStream) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = sock.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).to_lowercase();
            let len = head.lines().find_map(|l| l.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
            while buf.len() < end + 4 + len {
                let n = sock.read(&mut chunk).await.ok()?;
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            return Some(buf[end + 4..(end + 4 + len).min(buf.len())].to_vec());
        }
        if buf.len() > 8 * 1024 * 1024 {
            return None;
        }
    }
}

const WAIT: Duration = Duration::from_secs(20);

fn run(app: &Shared, cmd: &Value) -> Value {
    let op = cmd["op"].as_str().unwrap_or("").to_string();
    match op.as_str() {
        "eval-ui" => eval(app, None, cmd["js"].as_str().unwrap_or("")),
        "eval-tab" => {
            let tab = cmd["tabId"].as_str().map(str::to_string).or_else(|| app.store.lock().active_tab_id.clone());
            match tab {
                Some(t) => eval(app, Some(t), cmd["js"].as_str().unwrap_or("")),
                None => json!({ "error": "no tab" }),
            }
        }
        "snapshot" => snapshot(app, cmd["path"].as_str().unwrap_or("/tmp/serica-snapshot.png").to_string()),
        "extensions" => on_main(&app.handle, WAIT, crate::extensions::webkit::diagnostics).unwrap_or_else(|| json!({ "error": "main thread timeout" })),
        "unocclude" => {
            // The screen may be locked (no display, the window occluded):
            // WebKit would then hold back frames. Test runs only.
            let handle = app.handle.clone();
            on_main(&app.handle, WAIT, move || {
                let Some(window) = main_window(&handle) else { return json!({ "error": "no window" }) };
                let Some(content) = window.contentView() else { return json!({ "error": "no content view" }) };
                let mut n = 0;
                for v in content.subviews().iter() {
                    let sel = objc2::sel!(_setWindowOcclusionDetectionEnabled:);
                    if crate::mac_util::responds(&*v, sel) {
                        let _: () = unsafe { objc2::msg_send![&*v, _setWindowOcclusionDetectionEnabled: false] };
                        n += 1;
                    }
                }
                json!({ "ok": true, "views": n })
            })
            .unwrap_or_else(|| json!({ "error": "main thread timeout" }))
        }
        "key" | "keyequiv" | "menu" | "menu-action" | "mouse" | "focus" | "window" | "pasteboard" => {
            let cmd = cmd.clone();
            let handle = app.handle.clone();
            on_main(&app.handle, WAIT, move || {
                let mtm = MainThreadMarker::new().expect("main thread");
                let Some(window) = main_window(&handle) else { return json!({ "error": "no window" }) };
                match cmd["op"].as_str().unwrap_or("") {
                    "key" => key(mtm, &window, &cmd),
                    "keyequiv" => key_equivalent(mtm, &window, &cmd),
                    "menu" => menu_list(mtm),
                    "menu-action" => menu_action(mtm, &handle, &cmd),
                    "mouse" => mouse(mtm, &window, &cmd),
                    "focus" => focus(&handle, &window, cmd["target"].as_str().unwrap_or("ui")),
                    "window" => window_info(&window),
                    _ => pasteboard(&cmd),
                }
            })
            .unwrap_or_else(|| json!({ "error": "main thread timeout" }))
        }
        _ => json!({ "error": format!("unknown op {op:?}") }),
    }
}

fn main_window(handle: &tauri::AppHandle) -> Option<Retained<NSWindow>> {
    let w = handle.get_window("main")?;
    let ptr = w.ns_window().ok()? as *const NSWindow;
    // SAFETY: the live NSWindow, used on the main thread.
    (!ptr.is_null()).then(|| unsafe { (*ptr).retain() })
}

fn main_ui_view(handle: &tauri::AppHandle) -> Option<Retained<WKWebView>> {
    let wv = handle.get_webview("main")?;
    let (tx, rx) = mpsc::sync_channel(1);
    let _ = wv.with_webview(move |pw| {
        let ptr = pw.inner() as *const WKWebView;
        // SAFETY: wry's live WKWebView, on the main thread.
        let _ = tx.send((!ptr.is_null()).then(|| crate::mac_util::MainBox(unsafe { (*ptr).retain() })));
    });
    // with_webview runs right away on the main thread.
    rx.try_recv().ok().flatten().map(|b| b.0)
}

// ------------------------------------------------------------------ scripts

fn eval(app: &Shared, tab: Option<String>, js: &str) -> Value {
    let (tx, rx) = mpsc::sync_channel::<Value>(1);
    let js = js.to_string();
    let handle = app.handle.clone();
    let started = on_main(&app.handle, WAIT, move || {
        let mtm = MainThreadMarker::new().expect("main thread");
        let view = match &tab {
            Some(t) => crate::webview_mac::api::tab_view(t),
            None => main_ui_view(&handle),
        };
        let Some(view) = view else { return false };
        let done = RcBlock::new(move |result: *mut AnyObject, err: *mut NSError| {
            let v = match unsafe { err.as_ref() } {
                Some(e) => json!({ "error": e.localizedDescription().to_string(), "detail": format!("{:?}", e.userInfo()) }),
                None => json!({ "value": to_json(unsafe { result.as_ref() }) }),
            };
            let _ = tx.send(v);
        });
        unsafe {
            view.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(&ns(&js), None, None, &WKContentWorld::pageWorld(mtm), Some(&done));
        }
        true
    })
    .unwrap_or(false);
    if !started {
        return json!({ "error": "no such view" });
    }
    rx.recv_timeout(WAIT).unwrap_or_else(|_| json!({ "error": "timeout" }))
}

// ------------------------------------------------------------------ input

fn key_code(key: &str) -> u16 {
    const LETTERS: [u16; 26] = [0x00, 0x0B, 0x08, 0x02, 0x0E, 0x03, 0x05, 0x04, 0x22, 0x26, 0x28, 0x25, 0x2E, 0x2D, 0x1F, 0x23, 0x0C, 0x0F, 0x01, 0x11, 0x20, 0x09, 0x0D, 0x07, 0x10, 0x06];
    const DIGITS: [u16; 10] = [0x1D, 0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19];
    match key {
        "tab" => 0x30,
        "return" | "enter" => 0x24,
        "escape" => 0x35,
        "delete" | "backspace" => 0x33,
        "left" => 0x7B,
        "right" => 0x7C,
        "down" => 0x7D,
        "up" => 0x7E,
        "space" => 0x31,
        "," => 0x2B,
        "." => 0x2F,
        "[" => 0x21,
        "]" => 0x1E,
        "-" => 0x1B,
        "=" => 0x18,
        "f12" => 0x6F,
        k if k.len() == 1 => {
            let c = k.as_bytes()[0].to_ascii_lowercase();
            match c {
                b'a'..=b'z' => LETTERS[(c - b'a') as usize],
                b'0'..=b'9' => DIGITS[(c - b'0') as usize],
                _ => 0,
            }
        }
        _ => 0,
    }
}

fn key_chars(key: &str) -> String {
    match key {
        "tab" => "\t".into(),
        "return" | "enter" => "\r".into(),
        "escape" => "\u{1b}".into(),
        "delete" | "backspace" => "\u{7f}".into(),
        "left" => "\u{F702}".into(),
        "right" => "\u{F703}".into(),
        "down" => "\u{F701}".into(),
        "up" => "\u{F700}".into(),
        "space" => " ".into(),
        "f12" => "\u{F70F}".into(),
        k => k.to_string(),
    }
}

fn mods(cmd: &Value) -> NSEventModifierFlags {
    let mut f = NSEventModifierFlags(0);
    for m in cmd["mods"].as_array().into_iter().flatten().filter_map(|m| m.as_str()) {
        f.0 |= match m {
            "cmd" => NSEventModifierFlags::Command.0,
            "shift" => NSEventModifierFlags::Shift.0,
            "opt" | "alt" => NSEventModifierFlags::Option.0,
            "ctrl" => NSEventModifierFlags::Control.0,
            _ => 0,
        };
    }
    f
}

fn key(mtm: MainThreadMarker, window: &NSWindow, cmd: &Value) -> Value {
    let k = cmd["key"].as_str().unwrap_or("").to_string();
    let flags = mods(cmd);
    let chars = key_chars(&k);
    let shown = if flags.0 & NSEventModifierFlags::Shift.0 != 0 && chars.len() == 1 { chars.to_uppercase() } else { chars.clone() };
    let app = NSApplication::sharedApplication(mtm);
    let ts = NSProcessInfoUptime::now();
    for (ty, ts) in [(NSEventType::KeyDown, ts), (NSEventType::KeyUp, ts + 0.02)] {
        let ev = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            ty,
            NSPoint::new(0.0, 0.0),
            flags,
            ts,
            window.windowNumber(),
            None,
            &ns(&shown),
            &ns(&chars),
            false,
            key_code(&k),
        );
        match ev {
            Some(ev) => app.sendEvent(&ev),
            None => return json!({ "error": "could not make the key event" }),
        }
    }
    json!({ "ok": true })
}

/// A key press as AppKit handles a key equivalent once it has found the key
/// window: the window's views first (`performKeyEquivalent:`), then the menu
/// bar. Works while the screen is locked (no key window).
fn key_equivalent(mtm: MainThreadMarker, window: &NSWindow, cmd: &Value) -> Value {
    let k = cmd["key"].as_str().unwrap_or("").to_string();
    let chars = key_chars(&k);
    let flags = mods(cmd);
    let Some(ev) = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown,
        NSPoint::new(0.0, 0.0),
        flags,
        NSProcessInfoUptime::now(),
        window.windowNumber(),
        None,
        &ns(&chars),
        &ns(&chars),
        false,
        key_code(&k),
    ) else {
        return json!({ "error": "could not make the key event" });
    };
    if window.performKeyEquivalent(&ev) {
        return json!({ "handledBy": "window" });
    }
    let menu = NSApplication::sharedApplication(mtm).mainMenu();
    if menu.is_some_and(|m| m.performKeyEquivalent(&ev)) {
        return json!({ "handledBy": "menu" });
    }
    json!({ "handledBy": null })
}

fn menu_list(mtm: MainThreadMarker) -> Value {
    fn walk(menu: &objc2_app_kit::NSMenu) -> Vec<Value> {
        menu.itemArray()
            .iter()
            .filter(|i| !i.isSeparatorItem())
            .map(|i| {
                let m = i.keyEquivalentModifierMask();
                let mut glyph = String::new();
                for (bit, g) in [(NSEventModifierFlags::Control, "⌃"), (NSEventModifierFlags::Option, "⌥"), (NSEventModifierFlags::Shift, "⇧"), (NSEventModifierFlags::Command, "⌘")] {
                    if m.0 & bit.0 != 0 && !i.keyEquivalent().is_empty() {
                        glyph.push_str(g);
                    }
                }
                glyph.push_str(&i.keyEquivalent().to_string());
                json!({
                    "title": i.title().to_string(),
                    "key": glyph,
                    "action": i.action().map(|a| a.name().to_string_lossy().to_string()),
                    "hidden": i.isHidden(),
                    "items": i.submenu().map(|s| walk(&s)).unwrap_or_default(),
                })
            })
            .collect()
    }
    match NSApplication::sharedApplication(mtm).mainMenu() {
        Some(m) => json!({ "menu": walk(&m) }),
        None => json!({ "error": "no menu bar" }),
    }
}

/// Performs a menu item's action (`path`: titles), sent to the UI or the
/// active page as its first responder would receive it.
fn menu_action(mtm: MainThreadMarker, handle: &tauri::AppHandle, cmd: &Value) -> Value {
    let path: Vec<String> = cmd["path"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
    let Some(mut menu) = NSApplication::sharedApplication(mtm).mainMenu() else { return json!({ "error": "no menu bar" }) };
    let mut item = None;
    for (n, title) in path.iter().enumerate() {
        let Some(found) = menu.itemArray().iter().find(|i| i.title().to_string() == *title) else { return json!({ "error": format!("no item {title:?}") }) };
        if n + 1 < path.len() {
            let Some(sub) = found.submenu() else { return json!({ "error": format!("{title:?} has no submenu") }) };
            menu = sub;
        } else {
            item = Some(found);
        }
    }
    let Some(item) = item else { return json!({ "error": "empty path" }) };
    let Some(action) = item.action() else { return json!({ "error": "the item has no action" }) };
    let target: Option<Retained<AnyObject>> = match cmd["target"].as_str() {
        Some("tab") => crate::app::global()
            .and_then(|a| a.store.lock().active_tab_id.clone())
            .and_then(|t| crate::webview_mac::api::tab_view(&t))
            .map(|w| Retained::into_super(Retained::into_super(Retained::into_super(Retained::into_super(w))))),
        Some("ui") => main_ui_view(handle).map(|w| Retained::into_super(Retained::into_super(Retained::into_super(Retained::into_super(w))))),
        _ => item.target(),
    };
    let sent = unsafe { NSApplication::sharedApplication(mtm).sendAction_to_from(action, target.as_deref(), Some(&item)) };
    json!({ "sent": sent, "action": action.name().to_string_lossy() })
}

/// Seconds since boot, the clock NSEvent timestamps use.
struct NSProcessInfoUptime;
impl NSProcessInfoUptime {
    fn now() -> f64 {
        objc2_foundation::NSProcessInfo::processInfo().systemUptime()
    }
}

fn mouse(mtm: MainThreadMarker, window: &NSWindow, cmd: &Value) -> Value {
    let Some(content) = window.contentView() else { return json!({ "error": "no content view" }) };
    let h = content.frame().size.height;
    let (x, y) = (cmd["x"].as_f64().unwrap_or(0.0), cmd["y"].as_f64().unwrap_or(0.0));
    let at = NSPoint::new(x, h - y);
    let clicks = cmd["clicks"].as_i64().unwrap_or(1) as isize;
    let kinds: Vec<NSEventType> = match cmd["kind"].as_str().unwrap_or("click") {
        "right" => vec![NSEventType::RightMouseDown, NSEventType::RightMouseUp],
        "move" => vec![NSEventType::MouseMoved],
        "down" => vec![NSEventType::LeftMouseDown],
        "up" => vec![NSEventType::LeftMouseUp],
        "drag" => vec![NSEventType::LeftMouseDragged],
        _ => vec![NSEventType::LeftMouseDown, NSEventType::LeftMouseUp],
    };
    let app = NSApplication::sharedApplication(mtm);
    let mut ts = NSProcessInfoUptime::now();
    for ty in kinds {
        let ev = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            ty,
            at,
            mods(cmd),
            ts,
            window.windowNumber(),
            None,
            0,
            clicks,
            1.0,
        );
        match ev {
            Some(ev) => app.sendEvent(&ev),
            None => return json!({ "error": "could not make the mouse event" }),
        }
        ts += 0.03;
    }
    json!({ "ok": true })
}

fn focus(handle: &tauri::AppHandle, window: &NSWindow, target: &str) -> Value {
    let view: Option<Retained<NSView>> = if target == "tab" {
        let tab = crate::app::global().and_then(|a| a.store.lock().active_tab_id.clone());
        tab.and_then(|t| crate::webview_mac::api::tab_view(&t)).map(|w| Retained::into_super(w))
    } else {
        main_ui_view(handle).map(|w| Retained::into_super(w))
    };
    let Some(view) = view else { return json!({ "error": "no view" }) };
    // Key events go to the key window of the active app.
    let mtm = MainThreadMarker::new().expect("main thread");
    NSApplication::sharedApplication(mtm).activate();
    window.makeKeyAndOrderFront(None);
    json!({ "ok": window.makeFirstResponder(Some(&view)) })
}

// ------------------------------------------------------------------ geometry

/// A rect in window coordinates (bottom-left) as top-left points.
fn top_left(r: NSRect, h: f64) -> Value {
    json!({ "x": r.origin.x, "y": h - r.origin.y - r.size.height, "w": r.size.width, "h": r.size.height })
}

fn window_info(window: &NSWindow) -> Value {
    let Some(content) = window.contentView() else { return json!({ "error": "no content view" }) };
    let h = content.frame().size.height;
    let lights: Vec<Value> = [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton]
        .into_iter()
        .filter_map(|b| window.standardWindowButton(b))
        .map(|b| {
            let r = b.convertRect_toView(b.bounds(), None);
            json!({ "frame": top_left(r, h), "hidden": b.isHiddenOrHasHiddenAncestor() })
        })
        .collect();
    let subviews: Vec<Value> = content
        .subviews()
        .iter()
        .map(|v| {
            let r = v.convertRect_toView(v.bounds(), None);
            let masked = v.layer().and_then(|l| l.mask()).is_some();
            let (is_web, is_view): (bool, bool) = unsafe {
                (objc2::msg_send![&*v, isKindOfClass: <WKWebView as objc2::ClassType>::class()], objc2::msg_send![&*v, isKindOfClass: <NSView as objc2::ClassType>::class()])
            };
            let reported: &objc2::runtime::AnyClass = unsafe { objc2::msg_send![&*v, class] };
            json!({ "class": v.class().name().to_string_lossy(), "reportedClass": reported.name().to_string_lossy(), "isWKWebView": is_web, "isNSView": is_view,
                    "superclasses": std::iter::successors(Some(v.class()), |c| c.superclass()).map(|c| c.name().to_string_lossy().to_string()).collect::<Vec<_>>(),
                    "frame": top_left(r, h), "hidden": v.isHidden(), "masked": masked, "flipped": v.isFlipped() })
        })
        .collect();
    let f = window.frame();
    json!({
        "frame": { "x": f.origin.x, "y": f.origin.y, "w": f.size.width, "h": f.size.height },
        "content": { "w": content.frame().size.width, "h": h },
        "scale": window.backingScaleFactor(),
        "fullscreen": window.styleMask().contains(objc2_app_kit::NSWindowStyleMask::FullScreen),
        "key": window.isKeyWindow(),
        "firstResponder": window.firstResponder().map(|r| r.class().name().to_string_lossy().to_string()),
        "trafficLights": lights,
        "subviews": subviews,
    })
}

fn pasteboard(cmd: &Value) -> Value {
    let pb = NSPasteboard::generalPasteboard();
    unsafe {
        if let Some(s) = cmd["set"].as_str() {
            pb.clearContents();
            pb.setString_forType(&ns(s), NSPasteboardTypeString);
        }
        json!({ "string": pb.stringForType(NSPasteboardTypeString).map(|s| s.to_string()) })
    }
}

// ------------------------------------------------------------------ snapshot

struct Layer {
    /// Where the view is in the window (bottom-left points).
    rect: NSRect,
    /// The page's mask path and its layer → window transform.
    mask: Option<(Retained<objc2_core_graphics::CGPath>, CGAffineTransform, bool)>,
    image: Option<Retained<NSImage>>,
}

fn snapshot(app: &Shared, path: String) -> Value {
    let (tx, rx) = mpsc::sync_channel::<Value>(1);
    let handle = app.handle.clone();
    let started = on_main(&app.handle, WAIT, move || {
        let mtm = MainThreadMarker::new().expect("main thread");
        let Some(window) = main_window(&handle) else { return false };
        let Some(content) = window.contentView() else { return false };
        // The UI first, then the pages above it, in their order on screen.
        let mut views: Vec<Retained<WKWebView>> = Vec::new();
        for v in content.subviews().iter() {
            if v.isHidden() {
                continue;
            }
            // (`downcast` refuses KVO-swizzled classes, which these views are.)
            let is_web: bool = unsafe { objc2::msg_send![&*v, isKindOfClass: <WKWebView as objc2::ClassType>::class()] };
            if is_web {
                // SAFETY: checked just above.
                views.push(unsafe { Retained::cast_unchecked::<WKWebView>(v.clone()) });
            }
        }
        shoot(mtm, window, views, Vec::new(), path, tx);
        true
    })
    .unwrap_or(false);
    if !started {
        return json!({ "error": "no window" });
    }
    rx.recv_timeout(WAIT).unwrap_or_else(|_| json!({ "error": "timeout" }))
}

fn layer_info(view: &NSView) -> Layer {
    let rect = view.convertRect_toView(view.bounds(), None);
    let mask = view.layer().and_then(|l| l.mask()).and_then(|m| m.downcast::<CAShapeLayer>().ok()).and_then(|shape| {
        let path = shape.path()?;
        // Layer → window, from three points.
        let to_window = |p: CGPoint| view.convertPoint_toView(view.convertPointFromLayer(p), None);
        let o = to_window(CGPoint::new(0.0, 0.0));
        let x = to_window(CGPoint::new(1.0, 0.0));
        let y = to_window(CGPoint::new(0.0, 1.0));
        let t = CGAffineTransform { a: x.x - o.x, b: x.y - o.y, c: y.x - o.x, d: y.y - o.y, tx: o.x, ty: o.y };
        let even_odd = shape.fillRule().to_string() == "even-odd";
        Some((path, t, even_odd))
    });
    Layer { rect, mask, image: None }
}

type Shot = (Retained<NSWindow>, Vec<Retained<WKWebView>>, Vec<Layer>, String, mpsc::SyncSender<Value>);

/// Snapshots the views one after another (each completion starts the next),
/// then composes them.
fn shoot(mtm: MainThreadMarker, window: Retained<NSWindow>, mut todo: Vec<Retained<WKWebView>>, done: Vec<Layer>, path: String, tx: mpsc::SyncSender<Value>) {
    if todo.is_empty() {
        let _ = tx.send(compose(&window, done, &path));
        return;
    }
    let view = todo.remove(0);
    let layer = layer_info(&view);
    let state: std::cell::RefCell<Option<(Shot, Layer)>> = std::cell::RefCell::new(Some(((window, todo, done, path, tx), layer)));
    let block = RcBlock::new(move |image: *mut NSImage, _err: *mut NSError| {
        let Some(((window, todo, mut done, path, tx), mut layer)) = state.borrow_mut().take() else { return };
        layer.image = unsafe { image.as_ref() }.map(|i| i.retain());
        done.push(layer);
        shoot(mtm, window, todo, done, path, tx);
    });
    unsafe { view.takeSnapshotWithConfiguration_completionHandler(None, &block) };
}

fn compose(window: &NSWindow, layers: Vec<Layer>, path: &str) -> Value {
    let Some(content) = window.contentView() else { return json!({ "error": "no content view" }) };
    let size = content.frame().size;
    let scale = window.backingScaleFactor();
    let (pw, ph) = ((size.width * scale).round() as isize, (size.height * scale).round() as isize);
    unsafe {
        let Some(rep) = NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            pw,
            ph,
            8,
            4,
            true,
            false,
            objc2_app_kit::NSDeviceRGBColorSpace,
            0,
            0,
        ) else {
            return json!({ "error": "no bitmap" });
        };
        rep.setSize(size);
        let Some(gc) = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep) else { return json!({ "error": "no context" }) };
        NSGraphicsContext::saveGraphicsState_class();
        NSGraphicsContext::setCurrentContext(Some(&gc));
        let cg: Retained<CGContext> = gc.CGContext();
        let mut drawn = Vec::new();
        for l in &layers {
            CGContext::save_g_state(Some(&cg));
            if let Some((p, t, even_odd)) = &l.mask {
                // The mask's path, in the page's layer coordinates, mapped to
                // the window; the clip stays when the transform is undone.
                CGContext::concat_ctm(Some(&cg), *t);
                CGContext::add_path(Some(&cg), Some(p));
                if *even_odd {
                    CGContext::eo_clip(Some(&cg));
                } else {
                    CGContext::clip(Some(&cg));
                }
                CGContext::concat_ctm(Some(&cg), invert(*t));
            }
            if let Some(img) = &l.image {
                img.drawInRect_fromRect_operation_fraction(l.rect, NSRect::ZERO, NSCompositingOperation::SourceOver, 1.0);
            }
            CGContext::restore_g_state(Some(&cg));
            drawn.push(json!({ "frame": top_left(l.rect, size.height), "masked": l.mask.is_some(), "image": l.image.is_some() }));
        }
        // The traffic lights, where macOS draws them.
        let colors = [(1.0, 0.37, 0.34), (1.0, 0.74, 0.18), (0.16, 0.79, 0.25)];
        for (b, (r, g, bl)) in [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton].into_iter().zip(colors) {
            if let Some(btn) = window.standardWindowButton(b) {
                if btn.isHiddenOrHasHiddenAncestor() {
                    continue;
                }
                let fr = btn.convertRect_toView(btn.bounds(), None);
                let d = fr.size.width.min(fr.size.height) - 2.0;
                let c = CGRect::new(CGPoint::new(fr.origin.x + (fr.size.width - d) / 2.0, fr.origin.y + (fr.size.height - d) / 2.0), CGSize::new(d, d));
                CGContext::set_rgb_fill_color(Some(&cg), r, g, bl, 1.0);
                CGContext::fill_ellipse_in_rect(Some(&cg), c);
            }
        }
        NSGraphicsContext::restoreGraphicsState_class();
        let Some(png) = rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) else {
            return json!({ "error": "no png" });
        };
        let ok = png.writeToFile_atomically(&ns(path), true);
        json!({ "ok": ok, "path": path, "size": { "w": size.width, "h": size.height }, "scale": scale, "layers": drawn })
    }
}

fn invert(t: CGAffineTransform) -> CGAffineTransform {
    let det = t.a * t.d - t.b * t.c;
    if det.abs() < 1e-12 {
        return CGAffineTransform { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };
    }
    let (a, b, c, d) = (t.d / det, -t.b / det, -t.c / det, t.a / det);
    CGAffineTransform { a, b, c, d, tx: -(a * t.tx + c * t.ty), ty: -(b * t.tx + d * t.ty) }
}
