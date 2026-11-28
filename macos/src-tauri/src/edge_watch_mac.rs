//! Tells the UI when the pointer heads for a window edge over a web page,
//! and where the pointer is over the window (`ui_cursor`).
//!
//! The sidebar, top bar and pins dock open over a live page by sliding over
//! a capture of it, and a capture takes 100-300 ms. The UI can't see the
//! pointer over the page (a separate native view), so this watches the
//! system cursor and sends `serica:edge-approach` as it nears the left, top
//! or right edge; the UI captures the page then, and the panel opens over it
//! at once when the pointer arrives.
//!
//! Positions are global display coordinates with a top-left origin (Quartz
//! event space, in points): the cursor from `CGEventGetLocation`, the window's
//! content rectangle cached from window events on the main thread, so this
//! thread never waits on the main thread.

use crate::app::{ui_emit, Shared};
use parking_lot::Mutex;
use std::time::{Duration, Instant};

/// How close to an edge (points) counts as heading for it.
const SIDE_BAND: f64 = 160.0;
const TOP_BAND: f64 = 90.0;
/// At most one capture per edge this often.
const REPEAT: Duration = Duration::from_millis(900);

/// The main window's content area in global points (top-left origin) and
/// whether Serica is in front with it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Geometry {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub focused: bool,
    pub minimized: bool,
}

static GEOMETRY: Mutex<Geometry> = Mutex::new(Geometry { x: 0.0, y: 0.0, w: 0.0, h: 0.0, focused: false, minimized: false });

pub fn geometry() -> Geometry {
    *GEOMETRY.lock()
}

/// Re-reads the window's content rectangle. Main thread (window events).
pub fn refresh(window: &tauri::Window) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let (Ok(pos), Ok(size)) = (window.inner_position(), window.inner_size()) else { return };
    let mut g = GEOMETRY.lock();
    g.x = pos.x as f64 / scale;
    g.y = pos.y as f64 / scale;
    g.w = size.width as f64 / scale;
    g.h = size.height as f64 / scale;
    g.minimized = window.is_minimized().unwrap_or(false);
    g.focused = window.is_focused().unwrap_or(false);
}

pub fn set_focused(focused: bool) {
    GEOMETRY.lock().focused = focused;
}

/// The system cursor in global points (top-left origin).
pub fn cursor() -> Option<(f64, f64)> {
    use objc2_core_graphics::CGEvent;
    let ev = CGEvent::new(None)?;
    let p = CGEvent::location(Some(&ev));
    Some((p.x, p.y))
}

/// Where the cursor is over the window: `(x, y, width)` in points of the
/// content area, or `None` outside it.
pub fn cursor_in_window() -> Option<(f64, f64, f64)> {
    let g = geometry();
    let (cx, cy) = cursor()?;
    let (x, y) = (cx - g.x, cy - g.y);
    (g.w > 0.0 && x >= 0.0 && y >= 0.0 && x < g.w && y < g.h).then_some((x, y, g.w))
}

/// Seconds since the last keyboard or mouse input anywhere in the session.
pub fn idle_seconds() -> f64 {
    use objc2_core_graphics::{CGEventSource, CGEventSourceStateID, CGEventType};
    // kCGAnyInputEventType = ~0
    let any = CGEventType(u32::MAX);
    CGEventSource::seconds_since_last_event_type(CGEventSourceStateID::CombinedSessionState, any)
}

pub fn start(app: Shared) {
    std::thread::Builder::new().name("serica-edges".into()).spawn(move || run(app)).ok();
}

/// Which edge band a point of the content area is in.
pub fn edge_at(x: f64, y: f64, w: f64, h: f64) -> Option<&'static str> {
    if x < 0.0 || y < 0.0 || x >= w || y >= h {
        None
    } else if x < SIDE_BAND {
        Some("left")
    } else if w - x < SIDE_BAND {
        Some("right")
    } else if y < TOP_BAND {
        Some("top")
    } else {
        None
    }
}

fn run(app: Shared) {
    let mut inside: Option<&'static str> = None;
    let mut sent: [Option<Instant>; 3] = [None; 3];
    loop {
        std::thread::sleep(Duration::from_millis(16));
        let g = geometry();
        if !g.focused || g.minimized || g.w <= 0.0 {
            inside = None;
            std::thread::sleep(Duration::from_millis(150));
            continue;
        }
        // Only over a live page (the UI handles its own pages itself).
        let page = match app.host.try_lock() {
            Some(h) => h.visible && h.active.as_ref().and_then(|id| h.views.get(id)).is_some_and(|v| v.shown),
            None => continue,
        };
        if !page {
            inside = None;
            continue;
        }
        let Some((cx, cy)) = cursor() else { continue };
        let edge = edge_at(cx - g.x, cy - g.y, g.w, g.h);
        if edge != inside {
            inside = edge;
            if let Some(e) = edge {
                let i = match e {
                    "left" => 0,
                    "right" => 1,
                    _ => 2,
                };
                if sent[i].is_none_or(|t| t.elapsed() >= REPEAT) {
                    sent[i] = Some(Instant::now());
                    ui_emit(&app.handle, "serica:edge-approach", e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_bands() {
        assert_eq!(edge_at(10.0, 300.0, 1280.0, 800.0), Some("left"));
        assert_eq!(edge_at(1270.0, 300.0, 1280.0, 800.0), Some("right"));
        assert_eq!(edge_at(600.0, 20.0, 1280.0, 800.0), Some("top"));
        assert_eq!(edge_at(600.0, 400.0, 1280.0, 800.0), None);
        assert_eq!(edge_at(-1.0, 400.0, 1280.0, 800.0), None);
    }

    #[test]
    fn cursor_and_idle_are_read() {
        assert!(cursor().is_some());
        assert!(idle_seconds() >= 0.0);
    }
}
