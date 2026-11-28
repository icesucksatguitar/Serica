//! Real popup windows for pages: sign-in (Google, Apple, Microsoft…),
//! payment (PayPal, 3-D Secure) and other windows a page opens and keeps
//! talking to through `window.opener`.
//!
//! WebKit hands Serica the configuration the popup must be built on (it ties
//! the popup to its opener: same website data, same Shields rule lists, same
//! process pool), and Serica gives it a window of its own with a UI delegate,
//! so that `window.close()` closes it (a sign-in popup closes itself once it
//! has passed the result back), `alert` / `confirm` / `prompt` work, and a
//! popup can open a popup of its own.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSBackingStoreType, NSScreen, NSTextField, NSWindow, NSWindowDelegate, NSWindowStyleMask};
use objc2_foundation::{NSNotification, NSPoint, NSRect, NSSize, NSString};
use objc2_web_kit::{WKFrameInfo, WKNavigationAction, WKNavigationType, WKUIDelegate, WKWebView, WKWebViewConfiguration, WKWindowFeatures};
use std::cell::RefCell;

/// Size of a popup that asked for none (a typical sign-in window).
const DEFAULT_W: f64 = 520.0;
const DEFAULT_H: f64 = 680.0;

struct Popup {
    window: Retained<NSWindow>,
    webview: Retained<WKWebView>,
    _ui: Retained<PopupUIDelegate>,
    _win: Retained<PopupWindowDelegate>,
}

thread_local! {
    /// Open popups (kept alive until their window closes).
    static POPUPS: RefCell<Vec<Popup>> = const { RefCell::new(Vec::new()) };
    /// Keeps popup titles on their pages' titles while any popup is open.
    static TITLE_TIMER: RefCell<Option<Retained<objc2_foundation::NSTimer>>> = const { RefCell::new(None) };
}

fn start_title_timer() {
    TITLE_TIMER.with(|t| {
        if t.borrow().is_some() {
            return;
        }
        let block = block2::RcBlock::new(|_: std::ptr::NonNull<objc2_foundation::NSTimer>| sync_titles());
        let timer = unsafe { objc2_foundation::NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.5, true, &block) };
        *t.borrow_mut() = Some(timer);
    });
}

fn stop_title_timer() {
    if let Some(t) = TITLE_TIMER.with(|t| t.borrow_mut().take()) {
        t.invalidate();
    }
}

/// Whether a page's `window.open` / `target=_blank` should get a real popup
/// rather than a new tab: the page asked for a size (sign-in and payment
/// windows), or opened a blank window it fills in or steers itself (without
/// a real window, `window.open('')` then `w.location = …` does nothing).
/// Links the user clicks open as tabs.
pub fn wants_popup(action: &WKNavigationAction, features: &WKWindowFeatures) -> bool {
    let sized = unsafe { features.width().is_some() && features.height().is_some() };
    if sized {
        return true;
    }
    let url = unsafe { action.request().URL() }.and_then(|u| u.absoluteString()).map(|s| s.to_string()).unwrap_or_default();
    let blank = url.is_empty() || url == "about:blank";
    let link = unsafe { action.navigationType() } == WKNavigationType::LinkActivated;
    blank && !link
}

/// Builds the popup on WebKit's `config` and shows it near its opener.
pub fn open(opener: &WKWebView, config: &WKWebViewConfiguration, features: &WKWindowFeatures) -> Option<Retained<WKWebView>> {
    let mtm = MainThreadMarker::new()?;
    let num = |n: Option<Retained<objc2_foundation::NSNumber>>| n.map(|n| n.doubleValue());
    let (w, h) = unsafe { (num(features.width()).unwrap_or(DEFAULT_W), num(features.height()).unwrap_or(DEFAULT_H)) };
    let (w, h) = (w.clamp(240.0, 2400.0), h.clamp(200.0, 1600.0));

    // Centred over the opener's window, kept on its screen.
    let parent = opener.window();
    let screen = parent.as_ref().and_then(|p| p.screen()).or_else(|| NSScreen::mainScreen(mtm));
    let visible = screen.map(|s| s.visibleFrame()).unwrap_or(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0)));
    let anchor = parent.as_ref().map(|p| p.frame()).unwrap_or(visible);
    let mut x = anchor.origin.x + (anchor.size.width - w) / 2.0;
    let mut y = anchor.origin.y + (anchor.size.height - h) / 2.0;
    x = x.clamp(visible.origin.x, (visible.origin.x + visible.size.width - w).max(visible.origin.x));
    y = y.clamp(visible.origin.y, (visible.origin.y + visible.size.height - h).max(visible.origin.y));
    let rect = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));

    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable | NSWindowStyleMask::Resizable;
    let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(mtm.alloc(), rect, style, NSBackingStoreType::Buffered, false) };
    // Released by POPUPS, not by AppKit on close.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str(""));

    let content = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h));
    let webview = unsafe { WKWebView::initWithFrame_configuration(mtm.alloc(), content, config) };
    let ui = PopupUIDelegate::new(mtm);
    unsafe {
        webview.setUIDelegate(Some(ProtocolObject::from_ref(&*ui)));
        if crate::mac_util::responds(&*webview, objc2::sel!(setInspectable:)) {
            webview.setInspectable(true);
        }
        // Same browser identity as the opener (sign-in pages check it).
        if let Some(ua) = opener.customUserAgent() {
            webview.setCustomUserAgent(Some(&ua));
        }
    }
    window.setContentView(Some(&webview));
    let win = PopupWindowDelegate::new(mtm);
    window.setDelegate(Some(ProtocolObject::from_ref(&*win)));
    window.makeKeyAndOrderFront(None);

    POPUPS.with(|p| p.borrow_mut().push(Popup { window, webview: webview.clone(), _ui: ui, _win: win }));
    start_title_timer();
    Some(webview)
}

/// Closes the popup showing `wv` (the page called `window.close()`).
fn close_for(wv: &WKWebView) {
    let window = POPUPS.with(|p| p.borrow().iter().find(|x| std::ptr::eq(&*x.webview, wv)).map(|x| x.window.clone()));
    if let Some(w) = window {
        w.close();
    }
}

/// A popup's window closed: drop it (its page goes with it).
fn forget(window: &NSWindow) {
    let gone = POPUPS.with(|p| {
        let mut p = p.borrow_mut();
        p.iter().position(|x| std::ptr::eq(&*x.window, window)).map(|i| p.remove(i))
    });
    if POPUPS.with(|p| p.borrow().is_empty()) {
        stop_title_timer();
    }
    if let Some(p) = gone {
        unsafe {
            p.webview.stopLoading();
            p.webview.setUIDelegate(None);
        }
        // Released once this call has returned (AppKit is still using the window).
        let slot = RefCell::new(Some(p));
        let block = block2::RcBlock::new(move |_: std::ptr::NonNull<objc2_foundation::NSTimer>| drop(slot.borrow_mut().take()));
        unsafe { objc2_foundation::NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.0, false, &block) };
    }
}

fn host_of(frame: &WKFrameInfo) -> String {
    let origin = unsafe { frame.securityOrigin() };
    unsafe { origin.host() }.to_string()
}

/// An `alert` / `confirm` / `prompt` from a popup, as a sheet on its window.
fn dialog(wv: &WKWebView, frame: &WKFrameInfo, message: &NSString, buttons: &[&str], field: Option<&NSString>, done: impl FnOnce(bool, Option<String>) + 'static) {
    let Some(mtm) = MainThreadMarker::new() else { return done(false, None) };
    let alert = NSAlert::new(mtm);
    let host = host_of(frame);
    alert.setMessageText(&NSString::from_str(if host.is_empty() { "This page says" } else { &host }));
    alert.setInformativeText(message);
    for b in buttons {
        alert.addButtonWithTitle(&NSString::from_str(b));
    }
    let input = field.map(|default| {
        let f = NSTextField::initWithFrame(mtm.alloc(), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(260.0, 24.0)));
        f.setStringValue(default);
        alert.setAccessoryView(Some(&f));
        f
    });
    let Some(window) = wv.window() else { return done(false, None) };
    let done = RefCell::new(Some(done));
    let handler = block2::RcBlock::new(move |code: isize| {
        let ok = code == NSAlertFirstButtonReturn;
        let text = input.as_ref().map(|f| f.stringValue().to_string());
        if let Some(d) = done.borrow_mut().take() {
            d(ok, text);
        }
    });
    alert.beginSheetModalForWindow_completionHandler(&window, Some(&handler));
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct PopupUIDelegate;

    unsafe impl NSObjectProtocol for PopupUIDelegate {}

    unsafe impl WKUIDelegate for PopupUIDelegate {
        /// A popup opening a popup of its own.
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(&self, wv: &WKWebView, config: &WKWebViewConfiguration, _action: &WKNavigationAction, features: &WKWindowFeatures) -> Option<Retained<WKWebView>> {
            open(wv, config, features)
        }

        #[unsafe(method(webViewDidClose:))]
        fn did_close(&self, wv: &WKWebView) {
            close_for(wv);
        }

        #[unsafe(method(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn alert(&self, wv: &WKWebView, message: &NSString, frame: &WKFrameInfo, handler: &block2::Block<dyn Fn()>) {
            let h = handler.copy();
            dialog(wv, frame, message, &["OK"], None, move |_, _| h.call(()));
        }

        #[unsafe(method(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn confirm(&self, wv: &WKWebView, message: &NSString, frame: &WKFrameInfo, handler: &block2::Block<dyn Fn(Bool)>) {
            let h = handler.copy();
            dialog(wv, frame, message, &["OK", "Cancel"], None, move |ok, _| h.call((Bool::new(ok),)));
        }

        #[unsafe(method(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:))]
        fn prompt(&self, wv: &WKWebView, prompt: &NSString, default_text: Option<&NSString>, frame: &WKFrameInfo, handler: &block2::Block<dyn Fn(*mut NSString)>) {
            let h = handler.copy();
            let empty = NSString::from_str("");
            dialog(wv, frame, prompt, &["OK", "Cancel"], Some(default_text.unwrap_or(&empty)), move |ok, text| match (ok, text) {
                (true, Some(t)) => {
                    let s = NSString::from_str(&t);
                    h.call((Retained::as_ptr(&s) as *mut NSString,));
                }
                _ => h.call((std::ptr::null_mut(),)),
            });
        }
    }
);

impl PopupUIDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct PopupWindowDelegate;

    unsafe impl NSObjectProtocol for PopupWindowDelegate {}

    unsafe impl NSWindowDelegate for PopupWindowDelegate {
        #[unsafe(method(windowWillClose:))]
        fn will_close(&self, notification: &NSNotification) {
            let obj: Option<Retained<AnyObject>> = notification.object();
            if let Some(w) = obj.and_then(|o| o.downcast::<NSWindow>().ok()) {
                forget(&w);
            }
        }
    }
);

impl PopupWindowDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

/// Keeps each popup's window title on its page's title.
fn sync_titles() {
    POPUPS.with(|p| {
        for x in p.borrow().iter() {
            let title = unsafe { x.webview.title() }.map(|t| t.to_string()).unwrap_or_default();
            let title = if title.is_empty() { unsafe { x.webview.URL() }.and_then(|u| u.host()).map(|h| h.to_string()).unwrap_or_default() } else { title };
            if x.window.title().to_string() != title {
                x.window.setTitle(&NSString::from_str(&title));
            }
        }
    });
}
