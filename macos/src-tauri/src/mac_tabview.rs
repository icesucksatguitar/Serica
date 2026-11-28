//! What Serica changes on a tab's WKWebView itself (an Objective-C subclass
//! of wry's view, put on the instance when the tab is attached):
//!
//! * keys: Serica's shortcuts are taken before the page (like the Windows
//!   build's accelerator handler); every other key equivalent goes to the
//!   page first and then the menu bar, as in Safari (wry's child views would
//!   send every Command key straight to the menu bar, past the page);
//! * right-click: WebKit's menu becomes Serica's (see `webview_mac::menu`);
//! * holes: where UI shows through the page (`clip_mac`), the page neither
//!   takes clicks, scrolls nor hover, so the UI beneath gets them.

use crate::app::Shortcut;
use crate::mac_keys::{shortcut_for, Key};
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{msg_send, sel, ClassType};
use objc2_app_kit::{NSEvent, NSMenu, NSView};
use objc2_foundation::NSPoint;
use objc2_web_kit::WKWebView;
use std::cell::Cell;
use std::sync::OnceLock;

/// (subclass, the class it was made from).
static CLASS: OnceLock<(usize, usize)> = OnceLock::new();

thread_local! {
    /// Timestamp of the last Escape reported, so WebKit re-sending an
    /// unhandled key event doesn't report it twice.
    static LAST_ESC: Cell<f64> = const { Cell::new(-1.0) };
}

fn base_class() -> &'static AnyClass {
    let (_, base) = *CLASS.get().expect("tab view class");
    unsafe { &*(base as *const AnyClass) }
}

fn key_of(event: &NSEvent) -> (u16, String, u64) {
    let code = event.keyCode();
    let chars = event.charactersIgnoringModifiers().map(|s| s.to_string().to_lowercase()).unwrap_or_default();
    let flags = event.modifierFlags().0 as u64;
    (code, chars, flags)
}

fn shortcut_of(event: &NSEvent) -> Option<Shortcut> {
    let (code, chars, flags) = key_of(event);
    shortcut_for(Key { code, chars: &chars, flags })
}

fn dispatch(sc: Shortcut, event: &NSEvent) {
    if event.isARepeat() && !matches!(sc, Shortcut::CycleTab(_)) {
        return;
    }
    if let Some(app) = crate::app::global() {
        app.shortcut(sc);
    }
}

/// Whether the view (or a view inside it) has keyboard focus.
fn is_focused(view: &NSView) -> bool {
    let Some(window) = view.window() else { return false };
    let Some(first) = window.firstResponder() else { return false };
    if std::ptr::eq(&*first as *const _ as *const AnyObject, view as *const NSView as *const AnyObject) {
        return true;
    }
    first.downcast_ref::<NSView>().is_some_and(|v| v.isDescendantOf(view))
}

extern "C-unwind" fn perform_key_equivalent(this: *mut NSView, _sel: Sel, event: *mut NSEvent) -> Bool {
    let this = unsafe { &*this };
    let event = unsafe { &*event };
    if !is_focused(this) {
        return Bool::NO;
    }
    if let Some(sc) = shortcut_of(event) {
        if !matches!(sc, Shortcut::Escape) {
            dispatch(sc, event);
            return Bool::YES;
        }
    }
    // The page first; WebKit hands keys the page doesn't use to the menu bar.
    unsafe { msg_send![super(this, WKWebView::class()), performKeyEquivalent: event] }
}

extern "C-unwind" fn key_down(this: *mut NSView, _sel: Sel, event: *mut NSEvent) {
    let this = unsafe { &*this };
    let event = unsafe { &*event };
    match shortcut_of(event) {
        Some(Shortcut::Escape) => {
            // Escape still reaches the page (closing its own dialogs).
            let t = event.timestamp();
            if LAST_ESC.with(|l| l.replace(t)) != t {
                dispatch(Shortcut::Escape, event);
            }
        }
        Some(sc) => {
            dispatch(sc, event);
            return;
        }
        None => {}
    }
    unsafe { msg_send![super(this, base_class()), keyDown: event] }
}

extern "C-unwind" fn will_open_menu(this: *mut NSView, _sel: Sel, menu: *mut NSMenu, event: *mut NSEvent) {
    let this = unsafe { &*this };
    let menu = unsafe { &*menu };
    let event = unsafe { event.as_ref() };
    if let Some(wk) = this.downcast_ref::<WKWebView>() {
        crate::webview_mac::menu::handle_context_menu(wk, menu, event);
    }
    unsafe { msg_send![super(this, base_class()), willOpenMenu: menu, withEvent: event] }
}

extern "C-unwind" fn hit_test(this: *mut NSView, _sel: Sel, point: NSPoint) -> *mut NSView {
    let this = unsafe { &*this };
    let hit: *mut NSView = unsafe { msg_send![super(this, base_class()), hitTest: point] };
    if hit.is_null() {
        return hit;
    }
    // `point` is in the superview's coordinates.
    let superview = unsafe { this.superview() };
    let local = this.convertPoint_fromView(point, superview.as_deref());
    if crate::clip_mac::in_hole(this, local) {
        return std::ptr::null_mut();
    }
    hit
}

/// Pointer moves inside a hole belong to the UI beneath: the page must not
/// show hover effects or set the cursor there.
extern "C-unwind" fn mouse_moved(this: *mut NSView, _sel: Sel, event: *mut NSEvent) {
    let this = unsafe { &*this };
    let event = unsafe { &*event };
    let local = this.convertPoint_fromView(event.locationInWindow(), None);
    if crate::clip_mac::in_hole(this, local) {
        return;
    }
    unsafe { msg_send![super(this, base_class()), mouseMoved: event] }
}

extern "C-unwind" fn cursor_update(this: *mut NSView, _sel: Sel, event: *mut NSEvent) {
    let this = unsafe { &*this };
    let event = unsafe { &*event };
    let local = this.convertPoint_fromView(event.locationInWindow(), None);
    if crate::clip_mac::in_hole(this, local) {
        return;
    }
    unsafe { msg_send![super(this, base_class()), cursorUpdate: event] }
}

fn make_class(base: &'static AnyClass) -> &'static AnyClass {
    let name = std::ffi::CString::new(format!("SericaTabView_{}", base.name().to_string_lossy())).expect("class name");
    if let Some(existing) = AnyClass::get(&name) {
        return existing;
    }
    let mut b = ClassBuilder::new(&name, base).expect("tab view subclass");
    unsafe {
        b.add_method(sel!(performKeyEquivalent:), perform_key_equivalent as extern "C-unwind" fn(*mut NSView, Sel, *mut NSEvent) -> Bool);
        b.add_method(sel!(keyDown:), key_down as extern "C-unwind" fn(*mut NSView, Sel, *mut NSEvent));
        b.add_method(sel!(willOpenMenu:withEvent:), will_open_menu as extern "C-unwind" fn(*mut NSView, Sel, *mut NSMenu, *mut NSEvent));
        b.add_method(sel!(hitTest:), hit_test as extern "C-unwind" fn(*mut NSView, Sel, NSPoint) -> *mut NSView);
        b.add_method(sel!(mouseMoved:), mouse_moved as extern "C-unwind" fn(*mut NSView, Sel, *mut NSEvent));
        b.add_method(sel!(cursorUpdate:), cursor_update as extern "C-unwind" fn(*mut NSView, Sel, *mut NSEvent));
    }
    b.register()
}

/// Turns a tab's WKWebView into Serica's tab view. Must run before any
/// key-value observer is added (KVO subclasses the instance itself).
pub fn adopt(wk: &WKWebView) {
    // macOS 26 AppKit adds KVO/Touch Bar observations to WKWebView during
    // display-cycle updates. Replacing the object's runtime ISA beforehand
    // corrupts that observation path and causes a native SIGSEGV. Keep
    // Serica's WKWebView on WebKit's original class until this hook has a
    // safe implementation that does not mutate NSObject ISA at runtime.
    let _ = wk;
}
