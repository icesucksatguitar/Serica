//! Running extensions in WebKit.
//!
//! WebView2 loads Chrome extensions itself (`AddBrowserExtension`). WebKit
//! runs extensions in its own runtime, `WKWebExtensionController` (the one
//! Safari uses, macOS 15.4+): a Chrome Web Store package unpacked by Serica
//! is a web extension WebKit can load as it is (manifest, background service
//! worker, content scripts, popup and options pages). Each enabled extension
//! gets a context with the permissions it asked for granted (installing it
//! was the consent, as in Chrome), and every tab's configuration carries the
//! controller.
//!
//! Before macOS 15.4 there is no such runtime: Serica injects the content
//! scripts (and styles) an extension declares into the pages they match, in
//! the extension's own script world, with a small `chrome.runtime` /
//! `chrome.i18n` / in-memory `chrome.storage`. What needs Chrome's background
//! runtime says so in the Extensions panel instead of failing silently.

use super::{read_manifest, ExtensionRecord, Extensions};
use crate::app::Shared;
use crate::mac_util::{js_string, ns, on_main, os_version, post_main};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_foundation::{NSError, NSURL};
use objc2_web_kit::{
    WKContentWorld, WKUserContentController, WKUserScript, WKUserScriptInjectionTime, WKWebExtension, WKWebExtensionContext, WKWebExtensionContextPermissionStatus,
    WKWebExtensionController, WKWebExtensionControllerConfiguration, WKWebViewConfiguration,
};
use parking_lot::Mutex;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

const RUNTIME_UNKNOWN: u8 = 0;
const RUNTIME_WEBKIT: u8 = 1;
const RUNTIME_SCRIPTS: u8 = 2;
static RUNTIME: AtomicU8 = AtomicU8::new(RUNTIME_UNKNOWN);

/// What became of each extension in WebKit's runtime: "loaded",
/// "loading", or the error WebKit gave. Readable from any thread.
static STATE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

thread_local! {
    static CONTROLLER: RefCell<Option<Retained<WKWebExtensionController>>> = const { RefCell::new(None) };
    static CONTEXTS: RefCell<HashMap<String, Retained<WKWebExtensionContext>>> = RefCell::new(HashMap::new());
}

fn set_state(id: &str, s: &str) {
    STATE.lock().get_or_insert_with(HashMap::new).insert(id.to_string(), s.to_string());
}

fn state_of(id: &str) -> Option<String> {
    STATE.lock().as_ref().and_then(|m| m.get(id).cloned())
}

/// WebKit's extension runtime is here (macOS 15.4+).
pub fn runtime_available() -> bool {
    let (major, minor, _) = os_version();
    (major > 15 || (major == 15 && minor >= 4)) && objc2::runtime::AnyClass::get(c"WKWebExtensionController").is_some()
}

/// WebKit's extension controller, when its runtime is in use (main thread).
pub fn controller() -> Option<Retained<WKWebExtensionController>> {
    if runtime() != RUNTIME_WEBKIT {
        return None;
    }
    CONTROLLER.with(|c| c.borrow().clone())
}

/// The runtime in use, once `start` ran.
fn runtime() -> u8 {
    RUNTIME.load(Ordering::Relaxed)
}

/// At start, on the main thread: the controller, then every enabled extension.
pub fn start(app: &Shared) {
    let available = runtime_available();
    let a = app.clone();
    post_main(&app.handle, move || {
        if available {
            let mtm = MainThreadMarker::new().expect("main thread");
            unsafe {
                let config = WKWebExtensionControllerConfiguration::defaultConfiguration(mtm);
                let controller = WKWebExtensionController::initWithConfiguration(mtm.alloc(), &config);
                // Serica's window and tabs, for `chrome.tabs` / `chrome.windows`.
                super::webkit_tabs::install(&controller);
                CONTROLLER.with(|c| *c.borrow_mut() = Some(controller));
            }
            RUNTIME.store(RUNTIME_WEBKIT, Ordering::Relaxed);
            tracing::info!("extensions: WebKit's extension runtime");
        } else {
            RUNTIME.store(RUNTIME_SCRIPTS, Ordering::Relaxed);
            tracing::info!("extensions: content scripts only (WebKit's extension runtime needs macOS 15.4)");
        }
        sync_main(&a);
    });
}

/// Brings WebKit's runtime in line with the registry (after an install,
/// removal or switch).
pub fn sync(app: &Shared) {
    let a = app.clone();
    post_main(&app.handle, move || sync_main(&a));
}

fn sync_main(app: &Shared) {
    if runtime() == RUNTIME_SCRIPTS {
        // Open tabs get the new set of scripts from their next page load.
        crate::webview_mac::replace_extension_scripts(app);
        return;
    }
    if runtime() != RUNTIME_WEBKIT {
        return;
    }
    let Some(controller) = CONTROLLER.with(|c| c.borrow().clone()) else { return };
    let records = app.extensions.records();
    // Unload what is gone or switched off.
    let loaded: Vec<String> = CONTEXTS.with(|c| c.borrow().keys().cloned().collect());
    for id in loaded {
        let keep = records.iter().any(|r| r.id == id && r.enabled);
        if !keep {
            if let Some(ctx) = CONTEXTS.with(|c| c.borrow_mut().remove(&id)) {
                unsafe {
                    if let Err(e) = controller.unloadExtensionContext_error(&ctx) {
                        tracing::warn!("could not unload extension {id}: {}", e.localizedDescription());
                    }
                }
            }
            STATE.lock().get_or_insert_with(HashMap::new).remove(&id);
        }
    }
    // Load what is on and not running yet.
    for rec in records.into_iter().filter(|r| r.enabled) {
        if CONTEXTS.with(|c| c.borrow().contains_key(&rec.id)) || state_of(&rec.id).as_deref() == Some("loading") {
            continue;
        }
        let dir = app.extensions.path_of(&rec.id);
        if !dir.join("manifest.json").is_file() {
            continue;
        }
        load(app, &controller, rec.id.clone(), &dir);
    }
}

fn load(app: &Shared, controller: &Retained<WKWebExtensionController>, id: String, dir: &Path) {
    let mtm = MainThreadMarker::new().expect("main thread");
    set_state(&id, "loading");
    let url = NSURL::fileURLWithPath_isDirectory(&ns(&dir.to_string_lossy()), true);
    let (app, controller) = (app.clone(), controller.clone());
    let done = RcBlock::new(move |ext: *mut WKWebExtension, err: *mut NSError| {
        let Some(ext) = (unsafe { ext.as_ref() }) else {
            let why = unsafe { err.as_ref() }.map(|e| e.localizedDescription().to_string()).unwrap_or_else(|| "unknown error".into());
            tracing::warn!("WebKit could not read extension {id}: {why}");
            set_state(&id, &why);
            crate::app::ui_emit(&app.handle, "serica:extensions-updated", app.extensions.list());
            return;
        };
        unsafe {
            let ctx = WKWebExtensionContext::contextForExtension(ext);
            // The extension's Chrome id, as in Chrome: `runtime.id` matches
            // what it expects, and its pages and stored data stay at
            // webkit-extension://<id>/ from one launch to the next (WebKit
            // would pick a new one each time).
            ctx.setUniqueIdentifier(&ns(&id));
            if let Some(base) = crate::mac_util::nsurl(&format!("webkit-extension://{id}/")) {
                ctx.setBaseURL(&base);
            }
            ctx.setInspectable(true);
            // Installing the extension granted what it asked for, as in Chrome.
            for p in ext.requestedPermissions().iter() {
                ctx.setPermissionStatus_forPermission(WKWebExtensionContextPermissionStatus::GrantedExplicitly, &p);
            }
            for m in ext.allRequestedMatchPatterns().iter() {
                ctx.setPermissionStatus_forMatchPattern(WKWebExtensionContextPermissionStatus::GrantedExplicitly, &m);
            }
            match controller.loadExtensionContext_error(&ctx) {
                Ok(()) => {
                    CONTEXTS.with(|c| c.borrow_mut().insert(id.clone(), ctx));
                    set_state(&id, "loaded");
                    tracing::info!("extension {id} running in WebKit");
                }
                Err(e) => {
                    let why = e.localizedDescription().to_string();
                    tracing::warn!("WebKit could not load extension {id}: {why}");
                    set_state(&id, &why);
                }
            }
        }
        crate::app::ui_emit(&app.handle, "serica:extensions-updated", app.extensions.list());
    });
    unsafe { WKWebExtension::extensionWithResourceBaseURL_completionHandler(&url, &done, mtm) };
}

/// How an extension runs here, and one line for the Extensions panel.
pub fn status(exts: &Extensions, rec: &ExtensionRecord) -> (&'static str, String) {
    if !rec.enabled {
        return (if runtime() == RUNTIME_WEBKIT { "webkit" } else { "content-scripts" }, "Turned off".into());
    }
    match runtime() {
        RUNTIME_WEBKIT => match state_of(&rec.id).as_deref() {
            Some("loaded") => ("webkit", "Running in WebKit's extension runtime".into()),
            Some("loading") | None => ("webkit", "Starting…".into()),
            Some(err) => ("unsupported", format!("WebKit couldn't run it: {err}")),
        },
        _ => {
            let manifest = read_manifest(&exts.path_of(&rec.id)).unwrap_or_default();
            let has_scripts = manifest["content_scripts"].as_array().is_some_and(|a| !a.is_empty());
            let needs_background = manifest.get("background").is_some()
                || rec.permissions.iter().any(|p| matches!(p.as_str(), "webRequest" | "webRequestBlocking" | "declarativeNetRequest" | "nativeMessaging" | "debugger" | "proxy" | "tabs"));
            match (has_scripts, needs_background) {
                (true, false) => ("content-scripts", "Content scripts active".into()),
                (true, true) => ("content-scripts", "Content scripts active; its background features need macOS 15.4 or later".into()),
                (false, _) => ("unsupported", "Requires the Chrome extension runtime (WebKit has it from macOS 15.4)".into()),
            }
        }
    }
}

/// Puts the extension runtime (or, without it, the content scripts) in a
/// new tab's configuration. Main thread.
pub fn configure(app: &Shared, config: &WKWebViewConfiguration, ucc: &WKUserContentController) {
    if runtime() == RUNTIME_WEBKIT {
        if let Some(c) = CONTROLLER.with(|c| c.borrow().clone()) {
            unsafe { config.setWebExtensionController(Some(&c)) };
        }
        return;
    }
    install_content_scripts(app, ucc);
}

/// What WebKit says about each loaded extension (test automation).
#[cfg(feature = "automation")]
pub fn diagnostics() -> serde_json::Value {
    CONTEXTS.with(|c| {
        serde_json::Value::Array(
            c.borrow()
                .iter()
                .map(|(id, ctx)| unsafe {
                    let errors: Vec<String> = ctx.errors().iter().map(|e| e.localizedDescription().to_string()).collect();
                    let ext_errors: Vec<String> = ctx.webExtension().errors().iter().map(|e| e.localizedDescription().to_string()).collect();
                    serde_json::json!({
                        "id": id,
                        "loaded": ctx.isLoaded(),
                        "baseURL": crate::mac_util::url_string(Some(&ctx.baseURL())),
                        "uniqueIdentifier": ctx.uniqueIdentifier().to_string(),
                        "background": ctx.webExtension().hasBackgroundContent(),
                        "errors": errors,
                        "extensionErrors": ext_errors,
                    })
                })
                .collect(),
        )
    })
}

/// The URL of an extension's popup or options page (`webkit-extension://`),
/// to open in a tab.
pub async fn page_url(app: &Shared, rec: &ExtensionRecord, page: &str) -> Option<String> {
    let rel = match page {
        "popup" => rec.popup.clone()?,
        "options" => rec.options.clone()?,
        _ => return None,
    };
    let id = rec.id.clone();
    on_main(&app.handle, Duration::from_secs(3), move || {
        CONTEXTS.with(|c| {
            let ctx = c.borrow().get(&id).cloned()?;
            let base = unsafe { ctx.baseURL() };
            let base = base.absoluteString()?.to_string();
            let base = if base.ends_with('/') { base } else { format!("{base}/") };
            Some(format!("{base}{}", rel.trim_start_matches('/')))
        })
    })
    .flatten()
}

/// The configuration for a tab showing an extension's own page
/// (`webkit-extension://<id>/…`): WebKit loads those pages only in web views
/// made from the extension's configuration, as Safari's popups are.
pub fn page_configuration(app: &Shared, url: &str) -> Option<crate::mac_util::MainBox<Retained<WKWebViewConfiguration>>> {
    let host = url::Url::parse(url).ok().filter(|u| u.scheme() == "webkit-extension")?.host_str()?.to_ascii_lowercase();
    on_main(&app.handle, Duration::from_secs(3), move || {
        CONTEXTS.with(|c| {
            c.borrow().values().find_map(|ctx| unsafe {
                let base = ctx.baseURL().host()?.to_string().to_ascii_lowercase();
                (base == host).then(|| ctx.webViewConfiguration()).flatten()
            })
        })
        .map(crate::mac_util::MainBox)
    })
    .flatten()
}

// ------------------------------------------------------------------ content scripts (before macOS 15.4)

/// How every injected extension script starts (a block label), so a tab's
/// set can be swapped when extensions change.
pub const SCRIPT_MARK: &str = "serica_ext:";

/// A Chrome match pattern test, for the injected scripts.
const MATCH_JS: &str = r#"const __sericaMatch = (u, p) => {
  try {
    const url = new URL(u);
    if (p === '<all_urls>') return /^(https?|wss?|file|ftp):$/.test(url.protocol);
    const r = /^(\*|https?|wss?|file|ftp):\/\/(\*|\*\.[^/*]+|[^/*]*)(\/.*)$/.exec(p);
    if (!r) return false;
    const [, s, h, path] = r;
    if (s === '*' ? !/^(https?|wss?):$/.test(url.protocol) : url.protocol !== s + ':') return false;
    if (h && h !== '*') {
      if (h.startsWith('*.')) { const b = h.slice(2); if (url.hostname !== b && !url.hostname.endsWith('.' + b)) return false; }
      else if (url.hostname !== h) return false;
    }
    const re = new RegExp('^' + path.replace(/[.+?^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*') + '$');
    return re.test(url.pathname + url.search);
  } catch (e) { return false; }
};"#;

fn shim(id: &str, manifest: &serde_json::Value, messages: &serde_json::Value) -> String {
    format!(
        r#"(() => {{
  const g = globalThis;
  g.chrome = g.chrome || {{}};
  const store = {{}};
  const area = {{
    get(keys, cb) {{ let out = {{}}; if (keys == null) out = Object.assign({{}}, store); else if (typeof keys === 'string') {{ if (keys in store) out[keys] = store[keys]; }} else if (Array.isArray(keys)) {{ for (const k of keys) if (k in store) out[k] = store[k]; }} else {{ for (const k of Object.keys(keys)) out[k] = k in store ? store[k] : keys[k]; }} if (cb) setTimeout(() => cb(out), 0); return Promise.resolve(out); }},
    set(items, cb) {{ Object.assign(store, items); if (cb) setTimeout(cb, 0); return Promise.resolve(); }},
    remove(keys, cb) {{ for (const k of [].concat(keys)) delete store[k]; if (cb) setTimeout(cb, 0); return Promise.resolve(); }},
    clear(cb) {{ for (const k of Object.keys(store)) delete store[k]; if (cb) setTimeout(cb, 0); return Promise.resolve(); }},
  }};
  const ev = () => ({{ addListener() {{}}, removeListener() {{}}, hasListener: () => false }});
  g.chrome.runtime = g.chrome.runtime || {{
    id: {id}, getManifest: () => ({manifest}), getURL: (p) => 'about:blank#' + p, lastError: undefined,
    sendMessage(...a) {{ const cb = a.find((x) => typeof x === 'function'); if (cb) setTimeout(() => cb(undefined), 0); return Promise.resolve(undefined); }},
    connect: () => ({{ postMessage() {{}}, disconnect() {{}}, onMessage: ev(), onDisconnect: ev() }}), onMessage: ev(), onConnect: ev(),
  }};
  const msgs = {messages};
  g.chrome.i18n = g.chrome.i18n || {{ getMessage: (k, subs) => {{ const m = msgs[k] || msgs[String(k).toLowerCase()]; if (!m) return ''; let s = m.message || ''; [].concat(subs || []).forEach((v, i) => {{ s = s.replace('$' + (i + 1), v); }}); return s; }}, getUILanguage: () => navigator.language }};
  g.chrome.storage = g.chrome.storage || {{ local: area, sync: area, session: area, onChanged: ev() }};
  g.browser = g.browser || g.chrome;
}})();"#,
        id = js_string(id),
        manifest = manifest,
        messages = messages,
    )
}

/// The default locale's messages, keyed in lower case (as Chrome matches them).
fn messages(dir: &Path, manifest: &serde_json::Value) -> serde_json::Value {
    let loc = manifest["default_locale"].as_str().unwrap_or("en");
    for l in [loc, "en", "en_US"] {
        if let Ok(raw) = std::fs::read_to_string(dir.join("_locales").join(l).join("messages.json")) {
            if let Ok(serde_json::Value::Object(m)) = serde_json::from_str::<serde_json::Value>(super::strip_bom(&raw)) {
                return serde_json::Value::Object(m.into_iter().map(|(k, v)| (k.to_lowercase(), v)).collect());
            }
        }
    }
    serde_json::json!({})
}

/// The user scripts for one extension's `content_scripts` entries.
pub fn content_script_sources(id: &str, dir: &Path) -> Vec<(String, bool, bool, bool)> {
    let Ok(manifest) = read_manifest(dir) else { return Vec::new() };
    let msgs = messages(dir, &manifest);
    let Some(entries) = manifest["content_scripts"].as_array() else { return Vec::new() };
    let list = |v: &serde_json::Value| -> Vec<String> { v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
    let read = |rel: &str| -> Option<String> {
        let p = dir.join(rel.trim_start_matches('/'));
        // Stay inside the extension's folder.
        let canon = std::fs::canonicalize(&p).ok()?;
        let root = std::fs::canonicalize(dir).ok()?;
        if !canon.starts_with(&root) {
            return None;
        }
        let meta = std::fs::metadata(&canon).ok()?;
        (meta.len() < 4 * 1024 * 1024).then(|| std::fs::read_to_string(&canon).ok()).flatten()
    };
    let mut out = Vec::new();
    for e in entries {
        let matches = list(&e["matches"]);
        if matches.is_empty() {
            continue;
        }
        let excludes = list(&e["exclude_matches"]);
        let js: Vec<String> = list(&e["js"]).iter().filter_map(|f| read(f)).collect();
        let css: Vec<String> = list(&e["css"]).iter().filter_map(|f| read(f)).collect();
        if js.is_empty() && css.is_empty() {
            continue;
        }
        let at_start = e["run_at"].as_str() == Some("document_start");
        let all_frames = e["all_frames"].as_bool().unwrap_or(false);
        let main_world = e["world"].as_str() == Some("MAIN");
        let css_js = if css.is_empty() {
            String::new()
        } else {
            format!(
                "{{ const css = {}; const add = () => {{ const s = document.createElement('style'); s.textContent = css; (document.head || document.documentElement).appendChild(s); }}; if (document.documentElement) add(); else document.addEventListener('readystatechange', add, {{ once: true }}); }}\n",
                js_string(&css.join("\n"))
            )
        };
        let source = format!(
            "{SCRIPT_MARK} {{\n{MATCH_JS}\nif (!{matches}.some((p) => __sericaMatch(location.href, p)) || {excludes}.some((p) => __sericaMatch(location.href, p))) break serica_ext;\n{shim}\n{css_js}{js}\n}}",
            matches = serde_json::to_string(&matches).unwrap_or_default(),
            excludes = serde_json::to_string(&excludes).unwrap_or_default(),
            shim = if main_world { String::new() } else { shim(id, &manifest, &msgs) },
            js = js.join("\n;\n"),
        );
        out.push((source, at_start, all_frames, main_world));
    }
    out
}

/// The user scripts for every enabled extension (before macOS 15.4). Main thread.
pub fn user_scripts(app: &Shared) -> Vec<Retained<WKUserScript>> {
    if runtime() == RUNTIME_WEBKIT {
        return Vec::new();
    }
    let mtm = MainThreadMarker::new().expect("main thread");
    let mut out = Vec::new();
    for rec in app.extensions.records().into_iter().filter(|r| r.enabled) {
        let dir = app.extensions.path_of(&rec.id);
        for (source, at_start, all_frames, main_world) in content_script_sources(&rec.id, &dir) {
            unsafe {
                let world = if main_world { WKContentWorld::pageWorld(mtm) } else { WKContentWorld::worldWithName(&ns(&format!("ext-{}", rec.id)), mtm) };
                out.push(WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
                    mtm.alloc(),
                    &ns(&source),
                    if at_start { WKUserScriptInjectionTime::AtDocumentStart } else { WKUserScriptInjectionTime::AtDocumentEnd },
                    !all_frames,
                    &world,
                ));
            }
        }
    }
    out
}

fn install_content_scripts(app: &Shared, ucc: &WKUserContentController) {
    for script in user_scripts(app) {
        unsafe { ucc.addUserScript(&script) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_scripts_are_built_from_the_manifest() {
        let dir = std::env::temp_dir().join(format!("serica-ext-cs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("_locales/en")).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            r#"{"name":"T","version":"1","manifest_version":3,"default_locale":"en","content_scripts":[{"matches":["*://*.example.com/*"],"exclude_matches":["*://ads.example.com/*"],"js":["a.js"],"css":["a.css"],"run_at":"document_start","all_frames":true},{"matches":["<all_urls>"],"js":["../escape.js"]}]}"#,
        )
        .unwrap();
        std::fs::write(dir.join("a.js"), "window.__x = 1;").unwrap();
        std::fs::write(dir.join("a.css"), "body{color:red}").unwrap();
        std::fs::write(dir.join("_locales/en/messages.json"), r#"{"Hello":{"message":"Hi $1"}}"#).unwrap();
        let s = content_script_sources("abcdefghijklmnopabcdefghijklmnop", &dir);
        // The second entry points outside the folder: skipped.
        assert_eq!(s.len(), 1);
        let (src, at_start, all_frames, main_world) = &s[0];
        assert!(*at_start && *all_frames && !*main_world);
        assert!(src.contains("window.__x = 1;") && src.contains("body{color:red}") && src.contains("\"hello\""));
        assert!(src.contains("break serica_ext"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
