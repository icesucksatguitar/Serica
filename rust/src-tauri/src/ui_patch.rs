//! Instant UI fixes ("UI patches") without a new installer.
//!
//! Most bugs live in the React UI, which is only HTML/JS/CSS. `build.py
//! --ui-patch` builds just that (seconds, no cargo), zips it, signs the zip
//! with the same key as the installer, and puts it on the GitHub release with
//! `ui-latest.json`. Installed copies check that file shortly after launch and
//! every 30 minutes. A patch is taken only when:
//!
//! * its signature verifies against the updater public key in tauri.conf.json,
//! * it was built for exactly this app version (the UI calls Rust commands, so
//!   a UI built for another binary could break), checked from the metadata
//!   inside the signed zip, not just the unsigned manifest,
//! * its build number is newer than the one in use and not known to be bad.
//!
//! It is unpacked to `%LOCALAPPDATA%\com.serica.browser\ui-patch\staged`, then
//! the UI offers "Reload", or it's picked up on the next launch. Files are
//! served from the patch folder, falling back to the UI embedded in the exe.
//!
//! Safety net: launching with a patch writes a `booting` marker, which the UI
//! removes once it has rendered (`ui_patch_confirm`). If the marker is still
//! there on the next launch, the patch broke the UI: it is dropped (and never
//! downloaded again) and Serica runs on its built-in UI. A new installer
//! (new version) ignores and deletes patches made for the old one.

use crate::app::ui_emit;
use base64::Engine;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use tauri::utils::assets::{AssetKey, AssetsIter, CspHash};
use tauri::{AppHandle, Assets, Runtime};

const MANIFEST_URL: &str = "https://github.com/icesucksatguitar/Serica/releases/latest/download/ui-latest.json";
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(15);
const CHECK_INTERVAL: Duration = Duration::from_secs(30 * 60);
const EVENT: &str = "serica:ui-patch";
/// Written into the zip by build.py; covered by the signature.
const META_FILE: &str = "serica-ui.json";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Meta {
    app_version: String,
    build: u64,
    #[serde(default)]
    notes: String,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    app_version: String,
    build: u64,
    url: String,
    signature: String,
    #[serde(default)]
    notes: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UiPatchStatus {
    /// Build of the patch the UI is running on (0: the built-in UI).
    pub active_build: u64,
    /// A newer patch, downloaded and waiting for a reload.
    pub ready_build: Option<u64>,
    pub notes: Option<String>,
}

fn root() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join("com.serica.browser").join("ui-patch"))
}

/// Folder the UI is currently served from, if a patch is active.
fn active_dir() -> &'static RwLock<Option<PathBuf>> {
    static D: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();
    D.get_or_init(|| RwLock::new(None))
}

fn status_cell() -> &'static Mutex<UiPatchStatus> {
    static S: OnceLock<Mutex<UiPatchStatus>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(UiPatchStatus::default()))
}

fn read_meta(dir: &Path) -> Option<Meta> {
    serde_json::from_slice(&std::fs::read(dir.join(META_FILE)).ok()?).ok()
}

fn bad_builds(root: &Path) -> Vec<u64> {
    std::fs::read_to_string(root.join("bad.txt"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

fn mark_bad(root: &Path, build: u64) {
    let mut list = bad_builds(root);
    if !list.contains(&build) {
        list.push(build);
    }
    let text: Vec<String> = list.iter().map(|b| b.to_string()).collect();
    let _ = std::fs::write(root.join("bad.txt"), text.join("\n"));
}

/// Usable patch in `dir` for this binary?
fn valid(dir: &Path, root: &Path) -> Option<Meta> {
    let meta = read_meta(dir)?;
    (meta.app_version == APP_VERSION && !bad_builds(root).contains(&meta.build) && dir.join("index.html").is_file())
        .then_some(meta)
}

/// Runs before the app is built: picks the patch to serve (promoting a staged
/// one), applies the safety net, and wraps the embedded assets.
pub fn install<R: Runtime>(context: &mut tauri::Context<R>) {
    if cfg!(debug_assertions) {
        return;
    }
    let Some(root) = root() else { return };
    let active = root.join("active");
    let staged = root.join("staged");
    let booting = root.join("booting");

    // The last launch with the active patch never rendered: drop it.
    if booting.exists() {
        if let Some(meta) = read_meta(&active) {
            tracing::warn!("UI patch {} did not start; using the built-in UI", meta.build);
            mark_bad(&root, meta.build);
        }
        let _ = std::fs::remove_dir_all(&active);
        let _ = std::fs::remove_file(&booting);
    }
    if valid(&staged, &root).is_some() {
        let _ = std::fs::remove_dir_all(&active);
        if std::fs::rename(&staged, &active).is_err() {
            let _ = std::fs::remove_dir_all(&staged);
        }
    } else {
        let _ = std::fs::remove_dir_all(&staged);
    }
    match valid(&active, &root) {
        Some(meta) => {
            tracing::info!("serving UI patch {} for {APP_VERSION}", meta.build);
            let _ = std::fs::write(&booting, b"");
            *active_dir().write() = Some(active);
            status_cell().lock().active_build = meta.build;
        }
        // Made for an older install, or broken: the built-in UI is newer.
        None => {
            let _ = std::fs::remove_dir_all(&active);
        }
    }
    let embedded = context.set_assets(Box::new(NoAssets));
    context.set_assets(Box::new(PatchedAssets { embedded }));
}

/// Placeholder while swapping the asset provider.
struct NoAssets;
impl<R: Runtime> Assets<R> for NoAssets {
    fn get(&self, _: &AssetKey) -> Option<Cow<'_, [u8]>> {
        None
    }
    fn iter(&self) -> Box<AssetsIter<'_>> {
        Box::new(std::iter::empty())
    }
    fn csp_hashes(&self, _: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        Box::new(std::iter::empty())
    }
}

struct PatchedAssets<R: Runtime> {
    embedded: Box<dyn Assets<R>>,
}

impl<R: Runtime> Assets<R> for PatchedAssets<R> {
    fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
        if let Some(dir) = active_dir().read().as_ref() {
            let rel = Path::new(key.as_ref().trim_start_matches('/'));
            let safe = rel.components().all(|c| matches!(c, Component::Normal(_)));
            if safe {
                if let Ok(bytes) = std::fs::read(dir.join(rel)) {
                    return Some(Cow::Owned(bytes));
                }
            }
        }
        self.embedded.get(key)
    }
    fn iter(&self) -> Box<AssetsIter<'_>> {
        self.embedded.iter()
    }
    fn csp_hashes(&self, html_path: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        self.embedded.csp_hashes(html_path)
    }
}

pub fn status() -> UiPatchStatus {
    status_cell().lock().clone()
}

/// The UI rendered: the active patch works.
pub fn confirm() {
    if let Some(root) = root() {
        let _ = std::fs::remove_file(root.join("booting"));
    }
}

/// Switches to the downloaded patch now; the UI reloads itself after this.
pub fn apply() -> Result<(), String> {
    let root = root().ok_or("no data folder")?;
    let (active, staged) = (root.join("active"), root.join("staged"));
    let meta = valid(&staged, &root).ok_or("No fix has been downloaded")?;
    // Release the old folder before replacing it.
    *active_dir().write() = None;
    let _ = std::fs::remove_dir_all(&active);
    std::fs::rename(&staged, &active).map_err(|e| e.to_string())?;
    std::fs::write(root.join("booting"), b"").map_err(|e| e.to_string())?;
    *active_dir().write() = Some(active);
    let mut s = status_cell().lock();
    s.active_build = meta.build;
    s.ready_build = None;
    Ok(())
}

pub fn start(handle: AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_DELAY).await;
        loop {
            if let Err(e) = check(&handle).await {
                tracing::warn!("UI patch check failed: {e}");
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

async fn check(handle: &AppHandle) -> Result<(), String> {
    let root = root().ok_or("no data folder")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(format!("Serica/{APP_VERSION}"))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(MANIFEST_URL).send().await.map_err(|e| e.to_string())?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(()); // no patch published
    }
    let manifest: Manifest = resp.error_for_status().map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    let current = {
        let s = status_cell().lock();
        s.ready_build.unwrap_or(s.active_build)
    };
    if manifest.app_version != APP_VERSION || manifest.build <= current || bad_builds(&root).contains(&manifest.build) {
        return Ok(());
    }

    let zip = client
        .get(&manifest.url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    verify(handle, &zip, &manifest.signature)?;

    let tmp = root.join("staging");
    let _ = std::fs::remove_dir_all(&tmp);
    extract(&zip, &tmp)?;
    // Trust only the metadata the signature covers.
    let meta = read_meta(&tmp).ok_or("patch has no metadata")?;
    if meta.app_version != APP_VERSION || meta.build != manifest.build {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err("patch metadata does not match the manifest".into());
    }
    let staged = root.join("staged");
    let _ = std::fs::remove_dir_all(&staged);
    std::fs::rename(&tmp, &staged).map_err(|e| e.to_string())?;
    tracing::info!("UI patch {} downloaded", meta.build);

    let snapshot = {
        let mut s = status_cell().lock();
        s.ready_build = Some(meta.build);
        s.notes = Some(if meta.notes.is_empty() { manifest.notes } else { meta.notes }).filter(|n| !n.is_empty());
        s.clone()
    };
    ui_emit(handle, EVENT, snapshot);
    Ok(())
}

/// Same check the installer updater does: minisign signature by our key.
fn verify(handle: &AppHandle, data: &[u8], signature_b64: &str) -> Result<(), String> {
    let pubkey_b64 = handle
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u.get("pubkey"))
        .and_then(|k| k.as_str())
        .ok_or("no updater public key")?;
    verify_with(pubkey_b64, data, signature_b64)
}

fn verify_with(pubkey_b64: &str, data: &[u8], signature_b64: &str) -> Result<(), String> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let pubkey = String::from_utf8(b64.decode(pubkey_b64).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let sig = String::from_utf8(b64.decode(signature_b64.trim()).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let pk = minisign_verify::PublicKey::decode(&pubkey).map_err(|e| e.to_string())?;
    let sig = minisign_verify::Signature::decode(&sig).map_err(|e| e.to_string())?;
    pk.verify(data, &sig, false).map_err(|e| format!("bad signature: {e}"))
}

fn extract(zip: &[u8], dest: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = file.enclosed_name() else { continue };
        let out = dest.join(rel);
        if file.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut buf = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        std::fs::write(&out, buf).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checks a patch made by `build.py --ui-patch` against the real key:
    /// UI_PATCH_TEST=release/ui-patch cargo test --lib ui_patch
    #[test]
    fn verifies_and_extracts_built_patch() {
        let Some(dir) = std::env::var_os("UI_PATCH_TEST").map(PathBuf::from) else { return };
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let pubkey = conf["plugins"]["updater"]["pubkey"].as_str().unwrap();
        let manifest: Manifest = serde_json::from_slice(&std::fs::read(dir.join("ui-latest.json")).unwrap()).unwrap();
        let name = manifest.url.rsplit('/').next().unwrap();
        let mut zip = std::fs::read(dir.join(name)).unwrap();
        verify_with(pubkey, &zip, &manifest.signature).expect("signature");

        let out = std::env::temp_dir().join("serica-ui-patch-test");
        let _ = std::fs::remove_dir_all(&out);
        extract(&zip, &out).unwrap();
        let meta = read_meta(&out).unwrap();
        assert_eq!(meta.build, manifest.build);
        assert_eq!(meta.app_version, manifest.app_version);
        assert!(out.join("index.html").is_file());

        // One flipped byte must be rejected.
        let last = zip.len() - 1;
        zip[last] ^= 1;
        assert!(verify_with(pubkey, &zip, &manifest.signature).is_err());
    }

    struct Embedded;
    impl<R: Runtime> Assets<R> for Embedded {
        fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
            Some(Cow::Owned(format!("embedded{}", key.as_ref()).into_bytes()))
        }
        fn iter(&self) -> Box<AssetsIter<'_>> {
            Box::new(std::iter::empty())
        }
        fn csp_hashes(&self, _: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
            Box::new(std::iter::empty())
        }
    }

    #[test]
    fn serves_patch_files_then_falls_back() {
        let dir = std::env::temp_dir().join("serica-ui-patch-serve");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("index.html"), "patched index").unwrap();
        std::fs::write(dir.join("assets/app.js"), "patched js").unwrap();
        std::fs::write(std::env::temp_dir().join("secret.txt"), "secret").unwrap();
        *active_dir().write() = Some(dir.clone());

        let a = PatchedAssets::<tauri::Wry> { embedded: Box::new(Embedded) };
        let get = |k: &str| String::from_utf8(a.get(&AssetKey::from(k)).unwrap().into_owned()).unwrap();
        assert_eq!(get("index.html"), "patched index");
        assert_eq!(get("/assets/app.js"), "patched js");
        assert_eq!(get("/assets/missing.js"), "embedded/assets/missing.js");
        // Never reads outside the patch folder.
        assert_eq!(get("/../secret.txt"), "embedded/../secret.txt");

        *active_dir().write() = None;
        assert_eq!(get("index.html"), "embedded/index.html");
    }
}
