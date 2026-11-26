//! Page window regions ("holes" UI shows through), applied off the main thread.
//!
//! Every tab page is a native child window above the UI. Cutting a hole is a
//! `SetWindowRgn` on the page's container window. That used to be posted to
//! the main thread like every other webview call, so a hole waited behind
//! whatever the main thread was doing: building a tab's webview blocks it for
//! 100-350 ms, during which the sidebar, top bar and dock could not open, and
//! an animated hole fell behind the UI it uncovers (the page drew over it).
//!
//! Here one worker thread owns the regions. `SetWindowRgn` from another thread
//! is delivered to the window as a sent message, which the main thread handles
//! as soon as it pumps messages (also while it waits for a webview to be
//! built), ahead of everything queued. Animated holes are evaluated on this
//! thread once per display refresh (`DwmFlush`), from the same curve and start
//! time as the UI's CSS transition.
use crate::tabhost::{cubic_bezier, Hole};
use parking_lot::{Condvar, Mutex};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
struct Anim {
    from: Hole,
    to: Hole,
    start: Instant,
    ms: f64,
    ease: [f64; 4],
    lead: f64,
    remove: bool,
}

impl Anim {
    fn progress(&self, now: Instant) -> f64 {
        ((now.saturating_duration_since(self.start).as_secs_f64() * 1000.0 + self.lead) / self.ms).clamp(0.0, 1.0)
    }
    fn at(&self, now: Instant) -> Hole {
        Hole::lerp(self.from, self.to, cubic_bezier(self.ease, self.progress(now)))
    }
}

#[derive(Clone, Copy)]
enum Slot {
    Still(Hole),
    Moving(Anim),
}

#[derive(Default)]
struct State {
    holes: HashMap<String, Slot>,
    /// Labels of the page webviews on screen.
    pages: Vec<String>,
    /// Container window of each page webview, by label.
    windows: HashMap<String, isize>,
    /// Page origin in logical px (zoomed by the text size).
    origin: (f64, f64),
    /// Text size: UI CSS px to logical px.
    z: f64,
    /// Bumped on every change; `applied` is the last one on screen.
    seq: u64,
    applied: u64,
}

pub struct Clipper {
    st: Mutex<State>,
    wake: Condvar,
    done: Condvar,
}

impl Clipper {
    pub fn new() -> Arc<Self> {
        let c = Arc::new(Clipper { st: Mutex::new(State { z: 1.0, ..Default::default() }), wake: Condvar::new(), done: Condvar::new() });
        let w = c.clone();
        std::thread::Builder::new().name("serica-clip".into()).spawn(move || w.run()).expect("clip thread");
        c
    }

    fn changed(&self, st: &mut State) -> u64 {
        st.seq += 1;
        self.wake.notify_one();
        st.seq
    }

    /// The container window of a page webview (found once, on the main thread).
    pub fn register(&self, label: String, hwnd: isize) {
        let mut st = self.st.lock();
        st.windows.insert(label, hwnd);
        self.changed(&mut st);
    }

    pub fn forget(&self, label: &str) {
        let mut st = self.st.lock();
        st.windows.remove(label);
        st.pages.retain(|l| l != label);
    }

    /// The page webviews on screen and where the page area is.
    pub fn set_pages(&self, pages: Vec<String>, origin: (f64, f64), z: f64) -> u64 {
        let mut st = self.st.lock();
        st.pages = pages;
        st.origin = origin;
        st.z = if z.is_finite() && z > 0.0 { z } else { 1.0 };
        self.changed(&mut st)
    }

    /// Sets, animates (`from` + `ms`) or removes a hole. A change mid-animation
    /// continues from where the hole is now. Returns the change's sequence number.
    #[allow(clippy::too_many_arguments)]
    pub fn set_hole(&self, id: String, rect: Hole, from: Option<Hole>, ms: u32, ease: [f64; 4], remove: bool, lead: f64) -> u64 {
        let mut st = self.st.lock();
        let now = Instant::now();
        let current = st.holes.get(&id).map(|s| match s {
            Slot::Still(h) => *h,
            Slot::Moving(a) => a.at(now),
        });
        match from.filter(|_| ms > 0) {
            Some(f) => {
                let anim = Anim { from: current.unwrap_or(f), to: rect, start: now, ms: ms as f64, ease, lead, remove };
                st.holes.insert(id, Slot::Moving(anim));
            }
            None if remove => {
                st.holes.remove(&id);
            }
            None => {
                st.holes.insert(id, Slot::Still(rect));
            }
        }
        self.changed(&mut st)
    }

    /// Waits (up to `timeout`) until change `seq` is on screen.
    pub fn wait_applied(&self, seq: u64, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut st = self.st.lock();
        while st.applied < seq {
            if self.done.wait_until(&mut st, deadline).timed_out() {
                return;
            }
        }
    }

    fn run(self: Arc<Self>) {
        // Region last set on each window (left, top, right, bottom, diameter per hole).
        let mut last: HashMap<isize, Vec<[i32; 5]>> = HashMap::new();
        let mut shown_before: HashSet<isize> = HashSet::new();
        loop {
            let (seq, holes, targets, origin, z, animating) = {
                let mut st = self.st.lock();
                let moving = |st: &State| st.holes.values().any(|s| matches!(s, Slot::Moving(_)));
                while st.applied == st.seq && !moving(&st) {
                    self.wake.wait(&mut st);
                }
                let now = Instant::now();
                let mut holes = Vec::with_capacity(st.holes.len());
                let mut finished = Vec::new();
                for (id, slot) in st.holes.iter() {
                    match slot {
                        Slot::Still(h) => holes.push(*h),
                        Slot::Moving(a) => {
                            if a.progress(now) >= 1.0 {
                                finished.push(id.clone());
                                if !a.remove {
                                    holes.push(a.to);
                                }
                            } else {
                                holes.push(a.at(now));
                            }
                        }
                    }
                }
                for id in finished {
                    if let Some(Slot::Moving(a)) = st.holes.get(&id).copied() {
                        if a.remove {
                            st.holes.remove(&id);
                        } else {
                            st.holes.insert(id, Slot::Still(a.to));
                        }
                    }
                }
                let targets: Vec<isize> = st.pages.iter().filter_map(|l| st.windows.get(l).copied()).collect();
                let animating = moving(&st);
                (st.seq, holes, targets, st.origin, st.z, animating)
            };
            let now_shown: HashSet<isize> = targets.iter().copied().collect();
            for hwnd in &targets {
                // A window that just came on screen gets its region set again.
                if !shown_before.contains(hwnd) {
                    last.remove(hwnd);
                }
                let region = region_for(*hwnd, &holes, origin, z);
                if last.get(hwnd) != Some(&region) {
                    {
                        let _p = crate::perf::span("clip.set_region");
                        apply(*hwnd, &region);
                    }
                    last.insert(*hwnd, region);
                }
            }
            last.retain(|h, _| now_shown.contains(h) || shown_before.contains(h));
            shown_before = now_shown;
            {
                let mut st = self.st.lock();
                if st.applied < seq {
                    st.applied = seq;
                }
                self.done.notify_all();
            }
            if animating {
                // Next display refresh (falls back to ~4 ms steps without composition).
                if unsafe { windows::Win32::Graphics::Dwm::DwmFlush() }.is_err() {
                    std::thread::sleep(Duration::from_millis(4));
                }
            }
        }
    }
}

/// The holes of one page window in its own physical client px.
fn region_for(hwnd: isize, holes: &[Hole], origin: (f64, f64), z: f64) -> Vec<[i32; 5]> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    let dpi = unsafe { GetDpiForWindow(HWND(hwnd as _)) };
    let sf = if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 };
    holes
        .iter()
        .filter(|h| h.w > 0.5 && h.h > 0.5)
        .filter_map(|h| {
            let l = ((h.x * z - origin.0) * sf).floor() as i32;
            let t = ((h.y * z - origin.1) * sf).floor() as i32;
            let r = (((h.x + h.w) * z - origin.0) * sf).ceil() as i32;
            let b = (((h.y + h.h) * z - origin.1) * sf).ceil() as i32;
            let d = (h.r * z * sf * 2.0).round() as i32;
            (r > l && b > t).then_some([l, t, r, b, d])
        })
        .collect()
}

fn apply(hwnd: isize, region: &[[i32; 5]]) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, SetWindowRgn, HGDIOBJ, RGN_DIFF};
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;
    let hwnd = HWND(hwnd as _);
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return;
        }
        if region.is_empty() {
            let _ = SetWindowRgn(hwnd, None, true);
            return;
        }
        // Larger than any window: the page keeps its region when it grows.
        let full = CreateRectRgn(-16000, -16000, 32000, 32000);
        for &[l, t, r, b, d] in region {
            // Round-rect regions exclude their right and bottom edges.
            let hole = if d > 1 { CreateRoundRectRgn(l, t, r + 1, b + 1, d, d) } else { CreateRectRgn(l, t, r, b) };
            CombineRgn(Some(full), Some(full), Some(hole), RGN_DIFF);
            let _ = DeleteObject(HGDIOBJ(hole.0));
        }
        // The system owns the region from here on.
        let _ = SetWindowRgn(hwnd, Some(full), true);
    }
}
