//! Holes in the page views: rounded rectangles of UI that show through a
//! live page (the sidebar, the dock, menus, cards), cut out without freezing
//! the page.
//!
//! The Windows build cuts a window region (`SetWindowRgn`) on a worker
//! thread. Here each page view is a layer-backed `WKWebView`; its layer gets
//! a `CAShapeLayer` mask: the view's bounds minus the union of the holes,
//! as one path, set inside a transaction with implicit animations off (so
//! Core Animation never adds its own fade on top of the UI's curve). Moving
//! holes are evaluated once per display refresh by a `CADisplayLink` (60 Hz,
//! 120 Hz on ProMotion displays), from the same curve and start time as the
//! UI's CSS transition. The pages' hit testing follows the mask
//! (`mac_tabview`), so the UI under a hole takes the pointer too.

use crate::mac_util::post_main;
use crate::tabhost::{cubic_bezier, Hole};
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::NSView;
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColor, CGMutablePath, CGPath};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRunLoop, NSRunLoopCommonModes};
use objc2_quartz_core::{CADisplayLink, CALayer, CAShapeLayer, CATransaction};
use parking_lot::{Condvar, Mutex};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
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
    /// Page origin in points (window coordinates, top-left).
    origin: (f64, f64),
    /// Text size: UI CSS px to points.
    z: f64,
    /// Bumped on every change; `applied` is the last one on screen.
    seq: u64,
    applied: u64,
}

pub struct Clipper {
    st: Mutex<State>,
    done: Condvar,
    handle: OnceLock<tauri::AppHandle>,
    /// A main-thread pass is queued.
    queued: AtomicBool,
}

/// A hole rectangle in a page view's own coordinates (points, top-left
/// origin): x, y, w, h, corner radius.
type Rect5 = [f64; 5];

struct View {
    view: Weak<NSView>,
    /// What its mask cuts now (also used for hit testing).
    cut: Vec<Rect5>,
    /// HTML fullscreen: WebKit shows the view in its own window, uncut.
    fullscreen: bool,
}

thread_local! {
    /// Page views by webview label. Main thread only.
    static VIEWS: RefCell<HashMap<String, View>> = RefCell::new(HashMap::new());
    static LINK: RefCell<Option<(Retained<CADisplayLink>, Retained<SericaClipTicker>)>> = const { RefCell::new(None) };
    static CLIPPER: RefCell<Option<Arc<Clipper>>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct SericaClipTicker;

    unsafe impl NSObjectProtocol for SericaClipTicker {}

    impl SericaClipTicker {
        #[unsafe(method(tick:))]
        fn tick(&self, _link: &AnyObject) {
            if let Some(c) = CLIPPER.with(|c| c.borrow().clone()) {
                c.pass();
            }
        }
    }
);

impl Clipper {
    pub fn new() -> Arc<Self> {
        Arc::new(Clipper { st: Mutex::new(State { z: 1.0, ..Default::default() }), done: Condvar::new(), handle: OnceLock::new(), queued: AtomicBool::new(false) })
    }

    /// The app handle, for posting passes to the main thread.
    pub fn start(self: &Arc<Self>, handle: tauri::AppHandle) {
        let _ = self.handle.set(handle.clone());
        let me = self.clone();
        post_main(&handle, move || CLIPPER.with(|c| *c.borrow_mut() = Some(me)));
    }

    fn changed(self: &Arc<Self>, st: &mut State) -> u64 {
        st.seq += 1;
        let seq = st.seq;
        if !self.queued.swap(true, Ordering::AcqRel) {
            if let Some(h) = self.handle.get() {
                let me = self.clone();
                post_main(h, move || {
                    me.queued.store(false, Ordering::Release);
                    me.pass();
                });
            }
        }
        seq
    }

    /// The page webviews on screen and where the page area is.
    pub fn set_pages(self: &Arc<Self>, pages: Vec<String>, origin: (f64, f64), z: f64) -> u64 {
        let mut st = self.st.lock();
        st.pages = pages;
        st.origin = origin;
        st.z = if z.is_finite() && z > 0.0 { z } else { 1.0 };
        self.changed(&mut st)
    }

    /// Sets, animates (`from` + `ms`) or removes a hole. A change mid-animation
    /// continues from where the hole is now. Returns the change's sequence number.
    #[allow(clippy::too_many_arguments)]
    pub fn set_hole(self: &Arc<Self>, id: String, rect: Hole, from: Option<Hole>, ms: u32, ease: [f64; 4], remove: bool, lead: f64) -> u64 {
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
        if crate::mac_util::is_main_thread() {
            return; // the pass runs on this thread; never wait on ourselves
        }
        let deadline = Instant::now() + timeout;
        let mut st = self.st.lock();
        while st.applied < seq {
            if self.done.wait_until(&mut st, deadline).timed_out() {
                return;
            }
        }
    }

    /// Evaluates the holes now and applies them to the page views on screen.
    /// Main thread only.
    fn pass(&self) {
        let (seq, holes, pages, origin, z, animating) = {
            let mut st = self.st.lock();
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
            let animating = st.holes.values().any(|s| matches!(s, Slot::Moving(_)));
            (st.seq, holes, st.pages.clone(), st.origin, st.z, animating)
        };
        let rects = rects_for(&holes, origin, z);
        VIEWS.with(|v| {
            let mut v = v.borrow_mut();
            v.retain(|_, view| view.view.load().is_some());
            for (label, view) in v.iter_mut() {
                let Some(nv) = view.view.load() else { continue };
                let want: Vec<Rect5> = if pages.contains(label) && !view.fullscreen { rects.clone() } else { Vec::new() };
                if want != view.cut {
                    let _p = crate::perf::span("clip.set_mask");
                    apply_mask(&nv, &want);
                    view.cut = want;
                }
            }
        });
        {
            let mut st = self.st.lock();
            if st.applied < seq {
                st.applied = seq;
            }
            self.done.notify_all();
        }
        set_ticking(animating);
    }
}

/// The holes in a page view's coordinates (points from its top-left).
fn rects_for(holes: &[Hole], origin: (f64, f64), z: f64) -> Vec<Rect5> {
    holes
        .iter()
        .filter(|h| h.w > 0.5 && h.h > 0.5)
        .map(|h| [h.x * z - origin.0, h.y * z - origin.1, h.w * z, h.h * z, (h.r * z).max(0.0)])
        .filter(|r| r[2] > 0.0 && r[3] > 0.0)
        .collect()
}

/// Starts or pauses the per-frame passes.
fn set_ticking(on: bool) {
    let mtm = MainThreadMarker::new().expect("main thread");
    LINK.with(|l| {
        let mut l = l.borrow_mut();
        if l.is_none() {
            if !on {
                return;
            }
            // A display link of the main window's screen (60/120 Hz).
            let Some(view) = VIEWS.with(|v| v.borrow().values().find_map(|x| x.view.load())) else { return };
            let ticker: Retained<SericaClipTicker> = unsafe { msg_send![SericaClipTicker::alloc(mtm), init] };
            let link = unsafe { view.displayLinkWithTarget_selector(&ticker, sel!(tick:)) };
            unsafe { link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes) };
            *l = Some((link, ticker));
        }
        if let Some((link, _)) = l.as_ref() {
            link.setPaused(!on);
        }
    });
}

/// Sets a view's mask to its bounds minus the union of `rects` (no mask
/// when there are none).
fn apply_mask(view: &NSView, rects: &[Rect5]) {
    unsafe {
        view.setWantsLayer(true);
        let Some(layer) = view.layer() else { return };
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        if rects.is_empty() {
            layer.setMask(None);
        } else {
            let bounds = view.bounds();
            let flipped = view.isFlipped();
            let holes = CGMutablePath::new();
            for r in rects {
                let y = if flipped { r[1] } else { bounds.size.height - r[1] - r[3] };
                let in_layer = view.convertRectToLayer(CGRect::new(CGPoint::new(r[0], y), CGSize::new(r[2], r[3])));
                let rad = r[4].min(in_layer.size.width / 2.0).min(in_layer.size.height / 2.0).max(0.0);
                if rad > 0.5 {
                    CGMutablePath::add_rounded_rect(Some(&holes), std::ptr::null(), in_layer, rad, rad);
                } else {
                    CGMutablePath::add_rect(Some(&holes), std::ptr::null(), in_layer);
                }
            }
            // Larger than the view: the page keeps its mask while it grows.
            let lb = layer.bounds();
            let outer = CGMutablePath::new();
            CGMutablePath::add_rect(Some(&outer), std::ptr::null(), CGRect::new(CGPoint::new(lb.origin.x - 4000.0, lb.origin.y - 4000.0), CGSize::new(lb.size.width + 8000.0, lb.size.height + 8000.0)));
            let shape: Retained<CAShapeLayer> = match layer.mask().and_then(|m| m.downcast::<CAShapeLayer>().ok()) {
                Some(s) => s,
                None => {
                    let s = CAShapeLayer::new();
                    s.setFillColor(Some(&CGColor::new_generic_gray(0.0, 1.0)));
                    s
                }
            };
            shape.setFrame(lb);
            match CGPath::new_copy_by_subtracting_path(Some(&outer), Some(&holes), false) {
                Some(path) => {
                    shape.setFillRule(objc2_quartz_core::kCAFillRuleNonZero);
                    shape.setPath(Some(&path));
                }
                None => {
                    // No path operations: outer + holes, even-odd.
                    let both = CGMutablePath::new();
                    CGMutablePath::add_path(Some(&both), std::ptr::null(), Some(&outer));
                    CGMutablePath::add_path(Some(&both), std::ptr::null(), Some(&holes));
                    shape.setFillRule(objc2_quartz_core::kCAFillRuleEvenOdd);
                    shape.setPath(Some(&both));
                }
            }
            let as_layer: &CALayer = &shape;
            layer.setMask(Some(as_layer));
        }
        CATransaction::commit();
    }
}

/// Starts clipping a page webview (once it exists).
pub fn register(wv: &tauri::Webview) {
    let label = wv.label().to_string();
    crate::webview_mac::with_wk(wv, move |wk| {
        let view: &NSView = wk.as_ref();
        VIEWS.with(|v| v.borrow_mut().insert(label, View { view: Weak::from(view), cut: Vec::new(), fullscreen: false }));
    });
}

/// A page view is going away.
pub fn forget(wk: &objc2_web_kit::WKWebView) {
    let p = wk as *const _ as usize;
    VIEWS.with(|v| v.borrow_mut().retain(|_, x| x.view.load().map(|n| &*n as *const NSView as usize) != Some(p)));
}

/// HTML fullscreen: the view is shown uncut while WebKit has it in its own window.
pub fn set_fullscreen(wk: &objc2_web_kit::WKWebView, fs: bool) {
    let p = wk as *const _ as usize;
    VIEWS.with(|v| {
        for x in v.borrow_mut().values_mut() {
            if x.view.load().map(|n| &*n as *const NSView as usize) == Some(p) {
                x.fullscreen = fs;
                if fs {
                    if let Some(n) = x.view.load() {
                        apply_mask(&n, &[]);
                    }
                    x.cut.clear();
                }
            }
        }
    });
    if let Some(c) = CLIPPER.with(|c| c.borrow().clone()) {
        c.pass();
    }
}

/// Whether `point` (the view's own coordinates) is inside one of its holes.
pub fn in_hole(view: &NSView, point: NSPoint) -> bool {
    let p = view as *const NSView as usize;
    let flipped = view.isFlipped();
    let h = view.bounds().size.height;
    let y = if flipped { point.y } else { h - point.y };
    VIEWS.with(|v| {
        v.borrow().values().any(|x| {
            x.view.load().map(|n| &*n as *const NSView as usize) == Some(p)
                && x.cut.iter().any(|r| point.x >= r[0] && point.x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3] && in_rounded(point.x - r[0], y - r[1], r))
        })
    })
}

/// Outside the rounded corners counts as outside the hole.
fn in_rounded(x: f64, y: f64, r: &Rect5) -> bool {
    let rad = r[4].min(r[2] / 2.0).min(r[3] / 2.0);
    if rad <= 0.5 {
        return true;
    }
    let cx = if x < rad { rad } else if x > r[2] - rad { r[2] - rad } else { return true };
    let cy = if y < rad { rad } else if y > r[3] - rad { r[3] - rad } else { return true };
    (x - cx).powi(2) + (y - cy).powi(2) <= rad * rad
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holes_follow_the_page_origin_and_text_size() {
        let h = Hole { x: 10.0, y: 50.0, w: 100.0, h: 40.0, r: 8.0 };
        let r = rects_for(&[h], (8.0, 35.0), 1.25);
        assert_eq!(r, vec![[4.5, 27.5, 125.0, 50.0, 10.0]]);
        assert!(rects_for(&[Hole { w: 0.0, ..h }], (0.0, 0.0), 1.0).is_empty());
    }

    #[test]
    fn rounded_corners_are_outside() {
        let r = [0.0, 0.0, 100.0, 100.0, 20.0];
        assert!(in_rounded(50.0, 50.0, &r));
        assert!(in_rounded(10.0, 50.0, &r));
        assert!(!in_rounded(1.0, 1.0, &r));
        assert!(in_rounded(20.0, 20.0, &r));
    }
}
