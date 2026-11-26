//! Opening local files (PDFs above all) in tabs: from the "Open file" picker
//! (Ctrl+O), from the command line (Windows "Open with Serica" / double-click
//! once Serica is the PDF app), and from finished downloads.

use crate::app::Shared;
use std::path::Path;

/// `file:///` URL for an existing local file.
pub fn file_url(path: &Path) -> Option<String> {
    let abs = std::fs::canonicalize(path).ok()?;
    // canonicalize returns `\\?\C:\...`; Url wants a plain absolute path.
    let s = abs.to_string_lossy();
    let plain = s.strip_prefix(r"\\?\UNC\").map(|r| format!(r"\\{r}")).or_else(|| s.strip_prefix(r"\\?\").map(str::to_string)).unwrap_or_else(|| s.to_string());
    if !Path::new(&plain).is_file() {
        return None;
    }
    url::Url::from_file_path(&plain).ok().map(|u| u.to_string())
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
            // `serica-rust.exe https://…` (e.g. from another app).
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

/// Shows the "Open file" picker on its own thread (it is modal to the main
/// window) and opens the chosen files in new tabs.
#[cfg(windows)]
pub fn pick_and_open(app: &Shared) {
    use tauri::Manager;
    let owner = app.handle.get_webview_window("main").and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize).unwrap_or(0);
    let app = app.clone();
    std::thread::spawn(move || {
        let paths = unsafe { pick(owner) };
        let urls: Vec<String> = paths.iter().filter_map(|p| file_url(Path::new(p))).collect();
        for url in urls {
            app.mutate(|s| s.create_tab(Some(url), None, true, None));
        }
    });
}

#[cfg(not(windows))]
pub fn pick_and_open(_app: &Shared) {}

#[cfg(windows)]
unsafe fn pick(owner: isize) -> Vec<String> {
    pick_with(
        owner,
        &[
            ("PDF documents", "*.pdf"),
            ("Web pages and images", "*.html;*.htm;*.svg;*.txt;*.png;*.jpg;*.jpeg;*.gif;*.webp"),
            ("All files", "*.*"),
        ],
        true,
        "Open file",
    )
}

/// One picture (new-tab background).
#[cfg(windows)]
pub unsafe fn pick_one_image(owner: isize) -> Option<String> {
    pick_with(owner, &[("Pictures", "*.jpg;*.jpeg;*.png;*.webp;*.avif;*.gif;*.bmp")], false, "Choose a background picture").into_iter().next()
}

/// One file of the given kinds (password exports and the like).
#[cfg(windows)]
pub unsafe fn pick_one_file(owner: isize, filters: &[(&str, &str)], title: &str) -> Option<String> {
    pick_with(owner, filters, false, title).into_iter().next()
}

/// The system "Open" dialog, modal to `owner`.
#[cfg(windows)]
unsafe fn pick_with(owner: isize, filters: &[(&str, &str)], multi: bool, title: &str) -> Vec<String> {
    use windows::core::{HSTRING, PCWSTR, PWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, SIGDN_FILESYSPATH};

    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    let mut out = Vec::new();
    if let Ok(dialog) = CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) {
        // The strings must outlive the call that reads them.
        let held: Vec<(HSTRING, HSTRING)> = filters.iter().map(|(n, s)| (HSTRING::from(*n), HSTRING::from(*s))).collect();
        let specs: Vec<COMDLG_FILTERSPEC> = held.iter().map(|(n, s)| COMDLG_FILTERSPEC { pszName: PCWSTR(n.as_ptr()), pszSpec: PCWSTR(s.as_ptr()) }).collect();
        let _ = dialog.SetFileTypes(&specs);
        let _ = dialog.SetTitle(&HSTRING::from(title));
        if let Ok(opts) = dialog.GetOptions() {
            let _ = dialog.SetOptions(opts | FOS_FILEMUSTEXIST | if multi { FOS_ALLOWMULTISELECT } else { Default::default() });
        }
        let owner = (owner != 0).then_some(HWND(owner as _));
        if dialog.Show(owner).is_ok() {
            if let Ok(items) = dialog.GetResults() {
                let n = items.GetCount().unwrap_or(0);
                for i in 0..n {
                    let Ok(item) = items.GetItemAt(i) else { continue };
                    let Ok(p): Result<PWSTR, _> = item.GetDisplayName(SIGDN_FILESYSPATH) else { continue };
                    if let Ok(s) = p.to_string() {
                        out.push(s);
                    }
                    CoTaskMemFree(Some(p.0 as _));
                }
            }
        }
    }
    CoUninitialize();
    out
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
