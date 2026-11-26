//! Pictures for the new-tab background ("Your picture").
//!
//! The chosen file is copied into the app's data folder (so moving or
//! deleting the original doesn't break the background) and served to the UI
//! from the `sericabg` protocol: `http://sericabg.localhost/<file>`.

use std::path::{Path, PathBuf};
use tauri::Manager;

const MAX_BYTES: u64 = 40 * 1024 * 1024;
const EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "avif", "gif", "bmp"];

pub fn dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("backgrounds"))
}

fn mime(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        _ => "image/jpeg",
    }
}

/// Serves `sericabg://<file>` from the backgrounds folder (file names only).
pub fn serve(app: &tauri::AppHandle, request: &tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
    let not_found = || tauri::http::Response::builder().status(404).body(Vec::new()).unwrap_or_default();
    let name = request.uri().path().trim_start_matches('/');
    let ok_name = !name.is_empty() && name.len() < 100 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_') && !name.contains("..");
    if !ok_name {
        return not_found();
    }
    let Some(path) = dir(app).map(|d| d.join(name)) else { return not_found() };
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match std::fs::read(&path) {
        Ok(bytes) => tauri::http::Response::builder()
            .status(200)
            .header("Content-Type", mime(&ext))
            .header("Cache-Control", "max-age=31536000, immutable")
            .header("Access-Control-Allow-Origin", "*")
            .body(bytes)
            .unwrap_or_default(),
        Err(_) => not_found(),
    }
}

/// Copies `src` in as the background picture; older ones are removed.
fn adopt(app: &tauri::AppHandle, src: &Path) -> Result<String, String> {
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !EXTS.contains(&ext.as_str()) {
        return Err("not-an-image".into());
    }
    let len = std::fs::metadata(src).map_err(|e| e.to_string())?.len();
    if len == 0 || len > MAX_BYTES {
        return Err("too-large".into());
    }
    let bytes = std::fs::read(src).map_err(|e| e.to_string())?;
    use sha2::Digest;
    let hash = sha2::Sha256::digest(&bytes);
    let name = format!("bg-{}.{ext}", hash.iter().take(8).map(|b| format!("{b:02x}")).collect::<String>());
    let dir = dir(app).ok_or("no-data-dir")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    crate::store::write_atomic(&dir.join(&name), &bytes).map_err(|e| e.to_string())?;
    if let Ok(list) = std::fs::read_dir(&dir) {
        for e in list.flatten() {
            if e.file_name().to_string_lossy() != name {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    Ok(name)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Picked {
    pub file: String,
    pub url: String,
}

/// Asks for a picture and makes it the background picture. `None` when the
/// picker was cancelled.
#[tauri::command]
pub async fn pick_background_image(app: tauri::AppHandle) -> Result<Option<Picked>, String> {
    #[cfg(windows)]
    {
        let owner = app.get_webview_window("main").and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize).unwrap_or(0);
        let path = tauri::async_runtime::spawn_blocking(move || unsafe { crate::files::pick_one_image(owner) }).await.map_err(|e| e.to_string())?;
        let Some(path) = path else { return Ok(None) };
        let file = adopt(&app, Path::new(&path))?;
        Ok(Some(Picked { url: format!("http://sericabg.localhost/{file}"), file }))
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("unsupported".into())
    }
}
