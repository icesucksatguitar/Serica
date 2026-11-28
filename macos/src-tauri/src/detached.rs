//! Tabs in windows of their own. A tab dragged out of Serica's window (or
//! moved with "Move to new window") takes its live page into a new window:
//! the page keeps its state (no reload), under a slim toolbar that is the
//! app's UI bundle opened at `index.html?detached=<tab id>`. "Back to
//! Serica" moves the page back into the main window; closing the window
//! closes the tab. The store keeps the tab (see `Store::detached`), so it
//! stays in the sidebar and comes back to the main window after a restart.

use crate::app::Shared;
use crate::tabhost::{Bounds, TabHost};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;
use tauri::{LogicalPosition, LogicalSize, Manager, WebviewBuilder, WebviewUrl, WindowEvent};

/// Height of the toolbar above the page (logical px); the UI draws the same.
pub const TOOLBAR_H: f64 = 40.0;
/// Label prefix of the toolbar webviews (they get the app's IPC, see
/// capabilities/default.json).
pub const UI_PREFIX: &str = "dwin-ui-";

struct Win {
    window: tauri::Window,
    ui: tauri::Webview,
}

static SEQ: AtomicU64 = AtomicU64::new(1);
static WINDOWS: LazyLock<parking_lot::Mutex<HashMap<String, Win>>> = LazyLock::new(Default::default);
/// The tab whose window is the key window, if any (menu shortcuts act on it).
static KEY: LazyLock<parking_lot::Mutex<Option<String>>> = LazyLock::new(Default::default);

/// The tab in the window that has keyboard focus, when that is a tab's own
/// window (not the main window).
pub fn key_tab() -> Option<String> {
    let k = KEY.lock().clone()?;
    WINDOWS.lock().contains_key(&k).then_some(k)
}

/// Labels of the toolbar webviews, for events the UI gets (`ui_emit`).
pub fn ui_labels() -> Vec<String> {
    WINDOWS.lock().values().map(|w| w.ui.label().to_string()).collect()
}

pub fn is_detached(tab_id: &str) -> bool {
    WINDOWS.lock().contains_key(tab_id)
}

/// Brings a tab's window to the front. False when the tab has none.
pub fn focus(tab_id: &str) -> bool {
    let w = WINDOWS.lock().get(tab_id).map(|w| w.window.clone());
    match w {
        Some(w) => {
            let _ = w.unminimize();
            let _ = w.set_focus();
            true
        }
        None => false,
    }
}

fn layout(tab_id: &str, app: &Shared) {
    let (window, ui) = match WINDOWS.lock().get(tab_id) {
        Some(w) => (w.window.clone(), w.ui.clone()),
        None => return,
    };
    let (Ok(size), Ok(sf)) = (window.inner_size(), window.scale_factor()) else { return };
    let (w, h) = (size.width as f64 / sf, size.height as f64 / sf);
    let top = TOOLBAR_H.min(h);
    let _ = ui.set_bounds(tauri::Rect { position: LogicalPosition::new(0.0, 0.0).into(), size: LogicalSize::new(w.max(1.0), TOOLBAR_H).into() });
    if let Some(page) = TabHost::webview(app, tab_id) {
        TabHost::apply_bounds(&page, Bounds { x: 0.0, y: top, w, h: (h - top).max(1.0) });
    }
}

/// Moves `tab_id`'s page into a new window, near `at` (screen position,
/// physical px: where the tab was dropped). Blocking: run off the main thread.
pub fn detach(app: &Shared, tab_id: &str, at: Option<(f64, f64)>) -> Result<(), String> {
    let (title, url, profile, private, zoom) = {
        let s = app.store.lock();
        let t = s.tab(tab_id).ok_or("no such tab")?;
        if crate::model::is_internal_url(&t.url) || s.detached.contains_key(tab_id) {
            return Err("this tab cannot have its own window".into());
        }
        let zoom = t.zoom_factor.filter(|z| z.is_finite() && *z > 0.0).unwrap_or(s.settings.text_scale);
        (t.title.clone(), t.url.clone(), s.space_profile(&t.space_id), t.private, zoom)
    };
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let label = format!("dwin-{n}");
    // The page keeps the size it had in the main window.
    let area = app.host.lock().bounds.map(|b| (b.w, b.h)).unwrap_or((1100.0, 720.0));
    let (w, h) = (area.0.max(480.0), area.1.max(320.0) + TOOLBAR_H);
    let mut b = tauri::window::WindowBuilder::new(&app.handle, &label)
        .title(if title.is_empty() { "Serica" } else { title.as_str() })
        .inner_size(w, h)
        .min_inner_size(360.0, 240.0)
        // As the main window: the traffic lights sit in Serica's toolbar, at
        // AppKit's place (centred 16pt from the top; see detached.css)
        // (the content view runs under the title bar, which would otherwise
        // cover the toolbar).
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .visible(true);
    if let Some(icon) = app.handle.default_window_icon() {
        b = b.icon(icon.clone()).map_err(|e| e.to_string())?;
    }
    // The pointer ends up over the toolbar, as if the tab were held there
    // (`at` is in physical screen px).
    let sf = app.handle.get_window("main").and_then(|w| w.scale_factor().ok()).unwrap_or(1.0);
    b = match at {
        Some((x, y)) => b.position((x / sf - 160.0).max(0.0), (y / sf - 18.0).max(0.0)),
        None => b.center(),
    };
    let window = b.build().map_err(|e| e.to_string())?;
    let ui_url = WebviewUrl::App(format!("index.html?detached={}", urlencode(tab_id)).into());
    let ui = match window.add_child(
        WebviewBuilder::new(format!("{UI_PREFIX}{n}"), ui_url).background_color(tauri::webview::Color(24, 24, 27, 255)),
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(w, TOOLBAR_H),
    ) {
        Ok(ui) => ui,
        Err(e) => {
            let _ = window.destroy();
            return Err(e.to_string());
        }
    };
    let page_bounds = Bounds { x: 0.0, y: TOOLBAR_H, w, h: h - TOOLBAR_H };

    // The page: the tab's own webview moves over; a tab without one (asleep)
    // gets a new one in this window.
    let existing = {
        // Its UI cut-outs go with the next clip pass: it is no longer one of
        // the main window's pages on screen (see `TabHost::sync_clip_pages`).
        let mut host = app.host.lock();
        host.views.get_mut(tab_id).map(|v| {
            v.detached = true;
            v.shown = false;
            v.hidden_at = None;
            v.webview.clone()
        })
    };
    match existing {
        Some(page) => {
            if let Err(e) = page.reparent(&window) {
                tracing::error!("could not move {tab_id} to its own window: {e}");
                if let Some(v) = app.host.lock().views.get_mut(tab_id) {
                    v.detached = false;
                }
                let _ = window.destroy();
                TabHost::reconcile(app);
                return Err(e.to_string());
            }
            TabHost::apply_bounds(&page, page_bounds);
            let _ = page.show();
            let _ = page.set_focus();
        }
        None => {
            let pending = {
                let mut host = app.host.lock();
                let p = host.prepare_in(tab_id, &url, window.clone(), page_bounds);
                if p.is_some() {
                    host.creating.insert(tab_id.to_string());
                }
                p
            };
            let page = pending.and_then(|p| TabHost::build(app, tab_id, &profile, private, p));
            let mut host = app.host.lock();
            host.creating.remove(tab_id);
            let Some(page) = page else {
                drop(host);
                let _ = window.destroy();
                return Err("could not open the page".into());
            };
            if (zoom - 1.0).abs() > 0.001 {
                crate::webview_mac::set_zoom(&page, zoom);
            }
            crate::clip_mac::register(&page);
            if let Some(old) = host.views.insert(
                tab_id.to_string(),
                crate::tabhost::TabView { webview: page.clone(), profile, shown: false, hidden_at: None, extension_page: crate::tabhost::is_extension_page(&url), detached: true },
            ) {
                let _ = old.webview.close();
            }
            drop(host);
            let _ = page.set_focus();
        }
    }

    WINDOWS.lock().insert(tab_id.to_string(), Win { window: window.clone(), ui });
    {
        let (app2, id) = (app.clone(), tab_id.to_string());
        window.on_window_event(move |e| match e {
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => layout(&id, &app2),
            WindowEvent::Focused(true) => *KEY.lock() = Some(id.clone()),
            WindowEvent::Focused(false) => {
                let mut k = KEY.lock();
                if k.as_deref() == Some(id.as_str()) {
                    *k = None;
                }
            }
            // Closing the window closes its tab (the page goes with it).
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let (app3, id2) = (app2.clone(), id.clone());
                tauri::async_runtime::spawn_blocking(move || {
                    app3.mutate(|s| s.close_tab(&id2));
                });
            }
            _ => {}
        });
    }
    app.mutate(|s| s.detach(tab_id, &label));
    layout(tab_id, app);
    Ok(())
}

/// Moves a tab's page back into the main window and shows it there.
pub fn reattach(app: &Shared, tab_id: &str) -> Result<(), String> {
    let win = WINDOWS.lock().remove(tab_id).ok_or("not in its own window")?;
    let main = app.handle.get_window("main").ok_or("no main window")?;
    let page = {
        let mut host = app.host.lock();
        host.views.get_mut(tab_id).map(|v| {
            v.detached = false;
            v.shown = false;
            v.hidden_at = Some(std::time::Instant::now());
            v.webview.clone()
        })
    };
    if let Some(page) = page {
        let _ = page.hide();
        if let Err(e) = page.reparent(&main) {
            tracing::error!("could not move {tab_id} back: {e}");
        }
    }
    let _ = win.window.destroy();
    app.mutate(|s| s.reattach(tab_id));
    if let Some(w) = app.handle.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    Ok(())
}

/// The pages of these tabs were closed (tab closed, put to sleep, or gone to
/// one of Serica's own pages): their windows go too, and a tab that still
/// exists is back in the main window.
pub fn page_gone(app: &Shared, ids: Vec<String>) {
    let mut wins = Vec::new();
    {
        let mut map = WINDOWS.lock();
        for id in &ids {
            if let Some(w) = map.remove(id) {
                wins.push(w.window);
            }
        }
    }
    for w in wins {
        let _ = w.destroy();
    }
    let back = app.store.lock().detached.keys().any(|k| ids.contains(k));
    if back {
        app.mutate(|s| {
            for id in &ids {
                s.detached.remove(id);
            }
        });
    }
}

/// HTML fullscreen in a tab's own window. WebKit moves the element into a
/// fullscreen window of its own, so neither this window nor the main one
/// changes; once it is back the page is laid out under the toolbar again.
/// False when the tab is not in a window of its own.
pub fn set_fullscreen(app: &Shared, tab_id: &str, fs: bool) -> bool {
    if !is_detached(tab_id) {
        return false;
    }
    if !fs {
        layout(tab_id, app);
    }
    true
}

/// The window title follows the page.
pub fn sync_titles(app: &Shared) {
    let wins: Vec<(String, tauri::Window)> = WINDOWS.lock().iter().map(|(k, w)| (k.clone(), w.window.clone())).collect();
    if wins.is_empty() {
        return;
    }
    let titles: Vec<(tauri::Window, String)> = {
        let s = app.store.lock();
        wins.into_iter().filter_map(|(id, w)| s.tab(&id).map(|t| (w, if t.title.is_empty() { t.url.clone() } else { t.title.clone() }))).collect()
    };
    for (w, t) in titles {
        if w.title().ok().as_deref() != Some(t.as_str()) {
            let _ = w.set_title(&t);
        }
    }
}

/// Serica is quitting (or its main window closed): every tab window goes.
pub fn close_all(_app: &Shared) {
    let wins: Vec<Win> = WINDOWS.lock().drain().map(|(_, w)| w).collect();
    for w in wins {
        let _ = w.window.destroy();
    }
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}
