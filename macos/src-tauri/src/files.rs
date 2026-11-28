//! Opening local files (PDFs above all) in tabs: from the "Open File" panel
//! (Command-O), from Finder ("Open With", double-click once Serica is the PDF
//! app, a file dropped on the Dock icon), from links other apps hand over
//! when Serica is the default browser, and from finished downloads.
//!
//! The open and save panels are sheets on the main window: the UI keeps
//! running while they are up, and nothing blocks the async runtime.

use crate::app::Shared;
use crate::mac_util::post_main;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSModalResponseOK, NSOpenPanel, NSSavePanel};
use objc2_foundation::{NSArray, NSString};
use std::cell::Cell;
use std::path::{Path, PathBuf};

/// `file:///` URL for an existing local file.
pub fn file_url(path: &Path) -> Option<String> {
    let abs = std::fs::canonicalize(path).ok()?;
    if !abs.is_file() {
        return None;
    }
    url::Url::from_file_path(&abs).ok().map(|u| u.to_string())
}

/// Opens each existing file among `args` (program arguments) in a new tab.
/// Returns how many were opened.
pub fn open_args<I: IntoIterator<Item = String>>(app: &Shared, args: I) -> usize {
    let urls: Vec<String> = args
        .into_iter()
        .filter(|a| !a.starts_with('-'))
        .filter_map(|a| {
            let p = Path::new(&a);
            if p.is_file() {
                return file_url(p);
            }
            // `Serica https://…` (e.g. from another app or a script).
            let lower = a.to_ascii_lowercase();
            (lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("file:///")).then_some(a)
        })
        .collect();
    let n = urls.len();
    for url in urls {
        app.mutate(|s| s.create_tab(Some(url), None, true, None));
    }
    n
}

/// URLs macOS hands Serica (`application:openURLs:`): web links when it is
/// the default browser, and files opened with it.
pub fn open_urls<I: IntoIterator<Item = String>>(app: &Shared, urls: I) -> usize {
    let list: Vec<String> = urls
        .into_iter()
        .filter_map(|u| {
            let parsed = url::Url::parse(&u).ok()?;
            match parsed.scheme() {
                "http" | "https" => Some(parsed.to_string()),
                "file" => parsed.to_file_path().ok().and_then(|p| file_url(&p)),
                _ => None,
            }
        })
        .collect();
    let n = list.len();
    for url in list {
        app.mutate(|s| s.create_tab(Some(url), None, true, None));
    }
    n
}

/// A file type filter: content types by file extension.
fn content_types(exts: &[&str]) -> Option<Retained<NSArray<objc2_uniform_type_identifiers::UTType>>> {
    use objc2_uniform_type_identifiers::UTType;
    let types: Vec<Retained<UTType>> = exts.iter().filter_map(|e| UTType::typeWithFilenameExtension(&NSString::from_str(e))).collect();
    (!types.is_empty()).then(|| NSArray::from_retained_slice(&types))
}

/// The "Open" panel as a sheet on the main window; resolves with the chosen
/// paths (empty when cancelled).
async fn open_panel(app: &Shared, exts: Vec<&'static str>, multi: bool, title: &'static str) -> Vec<PathBuf> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Vec<PathBuf>>();
    let handle = app.handle.clone();
    post_main(&app.handle, move || {
        let mtm = MainThreadMarker::new().expect("main thread");
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(multi);
        panel.setMessage(Some(&NSString::from_str(title)));
        if let Some(types) = content_types(&exts) {
            panel.setAllowedContentTypes(&types);
        }
        let tx = Cell::new(Some(tx));
        let p2 = panel.clone();
        let done = RcBlock::new(move |response: isize| {
            let mut out = Vec::new();
            if response == NSModalResponseOK {
                for u in p2.URLs().iter() {
                    if let Some(path) = u.path() {
                        out.push(PathBuf::from(path.to_string()));
                    }
                }
            }
            if let Some(tx) = tx.take() {
                let _ = tx.send(out);
            }
        });
        let window = tauri::Manager::get_webview_window(&handle, "main").and_then(|w| w.ns_window().ok());
        match window {
            Some(ptr) if !ptr.is_null() => {
                // SAFETY: Tauri's live NSWindow, on the main thread.
                let w: &objc2_app_kit::NSWindow = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
                panel.beginSheetModalForWindow_completionHandler(w, &done);
            }
            _ => panel.beginWithCompletionHandler(&done),
        }
    });
    rx.await.unwrap_or_default()
}

/// Shows the "Open File" panel and opens the chosen files in new tabs.
pub fn pick_and_open(app: &Shared) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let paths = open_panel(&app, Vec::new(), true, "Open a PDF, web page or picture in Serica").await;
        let urls: Vec<String> = paths.iter().filter_map(|p| file_url(p)).collect();
        for url in urls {
            app.mutate(|s| s.create_tab(Some(url), None, true, None));
        }
    });
}

/// One picture (new-tab background).
pub async fn pick_one_image(app: &Shared) -> Option<PathBuf> {
    open_panel(app, vec!["jpg", "jpeg", "png", "webp", "avif", "gif", "bmp", "heic"], false, "Choose a background picture").await.into_iter().next()
}

/// One file of the given kinds (password exports and the like).
pub async fn pick_one_file(app: &Shared, exts: Vec<&'static str>, title: &'static str) -> Option<PathBuf> {
    open_panel(app, exts, false, title).await.into_iter().next()
}

/// The "Save" panel as a sheet on the main window.
pub async fn pick_save(app: &Shared, name: &str, _kind: &str, exts: &[&str]) -> Option<PathBuf> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<PathBuf>>();
    let (name, exts): (String, Vec<String>) = (name.to_string(), exts.iter().map(|e| e.to_string()).collect());
    let handle = app.handle.clone();
    post_main(&app.handle, move || {
        let mtm = MainThreadMarker::new().expect("main thread");
        let panel = NSSavePanel::savePanel(mtm);
        panel.setNameFieldStringValue(&NSString::from_str(&name));
        panel.setCanCreateDirectories(true);
        let ext_refs: Vec<&str> = exts.iter().map(String::as_str).collect();
        if let Some(types) = content_types(&ext_refs) {
            panel.setAllowedContentTypes(&types);
        }
        let tx = Cell::new(Some(tx));
        let p2 = panel.clone();
        let done = RcBlock::new(move |response: isize| {
            let out = (response == NSModalResponseOK).then(|| p2.URL()).flatten().and_then(|u| u.path()).map(|p| PathBuf::from(p.to_string()));
            if let Some(tx) = tx.take() {
                let _ = tx.send(out);
            }
        });
        let window = tauri::Manager::get_webview_window(&handle, "main").and_then(|w| w.ns_window().ok());
        match window {
            Some(ptr) if !ptr.is_null() => {
                let w: &objc2_app_kit::NSWindow = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
                panel.beginSheetModalForWindow_completionHandler(w, &done);
            }
            _ => panel.beginWithCompletionHandler(&done),
        }
    });
    rx.await.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls() {
        let dir = std::env::temp_dir().join("serica files test");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a b#.pdf");
        std::fs::write(&f, b"%PDF-1.4").unwrap();
        let u = file_url(&f).unwrap();
        assert!(u.starts_with("file:///"), "{u}");
        assert!(u.ends_with("/serica%20files%20test/a%20b%23.pdf"), "{u}");
        assert!(file_url(&dir.join("missing.pdf")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
