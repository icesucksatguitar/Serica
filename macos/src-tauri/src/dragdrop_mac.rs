//! Drag and drop onto Serica's own interface (sidebar, tab strip, new-tab
//! page) and out of it (finished downloads). Pages handle drops themselves.
//!
//! The main UI view takes files, links and text dragged in from Finder,
//! other apps or a page, and reports the drag to the UI (`serica:drag`
//! events, positions in points of the window's content, top-left origin,
//! with its width so the UI can convert to CSS px), which shows where it
//! will land and decides what the drop does. Drags that start inside the UI
//! itself (HTML drag and drop) stay with WebKit.

use crate::app::{ui_emit, Shared};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, NSObject, ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, ClassType, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSDragOperation, NSDraggingContext, NSDraggingInfo, NSDraggingItem, NSDraggingSession, NSDraggingSource, NSEvent, NSEventType,
    NSPasteboard, NSPasteboardTypeString, NSPasteboardTypeURL, NSPasteboardWriting, NSView, NSWorkspace,
};
use objc2_foundation::{NSArray, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL};
use std::sync::OnceLock;

/// What is being dragged.
#[derive(Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DragData {
    /// "files" | "url" | "text"
    pub kind: &'static str,
    pub paths: Vec<String>,
    pub url: Option<String>,
    pub text: Option<String>,
}

fn read(pb: &NSPasteboard) -> Option<DragData> {
    unsafe {
        // Files from Finder (and anything else that offers file URLs).
        let classes = NSArray::from_slice(&[NSURL::class()]);
        let mut paths = Vec::new();
        let mut web: Option<String> = None;
        if let Some(items) = pb.readObjectsForClasses_options(&classes, None) {
            for item in items.iter() {
                let Some(u) = item.downcast_ref::<NSURL>() else { continue };
                if u.isFileURL() {
                    if let Some(p) = u.filePathURL().and_then(|f| f.path()) {
                        paths.push(p.to_string());
                    }
                } else if web.is_none() {
                    web = u.absoluteString().map(|s| s.to_string());
                }
            }
        }
        if !paths.is_empty() {
            paths.truncate(50);
            return Some(DragData { kind: "files", paths, ..Default::default() });
        }
        // A link (from a page, or a browser's address bar).
        let url = web.or_else(|| pb.stringForType(NSPasteboardTypeURL).map(|s| s.to_string())).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        if let Some(url) = url {
            return Some(DragData { kind: "url", url: Some(url), ..Default::default() });
        }
        // Plain text: a search, or a URL written out.
        let text: String = pb.stringForType(NSPasteboardTypeString).map(|s| s.to_string()).unwrap_or_default().trim().chars().take(2000).collect();
        if !text.is_empty() {
            let looks_url = !text.contains(char::is_whitespace) && (text.starts_with("http://") || text.starts_with("https://"));
            return Some(if looks_url {
                DragData { kind: "url", url: Some(text), ..Default::default() }
            } else {
                DragData { kind: "text", text: Some(text), ..Default::default() }
            });
        }
        None
    }
}

static MAIN_CLASS: OnceLock<(usize, usize)> = OnceLock::new();
/// What `-class` answered before Serica's subclass went on the view.
static REPORTED_CLASS: OnceLock<usize> = OnceLock::new();

/// `-class` of the main view. Key-value observing had already given the view
/// a hidden subclass, whose `-class` answers the class *below* it, looked up
/// by the object's current class: under Serica's subclass that lookup finds
/// nothing and `-class` would answer nil (so `isKindOfClass:` would fail for
/// every class, `NSView` included). Answer what it answered before.
extern "C-unwind" fn reported_class(_this: *mut NSView, _sel: Sel) -> *const AnyClass {
    REPORTED_CLASS.get().map_or(std::ptr::null(), |c| *c as *const AnyClass)
}
static APP: OnceLock<Shared> = OnceLock::new();

fn base() -> &'static AnyClass {
    let (_, b) = *MAIN_CLASS.get().expect("main view class");
    unsafe { &*(b as *const AnyClass) }
}

/// A drag that started in the UI itself (HTML drag and drop) is WebKit's.
fn is_internal(view: &NSView, info: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
    let src: Option<Retained<AnyObject>> = info.draggingSource();
    src.is_some_and(|s| {
        let p = Retained::as_ptr(&s) as usize;
        p == view as *const NSView as usize || s.downcast_ref::<NSView>().is_some_and(|v| v.isDescendantOf(view))
    })
}

fn position(view: &NSView, info: &ProtocolObject<dyn NSDraggingInfo>) -> (f64, f64, f64) {
    let loc = info.draggingLocation();
    let p = view.convertPoint_fromView(loc, None);
    let b = view.bounds();
    let y = if view.isFlipped() { p.y } else { b.size.height - p.y };
    (p.x, y, b.size.width)
}

fn operation(info: &ProtocolObject<dyn NSDraggingInfo>, data: &DragData) -> NSDragOperation {
    let allowed = info.draggingSourceOperationMask();
    let want = if data.kind == "files" { NSDragOperation::Copy } else { NSDragOperation::Copy | NSDragOperation::Link };
    let both = allowed & want;
    if both.contains(NSDragOperation::Copy) {
        NSDragOperation::Copy
    } else if both.contains(NSDragOperation::Link) {
        NSDragOperation::Link
    } else if allowed.contains(NSDragOperation::Generic) {
        NSDragOperation::Generic
    } else {
        NSDragOperation::None
    }
}

fn emit(phase: &str, view: &NSView, info: Option<&ProtocolObject<dyn NSDraggingInfo>>, data: Option<&DragData>) {
    let Some(app) = APP.get() else { return };
    let (x, y, w) = info.map(|i| position(view, i)).unwrap_or((0.0, 0.0, 0.0));
    ui_emit(&app.handle, "serica:drag", serde_json::json!({ "phase": phase, "x": x, "y": y, "clientWidth": w, "data": data }));
}

thread_local! {
    static CURRENT: std::cell::RefCell<Option<DragData>> = const { std::cell::RefCell::new(None) };
}

extern "C-unwind" fn dragging_entered(this: *mut NSView, _sel: Sel, info: *mut ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
    let this = unsafe { &*this };
    let info = unsafe { &*info };
    if is_internal(this, info) {
        return unsafe { msg_send![super(this, base()), draggingEntered: info] };
    }
    let pasteboard = info.draggingPasteboard();
    let data = read(&pasteboard);
    let op = data.as_ref().map(|d| operation(info, d)).unwrap_or(NSDragOperation::None);
    if let Some(d) = &data {
        emit("enter", this, Some(info), Some(d));
    }
    CURRENT.with(|c| *c.borrow_mut() = data);
    op
}

extern "C-unwind" fn dragging_updated(this: *mut NSView, _sel: Sel, info: *mut ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
    let this = unsafe { &*this };
    let info = unsafe { &*info };
    if is_internal(this, info) {
        return unsafe { msg_send![super(this, base()), draggingUpdated: info] };
    }
    CURRENT.with(|c| match c.borrow().as_ref() {
        Some(d) => {
            emit("over", this, Some(info), Some(d));
            operation(info, d)
        }
        None => NSDragOperation::None,
    })
}

extern "C-unwind" fn dragging_exited(this: *mut NSView, _sel: Sel, info: *mut ProtocolObject<dyn NSDraggingInfo>) {
    let this = unsafe { &*this };
    let info = unsafe { info.as_ref() };
    if let Some(i) = info {
        if is_internal(this, i) {
            return unsafe { msg_send![super(this, base()), draggingExited: i] };
        }
    }
    if CURRENT.with(|c| c.borrow_mut().take()).is_some() {
        emit("leave", this, None, None);
    }
}

extern "C-unwind" fn prepare_for_drag(this: *mut NSView, _sel: Sel, info: *mut ProtocolObject<dyn NSDraggingInfo>) -> Bool {
    let this = unsafe { &*this };
    let info = unsafe { &*info };
    if is_internal(this, info) {
        return unsafe { msg_send![super(this, base()), prepareForDragOperation: info] };
    }
    Bool::new(CURRENT.with(|c| c.borrow().is_some()))
}

extern "C-unwind" fn perform_drag(this: *mut NSView, _sel: Sel, info: *mut ProtocolObject<dyn NSDraggingInfo>) -> Bool {
    let this = unsafe { &*this };
    let info = unsafe { &*info };
    if is_internal(this, info) {
        return unsafe { msg_send![super(this, base()), performDragOperation: info] };
    }
    // Read again: some sources only provide their data on drop.
    let pasteboard = info.draggingPasteboard();
    let fresh = read(&pasteboard);
    let data = fresh.or_else(|| CURRENT.with(|c| c.borrow().clone()));
    CURRENT.with(|c| *c.borrow_mut() = None);
    match data {
        Some(d) => {
            emit("drop", this, Some(info), Some(&d));
            Bool::YES
        }
        None => Bool::NO,
    }
}

extern "C-unwind" fn conclude_drag(this: *mut NSView, _sel: Sel, info: *mut ProtocolObject<dyn NSDraggingInfo>) {
    let this = unsafe { &*this };
    let info = unsafe { info.as_ref() };
    if let Some(i) = info {
        if is_internal(this, i) {
            unsafe { msg_send![super(this, base()), concludeDragOperation: i] }
        }
    }
}

/// Puts Serica's drop handling on the main UI view. Runs once, on the main
/// thread, after the view exists.
pub fn install(app: &Shared, main: &tauri::Webview) {
    // macOS 26 can crash in AppKit display-cycle KVO when a live WK/NS view
    // has had its Objective-C ISA replaced. Drag/drop hooks that depend on
    // this subclass remain disabled until a safer implementation exists.
    let _ = (app, main);
}

/// The native drag-out hook is currently disabled on macOS 26.
/// Return safely instead of installing a view subclass or KVO observer.
pub fn start_file_drag(_app: &Shared, _paths: Vec<std::path::PathBuf>) {}

/*
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct SericaMainView;

    unsafe impl NSObjectProtocol for SericaMainView {}

    unsafe impl NSDraggingSource for SericaMainView {
        /// Copy (or link); never move a download out from under the list.
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn mask(&self, _session: &NSDraggingSession, _context: NSDraggingContext) -> NSDragOperation {
            NSDragOperation::Copy | NSDragOperation::Link
        }
    }
);

thread_local! {
    static SOURCE: std::cell::RefCell<Option<Retained<SericaMainView>>> = const { std::cell::RefCell::new(None) };
}

/// Drags files out of Serica (finished downloads) as a normal Finder drag:
/// drop them in a folder or on the desktop to copy them, into a page or Mail
/// to attach them. Only while the mouse button is still held, or the drop
/// would land wherever the pointer is.
pub fn start_file_drag(app: &Shared, paths: Vec<std::path::PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let Some(main) = tauri::Manager::get_webview(&app.handle, "main") else { return };
    let _ = main.with_webview(move |pw| unsafe {
        let mtm = MainThreadMarker::new().expect("main thread");
        if NSEvent::pressedMouseButtons() & 1 == 0 {
            return;
        }
        let ptr = pw.inner() as *const NSView;
        if ptr.is_null() {
            return;
        }
        let view = &*ptr;
        let Some(window) = view.window() else { return };
        let ns_app = NSApplication::sharedApplication(mtm);
        let current = ns_app.currentEvent().filter(|e| matches!(e.r#type(), NSEventType::LeftMouseDown | NSEventType::LeftMouseDragged));
        let event = match current {
            Some(e) => e,
            None => {
                let loc = window.mouseLocationOutsideOfEventStream();
                let Some(e) = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                    NSEventType::LeftMouseDragged,
                    loc,
                    objc2_app_kit::NSEventModifierFlags::empty(),
                    0.0,
                    window.windowNumber(),
                    None,
                    0,
                    1,
                    1.0,
                ) else {
                    return;
                };
                e
            }
        };
        let at = view.convertPoint_fromView(event.locationInWindow(), None);
        let ws = NSWorkspace::sharedWorkspace();
        let mut items: Vec<Retained<NSDraggingItem>> = Vec::new();
        for (i, p) in paths.iter().enumerate() {
            let path = NSString::from_str(&p.to_string_lossy());
            let url = NSURL::fileURLWithPath(&path);
            let writer: &ProtocolObject<dyn NSPasteboardWriting> = ProtocolObject::from_ref(&*url);
            let item = NSDraggingItem::initWithPasteboardWriter(mtm.alloc(), writer);
            let icon = ws.iconForFile(&path);
            let off = i as f64 * 6.0;
            item.setDraggingFrame_contents(NSRect::new(NSPoint::new(at.x - 24.0 + off, at.y - 24.0 + off), NSSize::new(48.0, 48.0)), Some(&icon));
            items.push(item);
        }
        let source = SOURCE.with(|s| s.borrow_mut().get_or_insert_with(|| msg_send![SericaMainView::alloc(mtm), init]).clone());
        let list = NSArray::from_retained_slice(&items);
        let _ = view.beginDraggingSessionWithItems_event_source(&list, &event, ProtocolObject::from_ref(&*source));
    });
}
*/
