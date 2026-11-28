//! Remembers the main window's size, position and zoomed (maximized) state.
//!
//! Bounds are points (Cocoa's logical pixels, top-left origin of the main
//! display) in `window.json`, so they mean the same on Retina and regular
//! displays. The normal (un-zoomed) bounds are kept while zoomed so
//! un-zooming after a restart returns to the right size. Saved on
//! move/resize (throttled) and on close, so a crash loses at most the last
//! half second of dragging. A position on a display that is no longer
//! connected is not restored (the window is centered instead).

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{LogicalPosition, LogicalSize, Runtime, WebviewWindow, WindowEvent};

const MIN_WIDTH: u32 = 400;
const MIN_HEIGHT: u32 = 300;
const SAVE_EVERY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

fn load(path: &Path) -> Option<WindowGeometry> {
    crate::store::read_json(path).and_then(|v| serde_json::from_value(v).ok())
}

fn save(path: &Path, g: &WindowGeometry) {
    if let Ok(bytes) = serde_json::to_vec(g) {
        let _ = crate::store::write_atomic(path, &bytes);
    }
}

/// True if at least a 100x60 strip of the window's title area is on a monitor.
pub fn visible_on(g: &WindowGeometry, monitors: &[(i32, i32, u32, u32)]) -> bool {
    monitors.iter().any(|&(mx, my, mw, mh)| {
        let left = g.x.max(mx);
        let right = (g.x + g.width as i32).min(mx + mw as i32);
        let top = g.y.max(my);
        let bottom = (g.y + 60).min(my + mh as i32);
        right - left >= 100 && bottom - top >= 30
    })
}

/// Restores the saved geometry (if still on screen) and starts tracking changes.
pub fn attach<R: Runtime>(window: &WebviewWindow<R>, file: PathBuf) {
    let saved = load(&file).filter(|g| g.width >= MIN_WIDTH && g.height >= MIN_HEIGHT);
    if let Some(g) = saved {
        let monitors: Vec<(i32, i32, u32, u32)> = window
            .available_monitors()
            .unwrap_or_default()
            .iter()
            .map(|m| {
                let sf = m.scale_factor();
                let p = m.position().to_logical::<f64>(sf);
                let z = m.size().to_logical::<f64>(sf);
                (p.x.round() as i32, p.y.round() as i32, z.width.round() as u32, z.height.round() as u32)
            })
            .collect();
        let _ = window.set_size(LogicalSize::new(g.width, g.height));
        if visible_on(&g, &monitors) {
            let _ = window.set_position(LogicalPosition::new(g.x, g.y));
        } else {
            let _ = window.center();
        }
        if g.maximized {
            let _ = window.maximize();
        }
    }
    let _ = window.show();

    // Normal bounds come from the window while it isn't maximized/minimized.
    let current = |w: &WebviewWindow<R>, prev: Option<WindowGeometry>| -> Option<WindowGeometry> {
        let maximized = w.is_maximized().unwrap_or(false);
        // Minimized, or in full screen (its frame is the whole screen): the
        // window's own bounds are the last ones seen.
        if w.is_minimized().unwrap_or(false) || w.is_fullscreen().unwrap_or(false) {
            return prev;
        }
        if maximized {
            return prev.map(|p| WindowGeometry { maximized: true, ..p });
        }
        let sf = w.scale_factor().ok()?;
        let (pos, size) = (w.outer_position().ok()?.to_logical::<f64>(sf), w.inner_size().ok()?.to_logical::<f64>(sf));
        Some(WindowGeometry { x: pos.x.round() as i32, y: pos.y.round() as i32, width: size.width.round() as u32, height: size.height.round() as u32, maximized: false })
    };

    let state = Arc::new(Mutex::new((current(window, saved).or(saved), Instant::now() - SAVE_EVERY)));
    let (w, f) = (window.clone(), file);
    window.on_window_event(move |e| {
        let closing = matches!(e, WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed);
        if !closing && !matches!(e, WindowEvent::Moved(_) | WindowEvent::Resized(_)) {
            return;
        }
        let mut st = state.lock();
        let next = current(&w, st.0);
        if let Some(g) = next {
            let changed = st.0 != Some(g);
            st.0 = Some(g);
            if closing || (changed && st.1.elapsed() >= SAVE_EVERY) {
                save(&f, &g);
                st.1 = Instant::now();
            }
        }
    });
}

/// A double-click in the top bar or the sidebar's header does what the user
/// chose for title bars in System Settings › Desktop & Dock: zoom (the
/// default), minimize, or nothing.
pub fn title_bar_double_click(window: &tauri::Window) {
    let w = window.clone();
    let _ = window.run_on_main_thread(move || {
        use objc2_foundation::{NSString, NSUserDefaults};
        let defaults = NSUserDefaults::standardUserDefaults();
        let action = defaults.stringForKey(&NSString::from_str("AppleActionOnDoubleClick")).map(|s| s.to_string());
        let legacy_minimize = defaults.boolForKey(&NSString::from_str("AppleMiniaturizeOnDoubleClick"));
        let Ok(ptr) = w.ns_window() else { return };
        if ptr.is_null() {
            return;
        }
        // SAFETY: Tauri's live NSWindow, on the main thread.
        let ns: &objc2_app_kit::NSWindow = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
        match double_click_action(action.as_deref(), legacy_minimize) {
            "minimize" => ns.performMiniaturize(None),
            "zoom" => ns.performZoom(None),
            _ => {}
        }
    });
}

/// "Maximize" (zoom), "Minimize" or "None", as macOS stores the choice.
pub fn double_click_action(pref: Option<&str>, legacy_minimize: bool) -> &'static str {
    match pref {
        Some("Minimize") => "minimize",
        Some("None") => "none",
        Some(_) => "zoom",
        None if legacy_minimize => "minimize",
        None => "zoom",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offscreen_positions_are_rejected() {
        let monitors = [(0, 0, 1920, 1080), (1920, 0, 2560, 1440)];
        let g = |x, y| WindowGeometry { x, y, width: 1200, height: 800, maximized: false };
        assert!(visible_on(&g(100, 100), &monitors));
        assert!(visible_on(&g(2500, 200), &monitors)); // second monitor
        assert!(!visible_on(&g(5000, 200), &monitors)); // monitor unplugged
        assert!(!visible_on(&g(100, 1070), &monitors)); // title bar below the screen
        assert!(!visible_on(&g(-1150, 100), &monitors)); // only 50px showing
    }

    #[test]
    fn title_bar_double_click_follows_the_system() {
        assert_eq!(double_click_action(None, false), "zoom");
        assert_eq!(double_click_action(Some("Maximize"), false), "zoom");
        assert_eq!(double_click_action(Some("Minimize"), false), "minimize");
        assert_eq!(double_click_action(Some("None"), false), "none");
        assert_eq!(double_click_action(None, true), "minimize");
    }

    #[test]
    fn round_trips_through_disk() {
        let p = std::env::temp_dir().join(format!("serica-window-{}.json", std::process::id()));
        let g = WindowGeometry { x: 10, y: 20, width: 1500, height: 950, maximized: true };
        save(&p, &g);
        assert_eq!(load(&p), Some(g));
        let _ = std::fs::remove_file(p);
    }
}
