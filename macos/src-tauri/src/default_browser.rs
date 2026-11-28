//! Making Serica the default browser on macOS.
//!
//! macOS asks the user itself: setting the app for the `http` scheme shows
//! the system's "Do you want to change your default web browser?" prompt,
//! and the choice covers `https` too. Web pages (`public.html`) and PDFs go
//! along, as Windows' "Set default" does for the types a browser registers.
//! Serica declares all of these in its Info.plist.

use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSBundle, NSString, NSURL};

fn own_bundle_id() -> Option<String> {
    NSBundle::mainBundle().bundleIdentifier().map(|s| s.to_string())
}

/// The bundle id of the app macOS opens https links with.
fn default_handler_id() -> Option<String> {
    let ws = NSWorkspace::sharedWorkspace();
    let url = NSURL::URLWithString(&NSString::from_str("https://example.com/"))?;
    let app = ws.URLForApplicationToOpenURL(&url)?;
    NSBundle::bundleWithURL(&app).and_then(|b| b.bundleIdentifier()).map(|s| s.to_string())
}

/// Whether macOS sends https links to this Serica. A build run outside an
/// app bundle (no bundle id) can't be the default.
pub fn is_default() -> bool {
    match (own_bundle_id(), default_handler_id()) {
        (Some(own), Some(current)) => own.eq_ignore_ascii_case(&current),
        _ => false,
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultBrowserStatus {
    pub is_default: bool,
    pub supported: bool,
}

#[tauri::command]
pub fn default_browser_status() -> DefaultBrowserStatus {
    DefaultBrowserStatus { is_default: is_default(), supported: own_bundle_id().is_some() }
}

/// Asks macOS to make Serica the default browser (the system confirms with
/// the user); the UI polls `default_browser_status` to see the choice land.
#[tauri::command]
pub async fn make_default_browser(app: tauri::AppHandle) -> Result<DefaultBrowserStatus, String> {
    if own_bundle_id().is_none() {
        return Err("Serica must run from its app bundle to become the default browser".into());
    }
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    crate::mac_util::post_main(&app, move || unsafe {
        use block2::RcBlock;
        use std::cell::Cell;
        let ws = NSWorkspace::sharedWorkspace();
        let bundle = NSBundle::mainBundle().bundleURL();
        let tx = std::rc::Rc::new(Cell::new(Some(tx)));
        let t2 = tx.clone();
        let done = RcBlock::new(move |err: *mut objc2_foundation::NSError| {
            let r = if err.is_null() { Ok(()) } else { Err((*err).localizedDescription().to_string()) };
            if let Some(tx) = t2.take() {
                let _ = tx.send(r);
            }
        });
        // One prompt covers http and https.
        ws.setDefaultApplicationAtURL_toOpenURLsWithScheme_completionHandler(&bundle, &NSString::from_str("http"), Some(&done));
        // Web pages and PDFs follow (no prompt for document types).
        for id in ["public.html", "public.xhtml", "com.adobe.pdf"] {
            if let Some(t) = objc2_uniform_type_identifiers::UTType::typeWithIdentifier(&NSString::from_str(id)) {
                ws.setDefaultApplicationAtURL_toOpenContentType_completionHandler(&bundle, &t, None);
            }
        }
        let _ = tx;
    });
    let r = tokio::time::timeout(std::time::Duration::from_secs(120), rx).await.ok().and_then(Result::ok).unwrap_or(Ok(()));
    if let Err(e) = r {
        // The user kept their current browser: not an error worth showing.
        tracing::info!("default browser not changed: {e}");
    }
    // Give LaunchServices a moment to record the choice.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    Ok(DefaultBrowserStatus { is_default: is_default(), supported: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn some_app_handles_https() {
        // Every Mac has one (Safari at least).
        assert!(default_handler_id().is_some());
    }
}
