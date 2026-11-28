//! The window and tabs WebKit's extension runtime sees (macOS 15.4+), the
//! counterpart of what Chrome gives extensions through `chrome.tabs` and
//! `chrome.windows`: Serica's window, holding the pages of the space in use
//! (private pages are left out, as in a Chrome window that isn't
//! incognito). Extensions can list, open, switch, reload and close them, and
//! hear when they open, close, switch or change; popups and options pages
//! open in a tab.
//!
//! WebKit tells tabs apart by object identity, so each tab keeps one object
//! for as long as it is open. Everything here runs on the main thread.

use super::webkit::{controller, page_url};
use crate::app::Shared;
use crate::mac_util::{nsurl, post_main, MainBox};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, ProtocolObject};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_core_foundation::{CGRect, CGSize};
use objc2_foundation::{NSArray, NSError, NSObjectProtocol, NSString, NSURL};
use objc2_web_kit::{
    WKWebExtensionAction, WKWebExtensionContext, WKWebExtensionController, WKWebExtensionControllerDelegate, WKWebExtensionTab, WKWebExtensionTabChangedProperties,
    WKWebExtensionTabConfiguration, WKWebExtensionWindow, WKWebExtensionWindowState, WKWebExtensionWindowType, WKWebView,
};
use std::cell::RefCell;
use std::collections::HashMap;

pub struct TabIvars {
    tab_id: String,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TabIvars]
    struct SericaExtTab;

    unsafe impl NSObjectProtocol for SericaExtTab {}

    unsafe impl WKWebExtensionTab for SericaExtTab {
        #[unsafe(method_id(windowForWebExtensionContext:))]
        fn window_for(&self, _ctx: &WKWebExtensionContext) -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
            Some(window_object())
        }

        #[unsafe(method(indexInWindowForWebExtensionContext:))]
        fn index_in_window(&self, _ctx: &WKWebExtensionContext) -> usize {
            tab_ids().iter().position(|t| *t == self.ivars().tab_id).unwrap_or(usize::MAX)
        }

        #[unsafe(method_id(webViewForWebExtensionContext:))]
        fn web_view(&self, _ctx: &WKWebExtensionContext) -> Option<Retained<WKWebView>> {
            crate::webview_mac::api::tab_view(&self.ivars().tab_id)
        }

        #[unsafe(method_id(titleForWebExtensionContext:))]
        fn title(&self, _ctx: &WKWebExtensionContext) -> Option<Retained<NSString>> {
            with_tab(&self.ivars().tab_id, |t| NSString::from_str(&t.title))
        }

        #[unsafe(method(isPinnedForWebExtensionContext:))]
        fn is_pinned(&self, _ctx: &WKWebExtensionContext) -> bool {
            with_tab(&self.ivars().tab_id, |t| t.is_pinned).unwrap_or(false)
        }

        #[unsafe(method(isPlayingAudioForWebExtensionContext:))]
        fn is_playing_audio(&self, _ctx: &WKWebExtensionContext) -> bool {
            with_tab(&self.ivars().tab_id, |t| t.has_active_media).unwrap_or(false)
        }

        #[unsafe(method(sizeForWebExtensionContext:))]
        fn size(&self, _ctx: &WKWebExtensionContext) -> CGSize {
            crate::webview_mac::api::tab_view(&self.ivars().tab_id).map(|w| w.frame().size).unwrap_or(CGSize::new(0.0, 0.0))
        }

        #[unsafe(method(zoomFactorForWebExtensionContext:))]
        fn zoom_factor(&self, _ctx: &WKWebExtensionContext) -> f64 {
            with_tab(&self.ivars().tab_id, |t| t.zoom_factor.unwrap_or(1.0)).unwrap_or(1.0)
        }

        #[unsafe(method_id(urlForWebExtensionContext:))]
        fn url(&self, _ctx: &WKWebExtensionContext) -> Option<Retained<NSURL>> {
            with_tab(&self.ivars().tab_id, |t| t.url.clone()).filter(|u| !crate::model::is_internal_url(u)).and_then(|u| nsurl(&u))
        }

        #[unsafe(method(isLoadingCompleteForWebExtensionContext:))]
        fn loading_complete(&self, _ctx: &WKWebExtensionContext) -> bool {
            with_tab(&self.ivars().tab_id, |t| !t.is_loading).unwrap_or(true)
        }

        #[unsafe(method(isSelectedForWebExtensionContext:))]
        fn is_selected(&self, _ctx: &WKWebExtensionContext) -> bool {
            active_tab_id().as_deref() == Some(self.ivars().tab_id.as_str())
        }

        #[unsafe(method(loadURL:forWebExtensionContext:completionHandler:))]
        fn load_url(&self, url: &NSURL, _ctx: &WKWebExtensionContext, handler: &block2::DynBlock<dyn Fn(*mut NSError)>) {
            let target = crate::mac_util::url_string(Some(url));
            match crate::webview_mac::api::tab_view(&self.ivars().tab_id) {
                Some(wv) => crate::webview_mac::nav_load(&wv, &target),
                None => {
                    let id = self.ivars().tab_id.clone();
                    mutate(move |s| {
                        if let Some(t) = s.tab_mut(&id) {
                            t.url = target;
                        }
                    });
                }
            }
            handler.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(reloadFromOrigin:forWebExtensionContext:completionHandler:))]
        fn reload(&self, from_origin: bool, _ctx: &WKWebExtensionContext, handler: &block2::DynBlock<dyn Fn(*mut NSError)>) {
            if let Some(wv) = crate::webview_mac::api::tab_view(&self.ivars().tab_id) {
                unsafe {
                    if from_origin {
                        wv.reloadFromOrigin();
                    } else {
                        wv.reload();
                    }
                }
            }
            handler.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(goBackForWebExtensionContext:completionHandler:))]
        fn go_back(&self, _ctx: &WKWebExtensionContext, handler: &block2::DynBlock<dyn Fn(*mut NSError)>) {
            if let Some(wv) = crate::webview_mac::api::tab_view(&self.ivars().tab_id) {
                unsafe { wv.goBack() };
            }
            handler.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(goForwardForWebExtensionContext:completionHandler:))]
        fn go_forward(&self, _ctx: &WKWebExtensionContext, handler: &block2::DynBlock<dyn Fn(*mut NSError)>) {
            if let Some(wv) = crate::webview_mac::api::tab_view(&self.ivars().tab_id) {
                unsafe { wv.goForward() };
            }
            handler.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(activateForWebExtensionContext:completionHandler:))]
        fn activate(&self, _ctx: &WKWebExtensionContext, handler: &block2::DynBlock<dyn Fn(*mut NSError)>) {
            let id = self.ivars().tab_id.clone();
            mutate(move |s| {
                s.activate_tab(&id);
            });
            handler.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(closeForWebExtensionContext:completionHandler:))]
        fn close(&self, _ctx: &WKWebExtensionContext, handler: &block2::DynBlock<dyn Fn(*mut NSError)>) {
            let id = self.ivars().tab_id.clone();
            mutate(move |s| {
                s.close_tab(&id);
            });
            handler.call((std::ptr::null_mut(),));
        }
    }
);

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct SericaExtWindow;

    unsafe impl NSObjectProtocol for SericaExtWindow {}

    unsafe impl WKWebExtensionWindow for SericaExtWindow {
        #[unsafe(method_id(tabsForWebExtensionContext:))]
        fn tabs(&self, _ctx: &WKWebExtensionContext) -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionTab>>> {
            let objs: Vec<Retained<ProtocolObject<dyn WKWebExtensionTab>>> = tab_ids().iter().map(|id| tab_object(id)).collect();
            NSArray::from_retained_slice(&objs)
        }

        #[unsafe(method_id(activeTabForWebExtensionContext:))]
        fn active_tab(&self, _ctx: &WKWebExtensionContext) -> Option<Retained<ProtocolObject<dyn WKWebExtensionTab>>> {
            active_tab_id().filter(|id| tab_ids().contains(id)).map(|id| tab_object(&id))
        }

        #[unsafe(method(windowTypeForWebExtensionContext:))]
        fn window_type(&self, _ctx: &WKWebExtensionContext) -> WKWebExtensionWindowType {
            WKWebExtensionWindowType::Normal
        }

        #[unsafe(method(windowStateForWebExtensionContext:))]
        fn window_state(&self, _ctx: &WKWebExtensionContext) -> WKWebExtensionWindowState {
            match ns_window() {
                Some(w) if w.styleMask().contains(objc2_app_kit::NSWindowStyleMask::FullScreen) => WKWebExtensionWindowState::Fullscreen,
                Some(w) if w.isMiniaturized() => WKWebExtensionWindowState::Minimized,
                Some(w) if w.isZoomed() => WKWebExtensionWindowState::Maximized,
                _ => WKWebExtensionWindowState::Normal,
            }
        }

        #[unsafe(method(isPrivateForWebExtensionContext:))]
        fn is_private(&self, _ctx: &WKWebExtensionContext) -> bool {
            false
        }

        #[unsafe(method(frameForWebExtensionContext:))]
        fn frame(&self, _ctx: &WKWebExtensionContext) -> CGRect {
            ns_window().map(|w| w.frame()).unwrap_or(CGRect::ZERO)
        }

        #[unsafe(method(screenFrameForWebExtensionContext:))]
        fn screen_frame(&self, _ctx: &WKWebExtensionContext) -> CGRect {
            ns_window().and_then(|w| w.screen()).map(|s| s.frame()).unwrap_or(CGRect::ZERO)
        }

        #[unsafe(method(focusForWebExtensionContext:completionHandler:))]
        fn focus(&self, _ctx: &WKWebExtensionContext, handler: &block2::DynBlock<dyn Fn(*mut NSError)>) {
            if let Some(w) = ns_window() {
                let mtm = MainThreadMarker::new().expect("main thread");
                objc2_app_kit::NSApplication::sharedApplication(mtm).activate();
                w.makeKeyAndOrderFront(None);
            }
            handler.call((std::ptr::null_mut(),));
        }
    }
);

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct SericaExtDelegate;

    unsafe impl NSObjectProtocol for SericaExtDelegate {}

    unsafe impl WKWebExtensionControllerDelegate for SericaExtDelegate {
        #[unsafe(method_id(webExtensionController:openWindowsForExtensionContext:))]
        fn open_windows(&self, _c: &WKWebExtensionController, _ctx: &WKWebExtensionContext) -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionWindow>>> {
            NSArray::from_retained_slice(&[window_object()])
        }

        #[unsafe(method_id(webExtensionController:focusedWindowForExtensionContext:))]
        fn focused_window(&self, _c: &WKWebExtensionController, _ctx: &WKWebExtensionContext) -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
            Some(window_object())
        }

        #[unsafe(method(webExtensionController:openNewTabUsingConfiguration:forExtensionContext:completionHandler:))]
        fn open_new_tab(
            &self,
            _c: &WKWebExtensionController,
            config: &WKWebExtensionTabConfiguration,
            _ctx: &WKWebExtensionContext,
            handler: &block2::DynBlock<dyn Fn(*mut ProtocolObject<dyn WKWebExtensionTab>, *mut NSError)>,
        ) {
            let (url, active) = unsafe { (config.url().map(|u| crate::mac_util::url_string(Some(&u))), config.shouldBeActive()) };
            open_tab(url, active, Some(MainBox(handler.copy())));
        }

        #[unsafe(method(webExtensionController:openOptionsPageForExtensionContext:completionHandler:))]
        fn open_options(&self, _c: &WKWebExtensionController, ctx: &WKWebExtensionContext, handler: &block2::DynBlock<dyn Fn(*mut NSError)>) {
            open_page(ctx, "options");
            handler.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(webExtensionController:presentPopupForAction:forExtensionContext:completionHandler:))]
        fn present_popup(&self, _c: &WKWebExtensionController, _action: &WKWebExtensionAction, ctx: &WKWebExtensionContext, handler: &block2::DynBlock<dyn Fn(*mut NSError)>) {
            // Serica has no toolbar button per extension: the popup opens in a
            // tab, as from the Extensions panel.
            open_page(ctx, "popup");
            handler.call((std::ptr::null_mut(),));
        }
    }
);

type NewTabDone = MainBox<block2::RcBlock<dyn Fn(*mut ProtocolObject<dyn WKWebExtensionTab>, *mut NSError)>>;

thread_local! {
    static WINDOW: RefCell<Option<Retained<SericaExtWindow>>> = const { RefCell::new(None) };
    static TABS: RefCell<HashMap<String, Retained<SericaExtTab>>> = RefCell::new(HashMap::new());
    static DELEGATE: RefCell<Option<Retained<SericaExtDelegate>>> = const { RefCell::new(None) };
    static ACTIVE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Hands the controller Serica's window and tabs. Main thread, once.
pub fn install(controller: &WKWebExtensionController) {
    let mtm = MainThreadMarker::new().expect("main thread");
    let delegate: Retained<SericaExtDelegate> = unsafe { msg_send![SericaExtDelegate::alloc(mtm), init] };
    unsafe { controller.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
    DELEGATE.with(|d| *d.borrow_mut() = Some(delegate));
    unsafe { controller.didOpenWindow(&window_object()) };
}

fn app() -> Option<Shared> {
    crate::app::global().cloned()
}

fn ns_window() -> Option<Retained<objc2_app_kit::NSWindow>> {
    use tauri::Manager;
    let w = app()?.handle.get_window("main")?;
    let ptr = w.ns_window().ok()? as *const objc2_app_kit::NSWindow;
    // SAFETY: the live NSWindow, on the main thread.
    (!ptr.is_null()).then(|| unsafe { objc2::Message::retain(&*ptr) })
}

/// The pages of the space in use, in order (no private ones).
fn tab_ids() -> Vec<String> {
    let Some(app) = app() else { return Vec::new() };
    let s = app.store.lock();
    let space = s.active_space_id.clone();
    s.tabs.iter().filter(|t| t.space_id == space && !t.private).map(|t| t.id.clone()).collect()
}

fn active_tab_id() -> Option<String> {
    app()?.store.lock().active_tab_id.clone()
}

fn with_tab<T>(id: &str, f: impl FnOnce(&crate::model::TabState) -> T) -> Option<T> {
    let app = app()?;
    let s = app.store.lock();
    s.tab(id).map(f)
}

/// Changes the state off the main thread (a change can wait on it).
fn mutate(f: impl FnOnce(&mut crate::store::Store) + Send + 'static) {
    let Some(app) = app() else { return };
    tauri::async_runtime::spawn_blocking(move || app.mutate(f));
}

fn window_object() -> Retained<ProtocolObject<dyn WKWebExtensionWindow>> {
    let w = WINDOW.with(|w| {
        w.borrow_mut()
            .get_or_insert_with(|| {
                let mtm = MainThreadMarker::new().expect("main thread");
                unsafe { msg_send![SericaExtWindow::alloc(mtm), init] }
            })
            .clone()
    });
    ProtocolObject::from_retained(w)
}

fn tab_object(id: &str) -> Retained<ProtocolObject<dyn WKWebExtensionTab>> {
    let t = TABS.with(|t| {
        t.borrow_mut()
            .entry(id.to_string())
            .or_insert_with(|| {
                let mtm = MainThreadMarker::new().expect("main thread");
                let this = SericaExtTab::alloc(mtm).set_ivars(TabIvars { tab_id: id.to_string() });
                unsafe { msg_send![super(this), init] }
            })
            .clone()
    });
    ProtocolObject::from_retained(t)
}

/// Opens a tab for an extension; `done` gets its tab once it exists.
fn open_tab(url: Option<String>, active: bool, done: Option<NewTabDone>) {
    let Some(app) = app() else { return };
    let handle = app.handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let tab = app.mutate(move |s| s.create_tab(url, None, active, None));
        post_main(&handle, move || {
            if let Some(done) = done {
                let obj = tab_object(&tab.id);
                done.0.call((Retained::as_ptr(&obj) as *mut _, std::ptr::null_mut()));
            }
        });
    });
}

/// The extension's popup or options page, in a tab.
fn open_page(ctx: &WKWebExtensionContext, page: &'static str) {
    let Some(app) = app() else { return };
    let id = unsafe { ctx.uniqueIdentifier() }.to_string();
    let Some(rec) = app.extensions.get(&id) else { return };
    tauri::async_runtime::spawn(async move {
        if let Some(url) = page_url(&app, &rec, page).await {
            open_tab(Some(url), true, None);
        }
    });
}

// ------------------------------------------------------------------ what happens to tabs

fn with_controller(f: impl FnOnce(&WKWebExtensionController)) {
    if let Some(c) = controller() {
        f(&c);
    }
}

/// A page of a tab opened (its web view exists).
pub fn tab_opened(tab_id: &str, private: bool) {
    if private {
        return;
    }
    with_controller(|c| unsafe { c.didOpenTab(&tab_object(tab_id)) });
}

/// A tab's page closed.
pub fn tab_closed(tab_id: &str) {
    let Some(obj) = TABS.with(|t| t.borrow_mut().remove(tab_id)) else { return };
    ACTIVE.with(|a| {
        if a.borrow().as_deref() == Some(tab_id) {
            *a.borrow_mut() = None;
        }
    });
    with_controller(|c| unsafe { c.didCloseTab_windowIsClosing(&ProtocolObject::from_retained(obj), false) });
}

/// The tab on screen changed (from any thread).
pub fn tab_activated(app: &Shared, tab_id: &str) {
    let id = tab_id.to_string();
    post_main(&app.handle, move || {
        if ACTIVE.with(|a| a.borrow().as_deref() == Some(id.as_str())) || !tab_ids().contains(&id) {
            return;
        }
        let previous = ACTIVE.with(|a| a.borrow_mut().replace(id.clone()));
        with_controller(|c| unsafe {
            let prev = previous.as_deref().and_then(|p| TABS.with(|t| t.borrow().get(p).cloned())).map(ProtocolObject::from_retained);
            c.didActivateTab_previousActiveTab(&tab_object(&id), prev.as_deref());
        });
    });
}

/// A tab's address, title or loading state changed (main thread).
pub fn tab_changed(tab_id: &str, props: WKWebExtensionTabChangedProperties) {
    if !TABS.with(|t| t.borrow().contains_key(tab_id)) {
        return;
    }
    with_controller(|c| unsafe { c.didChangeTabProperties_forTab(props, &tab_object(tab_id)) });
}
