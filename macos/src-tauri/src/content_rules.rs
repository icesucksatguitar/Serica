//! Request blocking in WebKit: Shields' and Focus Mode's rules as
//! `WKContentRuleList`s.
//!
//! WebKit never shows an app a page's subresource requests (there is no
//! `WebResourceRequested`), so the filters run inside WebKit itself, as
//! Safari's content blockers do. Shields' network filters come from
//! `Shields::content_rule_chunks`; they are compiled into WebKit's rule list
//! store (kept across launches, found again by identifier) and put on every
//! tab's user content controller. A site allowed in Shields gets its tab's
//! Shields lists taken off for its pages (see `set_tab_shields`). Focus Mode
//! adds one small list for whole blocked sites. WebKit reports each block to
//! the navigation delegate, which feeds Shields' counters and live log.

use crate::app::Shared;
use crate::mac_util::{ns, post_main};
use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::{MainThreadMarker, Message};
use objc2_foundation::{NSArray, NSError, NSString};
use objc2_web_kit::{WKContentRuleList, WKContentRuleListStore, WKUserContentController};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Identifiers of Shields' compiled lists start with this.
pub const SHIELDS_PREFIX: &str = "serica-shields";
const FOCUS_PREFIX: &str = "serica-focus";

struct Controller {
    ucc: Weak<WKUserContentController>,
    /// Shields' lists are on it (its page's site is not allowed).
    shields: Cell<bool>,
    /// The assistant drawer: Focus Mode only.
    focus_only: bool,
}

thread_local! {
    static SHIELDS: RefCell<Vec<Retained<WKContentRuleList>>> = const { RefCell::new(Vec::new()) };
    static FOCUS: RefCell<Option<Retained<WKContentRuleList>>> = const { RefCell::new(None) };
    static CONTROLLERS: RefCell<Vec<Controller>> = const { RefCell::new(Vec::new()) };
    /// Shields switched on in Settings.
    static ENABLED: Cell<bool> = const { Cell::new(true) };
    static FOCUS_SEQ: Cell<u64> = const { Cell::new(0) };
}

fn store() -> Retained<WKContentRuleListStore> {
    let mtm = MainThreadMarker::new().expect("main thread");
    unsafe { WKContentRuleListStore::defaultStore(mtm) }.expect("content rule list store")
}

fn live_controllers(f: impl Fn(&WKUserContentController, &Controller)) {
    CONTROLLERS.with(|c| {
        let mut c = c.borrow_mut();
        c.retain(|x| x.ucc.load().is_some());
        for x in c.iter() {
            if let Some(u) = x.ucc.load() {
                f(&u, x);
            }
        }
    });
}

/// Puts the current lists on a new tab's controller. Main thread.
pub fn attach(ucc: &WKUserContentController) {
    let on = ENABLED.with(|e| e.get());
    unsafe {
        if on {
            SHIELDS.with(|s| s.borrow().iter().for_each(|l| ucc.addContentRuleList(l)));
        }
        FOCUS.with(|f| f.borrow().as_ref().map(|l| ucc.addContentRuleList(l)));
    }
    CONTROLLERS.with(|c| c.borrow_mut().push(Controller { ucc: Weak::from(ucc), shields: Cell::new(on), focus_only: false }));
}

/// The assistant drawer's controller: Focus Mode's list only.
pub fn attach_focus_only(ucc: &WKUserContentController) {
    unsafe { FOCUS.with(|f| f.borrow().as_ref().map(|l| ucc.addContentRuleList(l))) };
    CONTROLLERS.with(|c| c.borrow_mut().push(Controller { ucc: Weak::from(ucc), shields: Cell::new(false), focus_only: true }));
}

/// Shields on or off for the page a tab is about to load (off on sites the
/// user allowed, or when Shields are off). Main thread, before the page loads.
pub fn set_tab_shields(ucc: &WKUserContentController, active: bool) {
    let want = active && ENABLED.with(|e| e.get());
    CONTROLLERS.with(|c| {
        let c = c.borrow();
        let Some(x) = c.iter().find(|x| x.ucc.load().is_some_and(|u| std::ptr::eq(&*u, ucc))) else { return };
        if x.focus_only || x.shields.get() == want {
            return;
        }
        x.shields.set(want);
        SHIELDS.with(|s| {
            for l in s.borrow().iter() {
                unsafe {
                    if want {
                        ucc.addContentRuleList(l);
                    } else {
                        ucc.removeContentRuleList(l);
                    }
                }
            }
        });
    });
}

/// Shields switched on or off in Settings: every tab follows from its next
/// page (the caller reloads the page on screen).
pub fn set_enabled(handle: &tauri::AppHandle, on: bool) {
    post_main(handle, move || {
        if ENABLED.with(|e| e.replace(on)) == on {
            return;
        }
        live_controllers(|u, x| {
            if x.focus_only || x.shields.get() == on {
                return;
            }
            x.shields.set(on);
            SHIELDS.with(|s| {
                for l in s.borrow().iter() {
                    unsafe {
                        if on {
                            u.addContentRuleList(l);
                        } else {
                            u.removeContentRuleList(l);
                        }
                    }
                }
            });
        });
    });
}

/// Compiles (or finds, when already compiled) the lists named `ids` from
/// `sources`, then hands them to `done` (only if all of them compiled; a list
/// that WebKit refuses is split in two and retried, down to small pieces).
fn compile(ids: Vec<String>, sources: Vec<String>, done: impl FnOnce(Vec<Retained<WKContentRuleList>>) + 'static) {
    let n = ids.len();
    let results: Rc<RefCell<Vec<Option<Vec<Retained<WKContentRuleList>>>>>> = Rc::new(RefCell::new(vec![None; n]));
    let left = Rc::new(Cell::new(n));
    let done: Rc<RefCell<Option<Box<dyn FnOnce(Vec<Retained<WKContentRuleList>>)>>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    if n == 0 {
        if let Some(d) = done.borrow_mut().take() {
            d(Vec::new());
        }
        return;
    }
    for (i, (id, src)) in ids.into_iter().zip(sources).enumerate() {
        let (results, left, done) = (results.clone(), left.clone(), done.clone());
        compile_one(
            id,
            src,
            0,
            Box::new(move |lists| {
                results.borrow_mut()[i] = Some(lists);
                left.set(left.get() - 1);
                if left.get() == 0 {
                    let all: Vec<Retained<WKContentRuleList>> = results.borrow_mut().drain(..).flatten().flatten().collect();
                    if let Some(d) = done.borrow_mut().take() {
                        d(all);
                    }
                }
            }),
        );
    }
}

/// Called with the compiled lists (boxed: the bisection below recurses).
type Compiled = Box<dyn FnOnce(Vec<Retained<WKContentRuleList>>)>;

fn compile_one(id: String, src: String, depth: u32, done: Compiled) {
    let st = store();
    let done = Rc::new(RefCell::new(Some(done)));
    let (id2, src2) = (id.clone(), src.clone());
    let lookup_done = RcBlock::new(move |list: *mut WKContentRuleList, _err: *mut NSError| {
        if let Some(list) = unsafe { list.as_ref() } {
            if let Some(d) = done.borrow_mut().take() {
                d(vec![list.retain()]);
            }
            return;
        }
        let done = done.clone();
        let (id3, src3) = (id2.clone(), src2.clone());
        let compiled = RcBlock::new(move |list: *mut WKContentRuleList, err: *mut NSError| {
            if let Some(list) = unsafe { list.as_ref() } {
                if let Some(d) = done.borrow_mut().take() {
                    d(vec![list.retain()]);
                }
                return;
            }
            let why = unsafe { err.as_ref() }.map(|e| e.localizedDescription().to_string()).unwrap_or_default();
            // Split a refused list in two (by rules), down to small pieces.
            let rules: Vec<serde_json::Value> = serde_json::from_str(&src3).unwrap_or_default();
            if depth >= 6 || rules.len() < 64 {
                tracing::warn!("WebKit refused content rule list {id3} ({} rules): {why}", rules.len());
                if let Some(d) = done.borrow_mut().take() {
                    d(Vec::new());
                }
                return;
            }
            // Exceptions belong with every part.
            let exc: Vec<serde_json::Value> = rules.iter().filter(|r| r["action"]["type"] == "ignore-previous-rules").cloned().collect();
            let blocks: Vec<serde_json::Value> = rules.into_iter().filter(|r| r["action"]["type"] != "ignore-previous-rules").collect();
            let mid = blocks.len() / 2;
            let part = |b: &[serde_json::Value]| {
                let mut v = b.to_vec();
                v.extend(exc.iter().cloned());
                serde_json::to_string(&v).unwrap_or_else(|_| "[]".into())
            };
            let (a, b) = (part(&blocks[..mid]), part(&blocks[mid..]));
            let pieces = Rc::new(RefCell::new(Vec::new()));
            let left = Rc::new(Cell::new(2));
            for (k, s) in [a, b].into_iter().enumerate() {
                let (pieces, left, done) = (pieces.clone(), left.clone(), done.clone());
                compile_one(
                    format!("{id3}.{k}"),
                    s,
                    depth + 1,
                    Box::new(move |lists| {
                        pieces.borrow_mut().extend(lists);
                        left.set(left.get() - 1);
                        if left.get() == 0 {
                            if let Some(d) = done.borrow_mut().take() {
                                d(pieces.borrow_mut().drain(..).collect());
                            }
                        }
                    }),
                );
            }
        });
        unsafe { store().compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(Some(&ns(&id2)), Some(&ns(&src2)), Some(&*compiled)) };
    });
    unsafe { st.lookUpContentRuleListForIdentifier_completionHandler(Some(&ns(&id)), Some(&*lookup_done)) };
}

/// Removes compiled lists with `prefix` other than `keep` (old versions).
fn prune(prefix: &'static str, keep: Vec<String>) {
    let st = store();
    let blk = RcBlock::new(move |ids: *mut NSArray<NSString>| {
        let Some(ids) = (unsafe { ids.as_ref() }) else { return };
        for id in ids.iter() {
            let s = id.to_string();
            if s.starts_with(prefix) && !keep.iter().any(|k| s == *k || s.starts_with(&format!("{k}."))) {
                let none = RcBlock::new(|_e: *mut NSError| {});
                unsafe { store().removeContentRuleListForIdentifier_completionHandler(Some(&id), Some(&*none)) };
            }
        }
    });
    unsafe { st.getAvailableContentRuleListIdentifiers(Some(&*blk)) };
}

/// Swaps Shields' lists on every tab.
fn install_shields(lists: Vec<Retained<WKContentRuleList>>) {
    let old: Vec<Retained<WKContentRuleList>> = SHIELDS.with(|s| std::mem::replace(&mut *s.borrow_mut(), lists.clone()));
    live_controllers(|u, x| {
        if !x.shields.get() {
            return;
        }
        unsafe {
            for l in &old {
                u.removeContentRuleList(l);
            }
            for l in &lists {
                u.addContentRuleList(l);
            }
        }
    });
}

/// Compiles Shields' current filters for WebKit and puts them on every tab.
pub fn rebuild(app: &Shared) {
    let a = app.clone();
    std::thread::Builder::new()
        .name("serica-content-rules".into())
        .spawn(move || {
            let started = std::time::Instant::now();
            let Some((hash, chunks)) = a.shields.content_rule_chunks() else { return };
            let ids: Vec<String> = (0..chunks.len()).map(|i| format!("{SHIELDS_PREFIX}-{hash}-{i}")).collect();
            post_main(&a.handle, move || {
                let keep = ids.clone();
                compile(ids, chunks, move |lists| {
                    tracing::info!("shields: {} content rule lists ready in {:?}", lists.len(), started.elapsed());
                    install_shields(lists);
                    prune(SHIELDS_PREFIX, keep);
                });
            });
        })
        .ok();
}

/// Focus Mode's rules changed: its list follows (or goes).
pub fn update_focus(app: &Shared) {
    let rules = app.focus.snapshot().map(|c| c.content_rules()).unwrap_or_default();
    post_main(&app.handle, move || {
        let seq = FOCUS_SEQ.with(|s| {
            s.set(s.get() + 1);
            s.get()
        });
        let apply = move |list: Option<Retained<WKContentRuleList>>| {
            // A newer change already won.
            if FOCUS_SEQ.with(|s| s.get()) != seq {
                return;
            }
            let old = FOCUS.with(|f| std::mem::replace(&mut *f.borrow_mut(), list.clone()));
            live_controllers(|u, _| unsafe {
                if let Some(o) = &old {
                    u.removeContentRuleList(o);
                }
                if let Some(n) = &list {
                    u.addContentRuleList(n);
                }
            });
        };
        if rules.is_empty() {
            apply(None);
            prune(FOCUS_PREFIX, Vec::new());
            return;
        }
        let src = serde_json::to_string(&rules).unwrap_or_else(|_| "[]".into());
        use sha2::Digest;
        let hash: String = sha2::Sha256::digest(src.as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect();
        let id = format!("{FOCUS_PREFIX}-{hash}");
        let keep = vec![id.clone()];
        compile(vec![id], vec![src], move |lists| {
            apply(lists.into_iter().next());
            prune(FOCUS_PREFIX, keep);
        });
    });
}

/// At start: the hooks, then the lists from the cache (or compiled now).
pub fn start(app: &Shared) {
    let a = app.clone();
    app.shields.set_on_rebuilt(move || rebuild(&a));
    let on = app.shields.enabled();
    post_main(&app.handle, move || ENABLED.with(|e| e.set(on)));
    rebuild(app);
    update_focus(app);
}

/// A blocked request's kind, from its URL (WebKit doesn't say), for the
/// Shields panel's "data saved" estimate.
pub fn guess_type(url: &str) -> &'static str {
    let path = url::Url::parse(url).map(|u| u.path().to_ascii_lowercase()).unwrap_or_default();
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "js" | "mjs" => "script",
        "css" => "stylesheet",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "svg" | "ico" => "image",
        "woff" | "woff2" | "ttf" | "otf" => "font",
        "mp4" | "webm" | "m3u8" | "mp3" | "m4a" | "ts" => "media",
        "html" | "htm" => "sub_frame",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_kinds_from_urls() {
        assert_eq!(guess_type("https://a.test/x/y.js?v=1"), "script");
        assert_eq!(guess_type("https://a.test/pixel.gif"), "image");
        assert_eq!(guess_type("https://a.test/collect"), "other");
    }
}
