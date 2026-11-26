//! Tells the UI when the pointer heads for a window edge over a web page.
//!
//! The sidebar, top bar and pins dock open over a live page by sliding over a
//! capture of it, and a capture takes 100-300 ms (more on a busy page). The UI
//! cannot see the pointer over the page (a separate native window), so this
//! watches the OS cursor and sends `serica:edge-approach` as it nears the left,
//! top or right edge; the UI captures the page then, and the panel opens over
//! it at once when the pointer arrives.
use crate::app::{ui_emit, Shared};
use std::time::{Duration, Instant};

/// How close to an edge (logical px) counts as heading for it.
const SIDE_BAND: f64 = 160.0;
const TOP_BAND: f64 = 90.0;
/// At most one capture per edge this often.
const REPEAT: Duration = Duration::from_millis(900);

pub fn start(app: Shared) {
    std::thread::Builder::new()
        .name("serica-edges".into())
        .spawn(move || run(app))
        .ok();
}

fn run(app: Shared) {
    use tauri::Manager;
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow, GetWindowRect, IsIconic};
    let hwnd = loop {
        if let Some(w) = app.handle.get_window("main").and_then(|w| w.hwnd().ok()) {
            break HWND(w.0 as _);
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    let mut inside: Option<&'static str> = None;
    let mut sent: [Option<Instant>; 3] = [None; 3];
    loop {
        std::thread::sleep(Duration::from_millis(16));
        unsafe {
            if GetForegroundWindow() != hwnd || IsIconic(hwnd).as_bool() {
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
            let (mut p, mut r) = (POINT::default(), RECT::default());
            if GetCursorPos(&mut p).is_err() || GetWindowRect(hwnd, &mut r).is_err() {
                continue;
            }
            let sf = match GetDpiForWindow(hwnd) {
                0 => 1.0,
                d => d as f64 / 96.0,
            };
            let (x, y) = (p.x - r.left, p.y - r.top);
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let edge = if x < 0 || y < 0 || x >= w || y >= h {
                None
            } else if (x as f64) < SIDE_BAND * sf {
                Some("left")
            } else if ((w - x) as f64) < SIDE_BAND * sf {
                Some("right")
            } else if (y as f64) < TOP_BAND * sf {
                Some("top")
            } else {
                None
            };
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
}
