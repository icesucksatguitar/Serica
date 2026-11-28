//! Small helpers shared by the macOS (AppKit / WebKit) modules.
//!
//! Every AppKit and WebKit object lives on the main thread. Code elsewhere
//! (the sync thread, IPC handlers on the async runtime) reaches it through
//! `on_main`, which runs a closure on the main thread and waits for its
//! result, or runs it right away when already there.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{MainThreadMarker, NSString, NSURL};
use std::sync::mpsc;
use std::time::Duration;

/// Holds a main-thread-only object so it can travel through `Send` APIs.
/// It must only be touched (and dropped) on the main thread.
pub struct MainBox<T>(pub T);
// SAFETY: the wrapped value is only used on the main thread; callers move it
// across threads without dereferencing it there.
unsafe impl<T> Send for MainBox<T> {}
unsafe impl<T> Sync for MainBox<T> {}

pub fn is_main_thread() -> bool {
    MainThreadMarker::new().is_some()
}

/// Runs `f` on the main thread and returns its result (`None` if the main
/// thread did not answer within `timeout`, e.g. while the app is quitting).
pub fn on_main<T: Send + 'static>(handle: &tauri::AppHandle, timeout: Duration, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    if is_main_thread() {
        return Some(f());
    }
    let (tx, rx) = mpsc::sync_channel(1);
    handle
        .run_on_main_thread(move || {
            let _ = tx.send(f());
        })
        .ok()?;
    rx.recv_timeout(timeout).ok()
}

/// Runs `f` on the main thread without waiting.
pub fn post_main(handle: &tauri::AppHandle, f: impl FnOnce() + Send + 'static) {
    if is_main_thread() {
        f();
    } else {
        let _ = handle.run_on_main_thread(f);
    }
}

pub fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

pub fn nsurl(s: &str) -> Option<Retained<NSURL>> {
    NSURL::URLWithString(&ns(s))
}

/// `absoluteString` of an optional URL ("" when there is none).
pub fn url_string(u: Option<&NSURL>) -> String {
    u.and_then(|u| u.absoluteString()).map(|s| s.to_string()).unwrap_or_default()
}

/// A JSON string literal for embedding in scripts.
pub fn js_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// `[obj respondsToSelector:sel]`.
pub fn responds<T: objc2::Message + ?Sized>(obj: &T, sel: objc2::runtime::Sel) -> bool {
    unsafe { objc2::msg_send![obj, respondsToSelector: sel] }
}

/// macOS version (major, minor, patch).
pub fn os_version() -> (isize, isize, isize) {
    let v = objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersion();
    (v.majorVersion, v.minorVersion, v.patchVersion)
}

/// Converts a Foundation object (from a script message or an evaluation
/// result) to JSON, through `NSJSONSerialization`.
pub fn to_json(obj: Option<&AnyObject>) -> serde_json::Value {
    use objc2_foundation::{NSJSONSerialization, NSJSONWritingOptions};
    let Some(obj) = obj else { return serde_json::Value::Null };
    unsafe {
        let ok: bool = NSJSONSerialization::isValidJSONObject(obj);
        if !ok {
            // Plain strings, numbers and booleans are fragments.
            if let Some(s) = obj.downcast_ref::<NSString>() {
                return serde_json::Value::String(s.to_string());
            }
            if let Some(n) = obj.downcast_ref::<objc2_foundation::NSNumber>() {
                let ty: *const std::ffi::c_char = objc2::msg_send![n, objCType];
                let t = if ty.is_null() { "" } else { std::ffi::CStr::from_ptr(ty).to_str().unwrap_or("") };
                return match t {
                    "c" | "B" => serde_json::Value::Bool(n.boolValue()),
                    "f" | "d" => serde_json::json!(n.doubleValue()),
                    _ => serde_json::json!(n.longLongValue()),
                };
            }
            if let Ok(data) = NSJSONSerialization::dataWithJSONObject_options_error(obj, NSJSONWritingOptions::FragmentsAllowed) {
                return serde_json::from_slice(&data.to_vec()).unwrap_or(serde_json::Value::Null);
            }
            return serde_json::Value::Null;
        }
        match NSJSONSerialization::dataWithJSONObject_options_error(obj, NSJSONWritingOptions::FragmentsAllowed) {
            Ok(data) => serde_json::from_slice(&data.to_vec()).unwrap_or(serde_json::Value::Null),
            Err(_) => serde_json::Value::Null,
        }
    }
}

/// Converts JSON to a Foundation object (for script message replies).
pub fn from_json(v: &serde_json::Value) -> Option<Retained<AnyObject>> {
    use objc2_foundation::{NSData, NSJSONReadingOptions, NSJSONSerialization};
    let bytes = serde_json::to_vec(v).ok()?;
    let data = NSData::with_bytes(&bytes);
    NSJSONSerialization::JSONObjectWithData_options_error(&data, NSJSONReadingOptions::FragmentsAllowed).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_strings_are_escaped() {
        assert_eq!(js_string("a\"b"), r#""a\"b""#);
        assert_eq!(js_string("</script>"), r#""</script>""#);
    }

    #[test]
    fn os_version_is_sane() {
        let (major, _, _) = os_version();
        assert!(major >= 11, "macOS {major}");
    }
}
