//! The Objective-C classes Serica puts on each tab's WKWebView: navigation
//! and UI delegates (wrapping wry's), the `serica` script message handler,
//! and the key-value observer for the view's URL, title, history, audio,
//! fullscreen and theme color.

use super::*;
use std::ptr::NonNull;

// ------------------------------------------------------------------ navigation delegate

pub struct NavIvars {
    pub inner: Option<Retained<ProtocolObject<dyn WKNavigationDelegate>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = NavIvars]
    pub struct SericaNavDelegate;

    unsafe impl NSObjectProtocol for SericaNavDelegate {}

    unsafe impl WKNavigationDelegate for SericaNavDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:preferences:decisionHandler:))]
        fn decide_action(
            &self,
            wv: &WKWebView,
            action: &WKNavigationAction,
            prefs: &WKWebpagePreferences,
            handler: &block2::Block<dyn Fn(WKNavigationActionPolicy, NonNull<WKWebpagePreferences>)>,
        ) {
            let _perf = crate::perf::span("wk.decidePolicyForNavigationAction");
            super::nav::decide_action(self, wv, action, prefs, handler);
        }

        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn decide_response(&self, wv: &WKWebView, response: &WKNavigationResponse, handler: &block2::Block<dyn Fn(WKNavigationResponsePolicy)>) {
            super::nav::decide_response(wv, response, handler);
        }

        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn did_start(&self, wv: &WKWebView, _navigation: Option<&WKNavigation>) {
            let _perf = crate::perf::span("wk.didStartProvisionalNavigation");
            super::nav::did_start(wv, false);
        }

        #[unsafe(method(webView:didReceiveServerRedirectForProvisionalNavigation:))]
        fn did_redirect(&self, wv: &WKWebView, _navigation: Option<&WKNavigation>) {
            super::nav::did_start(wv, true);
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn did_fail_provisional(&self, wv: &WKWebView, navigation: Option<&WKNavigation>, error: &NSError) {
            self.forward_fail(wv, navigation, error, true);
            super::nav::did_fail(wv, error, true);
        }

        #[unsafe(method(webView:didCommitNavigation:))]
        fn did_commit(&self, wv: &WKWebView, navigation: Option<&WKNavigation>) {
            let _perf = crate::perf::span("wk.didCommitNavigation");
            // Wry runs the scripts queued before the first commit here, and
            // Tauri reports the page load as started.
            if let Some(inner) = &self.ivars().inner {
                if responds(&**inner, sel!(webView:didCommitNavigation:)) {
                    unsafe { let _: () = msg_send![&**inner, webView: wv, didCommitNavigation: navigation]; }
                }
            }
            super::nav::did_commit(wv);
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn did_finish(&self, wv: &WKWebView, navigation: Option<&WKNavigation>) {
            let _perf = crate::perf::span("wk.didFinishNavigation");
            if let Some(inner) = &self.ivars().inner {
                if responds(&**inner, sel!(webView:didFinishNavigation:)) {
                    unsafe { let _: () = msg_send![&**inner, webView: wv, didFinishNavigation: navigation]; }
                }
            }
            super::nav::did_finish(wv);
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn did_fail_navigation(&self, wv: &WKWebView, navigation: Option<&WKNavigation>, error: &NSError) {
            self.forward_fail(wv, navigation, error, false);
            super::nav::did_fail(wv, error, false);
        }

        #[unsafe(method(webView:didReceiveAuthenticationChallenge:completionHandler:))]
        fn auth_challenge(
            &self,
            wv: &WKWebView,
            challenge: &NSURLAuthenticationChallenge,
            handler: &block2::Block<dyn Fn(NSURLSessionAuthChallengeDisposition, *mut NSURLCredential)>,
        ) {
            super::nav::auth_challenge(wv, challenge, handler);
        }

        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn process_terminated(&self, wv: &WKWebView) {
            if let Some(inner) = &self.ivars().inner {
                if responds(&**inner, sel!(webViewWebContentProcessDidTerminate:)) {
                    unsafe { let _: () = msg_send![&**inner, webViewWebContentProcessDidTerminate: wv]; }
                }
            }
            super::nav::process_terminated(wv);
        }

        #[unsafe(method(webView:navigationAction:didBecomeDownload:))]
        fn action_download(&self, wv: &WKWebView, _action: &WKNavigationAction, download: &WKDownload) {
            super::nav::adopt_download(wv, download);
        }

        #[unsafe(method(webView:navigationResponse:didBecomeDownload:))]
        fn response_download(&self, wv: &WKWebView, _response: &WKNavigationResponse, download: &WKDownload) {
            super::nav::adopt_download(wv, download);
        }
    }

    /// WebKit's private navigation delegate method: a content rule list
    /// acted on a request (Shields' live counter and log).
    impl SericaNavDelegate {
        #[unsafe(method(_webView:contentRuleListWithIdentifier:performedAction:forURL:))]
        fn rule_list_action(&self, wv: &WKWebView, identifier: &NSString, action: &AnyObject, url: &NSURL) {
            super::nav::rule_list_action(wv, identifier, action, url);
        }
    }
);

impl SericaNavDelegate {
    pub fn new(mtm: MainThreadMarker, inner: Option<Retained<ProtocolObject<dyn WKNavigationDelegate>>>) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(NavIvars { inner });
        unsafe { msg_send![super(this), init] }
    }

    fn forward_fail(&self, wv: &WKWebView, navigation: Option<&WKNavigation>, error: &NSError, provisional: bool) {
        let Some(inner) = &self.ivars().inner else { return };
        unsafe {
            if provisional && responds(&**inner, sel!(webView:didFailProvisionalNavigation:withError:)) {
                let _: () = msg_send![&**inner, webView: wv, didFailProvisionalNavigation: navigation, withError: error];
            } else if !provisional && responds(&**inner, sel!(webView:didFailNavigation:withError:)) {
                let _: () = msg_send![&**inner, webView: wv, didFailNavigation: navigation, withError: error];
            }
        }
    }

    /// Lets wry (and Tauri's navigation hooks) have the final say on a
    /// navigation Serica allows; `handler` keeps the given preferences.
    pub fn forward_decide(
        &self,
        wv: &WKWebView,
        action: &WKNavigationAction,
        prefs: &WKWebpagePreferences,
        handler: &block2::Block<dyn Fn(WKNavigationActionPolicy, NonNull<WKWebpagePreferences>)>,
    ) {
        let prefs_ptr = NonNull::from(prefs);
        match &self.ivars().inner {
            Some(inner) if responds(&**inner, sel!(webView:decidePolicyForNavigationAction:decisionHandler:)) => {
                let h = handler.copy();
                let p: Retained<WKWebpagePreferences> = Message::retain(prefs);
                let once = Cell::new(false);
                let blk = RcBlock::new(move |policy: WKNavigationActionPolicy| {
                    if once.replace(true) {
                        return;
                    }
                    h.call((policy, NonNull::from(&*p)));
                });
                unsafe {
                    let _: () = msg_send![&**inner, webView: wv, decidePolicyForNavigationAction: action, decisionHandler: &*blk];
                }
            }
            _ => handler.call((WKNavigationActionPolicy::Allow, prefs_ptr)),
        }
    }
}

// ------------------------------------------------------------------ UI delegate

pub struct UiIvars {
    pub inner: Option<Retained<ProtocolObject<dyn WKUIDelegate>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = UiIvars]
    pub struct SericaUIDelegate;

    unsafe impl NSObjectProtocol for SericaUIDelegate {}

    unsafe impl WKUIDelegate for SericaUIDelegate {
        /// `target=_blank` links and `window.open`: a real popup window for
        /// sign-in / payment windows and blank windows the page steers
        /// (`popup.rs`); otherwise Tauri's `on_new_window`, a new tab.
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(
            &self,
            wv: &WKWebView,
            config: &WKWebViewConfiguration,
            action: &WKNavigationAction,
            features: &WKWindowFeatures,
        ) -> Option<Retained<WKWebView>> {
            if super::popup::wants_popup(action, features) {
                super::popup::open(wv, config, features)
            } else {
                match self.ivars().inner.as_ref() {
                Some(inner) if responds(&**inner, sel!(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:)) => unsafe {
                    msg_send![&**inner, webView: wv, createWebViewWithConfiguration: config, forNavigationAction: action, windowFeatures: features]
                },
                _ => None,
                }
            }
        }

        #[unsafe(method(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn alert(&self, wv: &WKWebView, message: &NSString, frame: &WKFrameInfo, handler: &block2::Block<dyn Fn()>) {
            super::dialogs::script_dialog(wv, "alert", message, None, frame, super::dialogs::Reply::Alert(handler.copy()));
        }

        #[unsafe(method(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn confirm(&self, wv: &WKWebView, message: &NSString, frame: &WKFrameInfo, handler: &block2::Block<dyn Fn(Bool)>) {
            super::dialogs::script_dialog(wv, "confirm", message, None, frame, super::dialogs::Reply::Confirm(handler.copy()));
        }

        #[unsafe(method(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:))]
        fn prompt(&self, wv: &WKWebView, prompt: &NSString, default_text: Option<&NSString>, frame: &WKFrameInfo, handler: &block2::Block<dyn Fn(*mut NSString)>) {
            super::dialogs::script_dialog(wv, "prompt", prompt, default_text, frame, super::dialogs::Reply::Prompt(handler.copy()));
        }

        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn media_permission(&self, wv: &WKWebView, origin: &WKSecurityOrigin, _frame: &WKFrameInfo, kind: WKMediaCaptureType, handler: &block2::Block<dyn Fn(WKPermissionDecision)>) {
            let kinds: &[&'static str] = match kind {
                WKMediaCaptureType::Camera => &["camera"],
                WKMediaCaptureType::Microphone => &["microphone"],
                _ => &["camera", "microphone"],
            };
            super::dialogs::permission(wv, origin, kinds, super::dialogs::Decide::Media(handler.copy()));
        }

        #[unsafe(method(webView:requestDeviceOrientationAndMotionPermissionForOrigin:initiatedByFrame:decisionHandler:))]
        fn motion_permission(&self, wv: &WKWebView, origin: &WKSecurityOrigin, _frame: &WKFrameInfo, handler: &block2::Block<dyn Fn(WKPermissionDecision)>) {
            super::dialogs::permission(wv, origin, &["sensors"], super::dialogs::Decide::Media(handler.copy()));
        }

        /// `<input type=file>`: wry's open panel.
        #[unsafe(method(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:))]
        fn open_panel(&self, wv: &WKWebView, params: &objc2_web_kit::WKOpenPanelParameters, frame: &WKFrameInfo, handler: &block2::Block<dyn Fn(*mut NSArray<NSURL>)>) {
            match &self.ivars().inner {
                Some(inner) if responds(&**inner, sel!(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:)) => unsafe {
                    let _: () = msg_send![&**inner, webView: wv, runOpenPanelWithParameters: params, initiatedByFrame: frame, completionHandler: handler];
                },
                _ => handler.call((std::ptr::null_mut(),)),
            }
        }
    }

    /// WebKit's private UI delegate methods Safari uses: "Leave page?",
    /// location and notification permission requests.
    impl SericaUIDelegate {
        #[unsafe(method(_webView:runBeforeUnloadConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn before_unload(&self, wv: &WKWebView, message: &NSString, frame: &WKFrameInfo, handler: &block2::Block<dyn Fn(Bool)>) {
            super::dialogs::script_dialog(wv, "beforeunload", message, None, frame, super::dialogs::Reply::Confirm(handler.copy()));
        }

        #[unsafe(method(_webView:requestGeolocationPermissionForOrigin:initiatedByFrame:decisionHandler:))]
        fn geolocation_permission(&self, wv: &WKWebView, origin: &WKSecurityOrigin, _frame: &WKFrameInfo, handler: &block2::Block<dyn Fn(WKPermissionDecision)>) {
            super::dialogs::permission(wv, origin, &["location"], super::dialogs::Decide::Media(handler.copy()));
        }

        #[unsafe(method(_webView:requestGeolocationPermissionForFrame:decisionHandler:))]
        fn geolocation_permission_legacy(&self, wv: &WKWebView, frame: &WKFrameInfo, handler: &block2::Block<dyn Fn(Bool)>) {
            let origin = unsafe { frame.securityOrigin() };
            super::dialogs::permission(wv, &origin, &["location"], super::dialogs::Decide::Bool(handler.copy()));
        }

        #[unsafe(method(_webView:requestNotificationPermissionForSecurityOrigin:decisionHandler:))]
        fn notification_permission(&self, wv: &WKWebView, origin: &WKSecurityOrigin, handler: &block2::Block<dyn Fn(Bool)>) {
            super::dialogs::permission(wv, origin, &["notifications"], super::dialogs::Decide::Bool(handler.copy()));
        }
    }
);

impl SericaUIDelegate {
    pub fn new(mtm: MainThreadMarker, inner: Option<Retained<ProtocolObject<dyn WKUIDelegate>>>) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(UiIvars { inner });
        unsafe { msg_send![super(this), init] }
    }
}

// ------------------------------------------------------------------ script messages

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    pub struct SericaScriptHandler;

    unsafe impl NSObjectProtocol for SericaScriptHandler {}

    unsafe impl WKScriptMessageHandlerWithReply for SericaScriptHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:replyHandler:))]
        fn did_receive(&self, _ucc: &WKUserContentController, message: &WKScriptMessage, reply: &block2::Block<dyn Fn(*mut AnyObject, *mut NSString)>) {
            let _perf = crate::perf::span("wk.didReceiveScriptMessage");
            let answer = super::messages::handle(message);
            match answer.as_ref().and_then(crate::mac_util::from_json) {
                Some(obj) => reply.call((Retained::as_ptr(&obj) as *mut AnyObject, std::ptr::null_mut())),
                None => reply.call((std::ptr::null_mut(), std::ptr::null_mut())),
            }
        }
    }
);

impl SericaScriptHandler {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

// ------------------------------------------------------------------ key-value observing

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    pub struct SericaObserver;

    unsafe impl NSObjectProtocol for SericaObserver {}

    impl SericaObserver {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(&self, key_path: Option<&NSString>, object: Option<&AnyObject>, _change: Option<&NSDictionary<NSKeyValueChangeKey, AnyObject>>, _context: *mut std::ffi::c_void) {
            let (Some(key), Some(obj)) = (key_path, object) else { return };
            let Some(wv) = obj.downcast_ref::<WKWebView>() else { return };
            super::nav::observed(wv, &key.to_string());
        }
    }
);

impl SericaObserver {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

/// Keys observed on every tab view (those this macOS has).
pub const OBSERVED: &[(&str, &str)] = &[
    ("URL", "URL"),
    ("title", "title"),
    ("canGoBack", "canGoBack"),
    ("canGoForward", "canGoForward"),
    ("_isPlayingAudio", "_isPlayingAudio"),
    ("fullscreenState", "fullscreenState"),
    ("themeColor", "themeColor"),
];

pub fn observe_all(ctx: &Ctx, wv: &WKWebView, observer: &SericaObserver) {
    // macOS 26 AppKit/WKWebView KVO setup is crashing during display-cycle
    // updates. Disable custom observations until they can use a safe, tied
    // lifetime implementation.
    let _ = (ctx, wv, observer);
}

pub fn unobserve_all(ctx: &Ctx, wv: &WKWebView) {
    let Some(observer) = ctx.observer.borrow().clone() else { return };
    for key in ctx.observed.borrow_mut().drain(..) {
        unsafe {
            wv.removeObserver_forKeyPath(&observer, &ns(key));
        }
    }
}
