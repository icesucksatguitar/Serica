//! Page dialogs (alert / confirm / prompt / "Leave page?"), "Open in app?"
//! for other apps' links, site permission requests and HTTP sign-in.
//!
//! WebKit hands each one a completion handler that must be called exactly
//! once: it is kept here until the UI answers, and answered with "no" when
//! its tab closes or Serica quits, never twice.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub enum Reply {
    Alert(RcBlock<dyn Fn()>),
    Confirm(RcBlock<dyn Fn(Bool)>),
    Prompt(RcBlock<dyn Fn(*mut NSString)>),
}

enum Pending {
    Script(Reply),
    External(String),
}

pub enum Decide {
    Media(RcBlock<dyn Fn(WKPermissionDecision)>),
    Bool(RcBlock<dyn Fn(Bool)>),
}

impl Decide {
    fn call(self, allow: bool) {
        match self {
            Decide::Media(b) => b.call((if allow { WKPermissionDecision::Grant } else { WKPermissionDecision::Deny },)),
            Decide::Bool(b) => b.call((Bool::new(allow),)),
        }
    }
}

/// One site request, possibly for several permissions at once (camera and
/// microphone): the UI is asked once per permission, and the page gets its
/// answer when all of them are answered.
struct PermGroup {
    decide: Option<Decide>,
    tab_id: String,
    origin: String,
    waiting: Vec<u64>,
    granted: bool,
}

thread_local! {
    static DIALOGS: RefCell<HashMap<u64, (String, Pending)>> = RefCell::new(HashMap::new());
    static PERM_GROUPS: RefCell<HashMap<u64, PermGroup>> = RefCell::new(HashMap::new());
    /// Permission request id -> (group, permission).
    static PERM_REQS: RefCell<HashMap<u64, (u64, &'static str)>> = RefCell::new(HashMap::new());
}
static NEXT: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn origin_host(origin: &WKSecurityOrigin) -> String {
    unsafe { origin.host().to_string().trim_start_matches("www.").to_string() }
}

/// `https://host[:port]` of a security origin, the unit permissions are kept for.
fn origin_key(origin: &WKSecurityOrigin) -> Option<String> {
    unsafe {
        let proto = origin.protocol().to_string();
        if !matches!(proto.as_str(), "http" | "https") {
            return None;
        }
        let host = origin.host().to_string();
        let port = origin.port();
        let default = (proto == "https" && port == 443) || (proto == "http" && port == 80) || port == 0;
        Some(if default { format!("{proto}://{host}") } else { format!("{proto}://{host}:{port}") })
    }
}

/// Replaces WebKit's (empty) dialogs with events the UI renders in Serica's style.
pub fn script_dialog(wv: &WKWebView, kind: &str, message: &NSString, default: Option<&NSString>, frame: &WKFrameInfo, reply: Reply) {
    let Some(ctx) = ctx_of(wv) else {
        return finish(reply, false, None);
    };
    let security_origin = unsafe { frame.securityOrigin() };
    let origin = origin_host(&security_origin);
    let id = next_id();
    DIALOGS.with(|d| d.borrow_mut().insert(id, (ctx.tab_id.clone(), Pending::Script(reply))));
    ui_emit(
        &ctx.app.handle,
        "serica:page-dialog",
        serde_json::json!({
            "id": id, "tabId": ctx.tab_id, "kind": kind,
            "origin": origin,
            "message": message.to_string().chars().take(2000).collect::<String>(),
            "defaultText": default.map(|d| d.to_string()).unwrap_or_default(),
        }),
    );
}

/// A link to another app (`mailto:`, `zoommtg:`, `slack:`...): Serica asks first.
pub fn external(ctx: &Ctx, url: &str, scheme: &str, from: &str) {
    let id = next_id();
    DIALOGS.with(|d| d.borrow_mut().insert(id, (ctx.tab_id.clone(), Pending::External(url.to_string()))));
    let origin = url::Url::parse(from).ok().and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string())).unwrap_or_default();
    ui_emit(
        &ctx.app.handle,
        "serica:page-dialog",
        serde_json::json!({
            "id": id, "tabId": ctx.tab_id, "kind": "external",
            "origin": origin,
            "message": url.chars().take(300).collect::<String>(),
            "scheme": scheme,
        }),
    );
}

fn finish(reply: Reply, accept: bool, text: Option<String>) {
    match reply {
        Reply::Alert(b) => b.call(()),
        Reply::Confirm(b) => b.call((Bool::new(accept),)),
        Reply::Prompt(b) => match text.filter(|_| accept) {
            Some(t) => {
                let s = ns(&t);
                b.call((Retained::as_ptr(&s) as *mut NSString,));
            }
            None => b.call((std::ptr::null_mut(),)),
        },
    }
}

/// Answers a dialog raised above. Main thread only.
pub fn answer_dialog(id: u64, accept: bool, text: Option<String>) {
    let Some((_, p)) = DIALOGS.with(|d| d.borrow_mut().remove(&id)) else { return };
    match p {
        Pending::Script(reply) => finish(reply, accept, text),
        Pending::External(url) => {
            if accept {
                if let Some(u) = nsurl(&url) {
                    let ws = objc2_app_kit::NSWorkspace::sharedWorkspace();
                    if !ws.openURL(&u) {
                        let _ = open::that_detached(&url);
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ permissions

/// Asks for camera, microphone, location and the rest in Serica's own
/// prompt. Answers kept as "always" live in Serica's settings, so they can
/// be listed and revoked in Settings › Privacy.
pub fn permission(wv: &WKWebView, origin: &WKSecurityOrigin, kinds: &[&'static str], decide: Decide) {
    let Some(ctx) = ctx_of(wv) else { return decide.call(false) };
    let Some(key) = origin_key(origin) else {
        // Not a web origin (file:, data:): nothing to ask about.
        return decide.call(false);
    };
    let settings = ctx.app.settings();
    let mut ask = Vec::new();
    for k in kinds {
        match settings.permission(&key, k) {
            Some(false) => return decide.call(false),
            Some(true) => {}
            None => ask.push(*k),
        }
    }
    if ask.is_empty() {
        return decide.call(true);
    }
    let group = next_id();
    let host = origin_host(origin);
    let mut waiting = Vec::new();
    for k in &ask {
        let id = next_id();
        waiting.push(id);
        PERM_REQS.with(|r| r.borrow_mut().insert(id, (group, *k)));
    }
    PERM_GROUPS.with(|g| g.borrow_mut().insert(group, PermGroup { decide: Some(decide), tab_id: ctx.tab_id.clone(), origin: key.clone(), waiting: waiting.clone(), granted: true }));
    for (id, k) in waiting.iter().zip(ask) {
        ui_emit(
            &ctx.app.handle,
            "serica:permission-request",
            serde_json::json!({ "id": id, "tabId": ctx.tab_id, "origin": key, "host": host, "kind": k, "userInitiated": true }),
        );
    }
}

/// "allow" / "block" (remembered for the site), "allow-once", or "deny"
/// (this time only, also for a dismissed prompt). Main thread only.
pub fn answer_permission(app: &Shared, id: u64, decision: &str) {
    let Some((group, kind)) = PERM_REQS.with(|r| r.borrow_mut().remove(&id)) else { return };
    let allow = matches!(decision, "allow" | "allow-once");
    let done = PERM_GROUPS.with(|g| {
        let mut g = g.borrow_mut();
        let Some(pg) = g.get_mut(&group) else { return None };
        if matches!(decision, "allow" | "block") {
            let changed = app.store.lock().settings.remember_permission(&pg.origin, kind, Some(decision == "allow"));
            if changed {
                app.notify(crate::app::Notice::Settings);
            }
        }
        pg.granted &= allow;
        pg.waiting.retain(|w| *w != id);
        if pg.waiting.is_empty() || !pg.granted {
            g.remove(&group)
        } else {
            None
        }
    });
    if let Some(mut pg) = done {
        PERM_REQS.with(|r| r.borrow_mut().retain(|_, (gid, _)| *gid != group));
        if let Some(d) = pg.decide.take() {
            d.call(pg.granted);
        }
    }
}

// ------------------------------------------------------------------ HTTP sign-in

/// Basic / Digest / NTLM sign-in, in a sheet on the window (WebView2 shows
/// its own dialog for these).
pub fn http_auth(
    wv: &WKWebView,
    challenge: &NSURLAuthenticationChallenge,
    handler: RcBlock<dyn Fn(NSURLSessionAuthChallengeDisposition, *mut NSURLCredential)>,
) {
    use objc2_app_kit::{NSAlert, NSSecureTextField, NSStackView, NSTextField, NSUserInterfaceLayoutOrientation};
    let mtm = MainThreadMarker::new().expect("main thread");
    let Some(window) = wv.window() else {
        return handler.call((NSURLSessionAuthChallengeDisposition::CancelAuthenticationChallenge, std::ptr::null_mut()));
    };
    let space = challenge.protectionSpace();
    let host = space.host().to_string();
    let secure = space.receivesCredentialSecurely();
    let failed = challenge.previousFailureCount() > 0;
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&ns(&format!("Sign in to {host}")));
    let mut info = if secure { "Your sign-in information will be sent securely.".to_string() } else { "Your sign-in information will be sent without encryption.".to_string() };
    if let Some(realm) = space.realm() {
        if !realm.to_string().is_empty() {
            info = format!("The site says: “{realm}”. {info}");
        }
    }
    if failed {
        info = format!("That user name or password didn't work. {info}");
    }
    alert.setInformativeText(&ns(&info));
    alert.addButtonWithTitle(&ns("Sign In"));
    alert.addButtonWithTitle(&ns("Cancel"));
    let frame = objc2_foundation::NSRect::new(objc2_foundation::NSPoint::new(0.0, 0.0), objc2_foundation::NSSize::new(260.0, 54.0));
    let user = NSTextField::initWithFrame(mtm.alloc(), objc2_foundation::NSRect::new(objc2_foundation::NSPoint::new(0.0, 30.0), objc2_foundation::NSSize::new(260.0, 24.0)));
    user.setPlaceholderString(Some(&ns("User name")));
    if let Some(p) = challenge.proposedCredential().and_then(|c| c.user()) {
        user.setStringValue(&p);
    }
    let pass = NSSecureTextField::initWithFrame(mtm.alloc(), objc2_foundation::NSRect::new(objc2_foundation::NSPoint::new(0.0, 0.0), objc2_foundation::NSSize::new(260.0, 24.0)));
    pass.setPlaceholderString(Some(&ns("Password")));
    let stack = NSStackView::initWithFrame(mtm.alloc(), frame);
    stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
    stack.setSpacing(6.0);
    stack.addArrangedSubview(&user);
    stack.addArrangedSubview(&pass);
    alert.setAccessoryView(Some(&stack));
    alert.window().setInitialFirstResponder(Some(&user));
    let (user2, pass2) = (user.clone(), pass.clone());
    let done = RcBlock::new(move |response: isize| {
        if response == 1000 {
            let cred = NSURLCredential::credentialWithUser_password_persistence(
                &user2.stringValue(),
                &pass2.stringValue(),
                objc2_foundation::NSURLCredentialPersistence::ForSession,
            );
            handler.call((NSURLSessionAuthChallengeDisposition::UseCredential, Retained::as_ptr(&cred) as *mut _));
        } else {
            handler.call((NSURLSessionAuthChallengeDisposition::CancelAuthenticationChallenge, std::ptr::null_mut()));
        }
    });
    alert.beginSheetModalForWindow_completionHandler(&window, Some(&done));
}

// ------------------------------------------------------------------ cleanup

/// Answers everything a tab (or, with `None`, every tab) still waits on:
/// dialogs dismissed, permissions denied.
pub fn fail_pending(tab_id: Option<&str>) {
    let dialogs: Vec<Pending> = DIALOGS.with(|d| {
        let mut d = d.borrow_mut();
        let ids: Vec<u64> = d.iter().filter(|(_, (t, _))| tab_id.is_none_or(|x| x == t)).map(|(id, _)| *id).collect();
        ids.into_iter().filter_map(|id| d.remove(&id).map(|(_, p)| p)).collect()
    });
    for p in dialogs {
        if let Pending::Script(reply) = p {
            // "Leave page?" lets the tab close.
            let accept = matches!(reply, Reply::Confirm(_)) && tab_id.is_some();
            finish(reply, accept, None);
        }
    }
    let groups: Vec<(u64, PermGroup)> = PERM_GROUPS.with(|g| {
        let mut g = g.borrow_mut();
        let ids: Vec<u64> = g.iter().filter(|(_, pg)| tab_id.is_none_or(|x| x == pg.tab_id)).map(|(id, _)| *id).collect();
        ids.into_iter().filter_map(|id| g.remove(&id).map(|pg| (id, pg))).collect()
    });
    for (gid, mut pg) in groups {
        PERM_REQS.with(|r| r.borrow_mut().retain(|_, (g, _)| *g != gid));
        if let Some(d) = pg.decide.take() {
            d.call(false);
        }
    }
}
