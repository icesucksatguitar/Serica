//! Browser extensions: installed from the Chrome Web Store (or by id),
//! unpacked under `<data>/extensions/<id>/` and loaded into every tab profile
//! through WebView2's profile extension API.
//!
//! The registry (`extensions.json`) is the source of truth. Each WebView2
//! profile is synced against it the first time a tab of that profile is
//! created in a session, and again whenever the registry changes: missing
//! extensions are added, enabled state is applied, and extensions that are no
//! longer in the registry are removed.

use crate::app::{ui_emit, Shared};
use crate::store::{read_json, write_atomic};
use base64::Engine as _;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Chrome version reported to the update server (it serves the newest CRX
/// compatible with this version).
const PROD_VERSION: &str = "131.0.0.0";
const MAX_CRX_BYTES: usize = 200 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionRecord {
    pub id: String,
    /// Id WebView2 assigned when it differs from the store id (no `key`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub enabled: bool,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub popup: Option<String>,
    #[serde(default)]
    pub options: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    pub source: String,
    pub installed_at: i64,
}

impl ExtensionRecord {
    pub fn live_id(&self) -> &str {
        self.runtime_id.as_deref().unwrap_or(&self.id)
    }
    pub fn page_url(&self, page: &str) -> Option<String> {
        let rel = match page {
            "popup" => self.popup.as_deref()?,
            "options" => self.options.as_deref()?,
            _ => return None,
        };
        Some(format!("chrome-extension://{}/{}", self.live_id(), rel.trim_start_matches('/')))
    }
}

pub struct Extensions {
    dir: PathBuf,
    registry: Mutex<Vec<ExtensionRecord>>,
    /// Profiles synced against the registry this session.
    synced: Mutex<HashSet<String>>,
    installing: Mutex<HashSet<String>>,
}

/// True for a 32-char Chrome extension id (`a`-`p`).
pub fn is_extension_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

/// Extracts an extension id from a bare id or a Chrome Web Store URL.
pub fn parse_extension_id(input: &str) -> Option<String> {
    let t = input.trim().to_ascii_lowercase();
    if is_extension_id(&t) {
        return Some(t);
    }
    let u = url::Url::parse(&t).ok()?;
    let host = u.host_str()?;
    if host != "chromewebstore.google.com" && host != "chrome.google.com" {
        return None;
    }
    u.path_segments()?.find(|seg| is_extension_id(seg)).map(str::to_string)
}

/// Chrome's id derivation: first 16 bytes of SHA-256(public key), one `a`-`p` letter per nibble.
fn id_from_key(key: &[u8]) -> String {
    Sha256::digest(key)[..16].iter().flat_map(|b| [b >> 4, b & 0xf]).map(|n| (b'a' + n) as char).collect()
}

fn read_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *buf.get(*pos)?;
        *pos += 1;
        v |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

/// Length-delimited fields `(number, bytes)` of a protobuf message; stops at malformed data.
fn proto_fields(buf: &[u8]) -> Vec<(u64, &[u8])> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < buf.len() {
        let Some(tag) = read_varint(buf, &mut pos) else { break };
        match tag & 7 {
            0 => {
                if read_varint(buf, &mut pos).is_none() {
                    break;
                }
            }
            1 => pos += 8,
            5 => pos += 4,
            2 => {
                let Some(len) = read_varint(buf, &mut pos) else { break };
                let end = pos.saturating_add(len as usize);
                if end > buf.len() {
                    break;
                }
                out.push((tag >> 3, &buf[pos..end]));
                pos = end;
            }
            _ => break,
        }
    }
    out
}

/// Splits a CRX (v2 or v3) into its ZIP payload and the public keys it was signed with.
pub fn parse_crx(bytes: &[u8]) -> Result<(&[u8], Vec<Vec<u8>>), String> {
    let u32_at = |o: usize| bytes.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    if bytes.starts_with(b"PK\x03\x04") {
        return Ok((bytes, Vec::new()));
    }
    if !bytes.starts_with(b"Cr24") {
        return Err("not a CRX package".into());
    }
    match u32_at(4) {
        Some(3) => {
            let hl = u32_at(8).ok_or("truncated CRX header")?;
            let header = bytes.get(12..12 + hl).ok_or("truncated CRX header")?;
            let zip = &bytes[12 + hl..];
            // CrxFileHeader: sha256_with_rsa = 2, sha256_with_ecdsa = 3; AsymmetricKeyProof.public_key = 1.
            let keys = proto_fields(header)
                .into_iter()
                .filter(|(n, _)| *n == 2 || *n == 3)
                .flat_map(|(_, proof)| proto_fields(proof).into_iter().filter(|(n, _)| *n == 1).map(|(_, k)| k.to_vec()))
                .collect();
            Ok((zip, keys))
        }
        Some(2) => {
            let kl = u32_at(8).ok_or("truncated CRX header")?;
            let sl = u32_at(12).ok_or("truncated CRX header")?;
            let key = bytes.get(16..16 + kl).ok_or("truncated CRX header")?.to_vec();
            let zip = bytes.get(16 + kl + sl..).ok_or("truncated CRX header")?;
            Ok((zip, vec![key]))
        }
        _ => Err("unsupported CRX version".into()),
    }
}

fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

fn read_manifest(dir: &Path) -> Result<serde_json::Value, String> {
    let raw = std::fs::read_to_string(dir.join("manifest.json")).map_err(|_| "extension has no manifest.json".to_string())?;
    serde_json::from_str(strip_bom(&raw)).map_err(|e| format!("invalid manifest.json: {e}"))
}

/// Resolves `__MSG_name__` strings against `_locales/<default_locale>/messages.json`.
fn localize(dir: &Path, manifest: &serde_json::Value, s: &str) -> String {
    let Some(key) = s.strip_prefix("__MSG_").and_then(|k| k.strip_suffix("__")) else { return s.to_string() };
    let locales: Vec<String> = [manifest["default_locale"].as_str().unwrap_or("en"), "en", "en_US"].iter().map(|l| l.to_string()).collect();
    for loc in locales {
        let Ok(raw) = std::fs::read_to_string(dir.join("_locales").join(&loc).join("messages.json")) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(strip_bom(&raw)) else { continue };
        if let Some(obj) = v.as_object() {
            if let Some((_, m)) = obj.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
                if let Some(msg) = m["message"].as_str() {
                    return msg.to_string();
                }
            }
        }
    }
    key.to_string()
}

/// Largest icon up to 128px, as a data URL.
fn icon_data_url(dir: &Path, manifest: &serde_json::Value) -> Option<String> {
    let icons = manifest["icons"].as_object()?;
    let mut sizes: Vec<(u32, &str)> = icons.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.as_str()?))).collect();
    sizes.sort_by_key(|(s, _)| *s);
    let (_, rel) = sizes.iter().rev().find(|(s, _)| *s <= 128).or(sizes.first())?;
    let path = dir.join(rel.trim_start_matches('/'));
    let bytes = std::fs::read(&path).ok()?;
    if bytes.len() > 512 * 1024 {
        return None;
    }
    let mime = match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "svg" => "image/svg+xml",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    };
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// Unpacks a ZIP into `dest`, refusing entries that escape it.
fn unzip(zip_bytes: &[u8], dest: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| format!("corrupt extension package: {e}"))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("corrupt extension package: {e}"))?;
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut data = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut data).map_err(|e| format!("corrupt extension package: {e}"))?;
        std::fs::write(&out, data).map_err(|e| e.to_string())?;
    }
    Ok(())
}

impl Extensions {
    pub fn new(dir: PathBuf) -> Self {
        let registry: Vec<ExtensionRecord> =
            read_json(&dir.join("extensions.json")).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        let ext = Self { dir, registry: Mutex::new(registry), synced: Mutex::new(HashSet::new()), installing: Mutex::new(HashSet::new()) };
        ext.cleanup_orphans();
        ext
    }

    fn save(&self) {
        let list = self.registry.lock().clone();
        if let Ok(bytes) = serde_json::to_vec_pretty(&list) {
            if let Err(e) = write_atomic(&self.dir.join("extensions.json"), &bytes) {
                tracing::warn!("failed to save extensions: {e}");
            }
        }
    }

    /// Removes unpacked folders left behind by uninstalled extensions.
    fn cleanup_orphans(&self) {
        let keep: HashSet<String> = self.registry.lock().iter().map(|r| r.id.clone()).collect();
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if e.path().is_dir() && !keep.contains(&name) {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }

    pub fn list(&self) -> Vec<ExtensionRecord> {
        let mut list = self.registry.lock().clone();
        list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        list
    }

    pub fn get(&self, id: &str) -> Option<ExtensionRecord> {
        self.registry.lock().iter().find(|r| r.id == id).cloned()
    }

    pub fn is_installed(&self, id: &str) -> bool {
        self.registry.lock().iter().any(|r| r.id == id)
    }

    pub fn path_of(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    /// Downloads, unpacks and registers an extension. Profiles are synced by the caller.
    pub async fn install(&self, id: &str) -> Result<ExtensionRecord, String> {
        if !is_extension_id(id) {
            return Err("invalid extension id".into());
        }
        if !self.installing.lock().insert(id.to_string()) {
            return Err("already installing".into());
        }
        let result = self.install_inner(id).await;
        self.installing.lock().remove(id);
        result
    }

    async fn install_inner(&self, id: &str) -> Result<ExtensionRecord, String> {
        let url = format!(
            "https://clients2.google.com/service/update2/crx?response=redirect&prodversion={PROD_VERSION}&acceptformat=crx2,crx3&x=id%3D{id}%26uc"
        );
        let client = reqwest::Client::builder()
            .user_agent(format!("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{PROD_VERSION} Safari/537.36"))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client.get(&url).send().await.map_err(|e| format!("download failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("download failed (HTTP {})", resp.status().as_u16()));
        }
        let bytes = resp.bytes().await.map_err(|e| format!("download failed: {e}"))?;
        if bytes.is_empty() {
            return Err("extension not found or not available for this browser".into());
        }
        if bytes.len() > MAX_CRX_BYTES {
            return Err("extension package is too large".into());
        }
        let dir = self.dir.clone();
        let id2 = id.to_string();
        let record = tokio::task::spawn_blocking(move || unpack(&dir, &id2, &bytes)).await.map_err(|e| e.to_string())??;
        {
            let mut reg = self.registry.lock();
            reg.retain(|r| r.id != record.id);
            reg.push(record.clone());
        }
        self.save();
        Ok(record)
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> bool {
        let changed = {
            let mut reg = self.registry.lock();
            match reg.iter_mut().find(|r| r.id == id) {
                Some(r) => {
                    r.enabled = enabled;
                    true
                }
                None => false,
            }
        };
        if changed {
            self.save();
        }
        changed
    }

    pub fn remove(&self, id: &str) -> bool {
        let removed = {
            let mut reg = self.registry.lock();
            let before = reg.len();
            reg.retain(|r| r.id != id);
            reg.len() != before
        };
        if removed {
            self.save();
        }
        removed
    }

    fn set_runtime_id(&self, id: &str, runtime: &str) {
        let changed = {
            let mut reg = self.registry.lock();
            match reg.iter_mut().find(|r| r.id == id) {
                Some(r) if r.live_id() != runtime => {
                    r.runtime_id = if runtime == r.id { None } else { Some(runtime.to_string()) };
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.save();
        }
    }

    /// Marks `profile` as synced; false if it already was this session.
    pub fn claim_sync(&self, profile: &str) -> bool {
        self.synced.lock().insert(profile.to_string())
    }

    pub fn reset_sync(&self) {
        self.synced.lock().clear();
    }
}

/// Unpacks a CRX into `<dir>/<id>` and reads its manifest.
fn unpack(dir: &Path, id: &str, crx: &[u8]) -> Result<ExtensionRecord, String> {
    let (zip, keys) = parse_crx(crx)?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let staging = dir.join(format!(".{id}.tmp"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let result = (|| {
        unzip(zip, &staging)?;
        // Chrome refuses to load unpacked extensions with reserved `_metadata` (store signatures).
        let _ = std::fs::remove_dir_all(staging.join("_metadata"));
        let mut manifest = read_manifest(&staging)?;
        // The store key keeps the extension's id stable (the id is otherwise derived from the path).
        if let Some(key) = keys.iter().find(|k| id_from_key(k) == id) {
            if manifest.get("key").is_none() {
                manifest["key"] = serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(key));
                let bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
                std::fs::write(staging.join("manifest.json"), bytes).map_err(|e| e.to_string())?;
            }
        }
        Ok::<_, String>(manifest)
    })();
    let manifest = match result {
        Ok(m) => m,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    let target = dir.join(id);
    let _ = std::fs::remove_dir_all(&target);
    std::fs::rename(&staging, &target).map_err(|e| format!("could not install extension files: {e}"))?;
    Ok(record_from_manifest(&target, id, &manifest))
}

fn record_from_manifest(dir: &Path, id: &str, m: &serde_json::Value) -> ExtensionRecord {
    let s = |v: &serde_json::Value| v.as_str().map(|x| localize(dir, m, x));
    let popup = m["action"]["default_popup"].as_str().or(m["browser_action"]["default_popup"].as_str()).filter(|p| !p.is_empty());
    let options = m["options_ui"]["page"].as_str().or(m["options_page"].as_str()).filter(|p| !p.is_empty());
    let mut permissions: Vec<String> = ["permissions", "host_permissions"]
        .iter()
        .flat_map(|k| m[*k].as_array().cloned().unwrap_or_default())
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    permissions.dedup();
    ExtensionRecord {
        id: id.to_string(),
        runtime_id: None,
        name: s(&m["name"]).unwrap_or_else(|| id.to_string()),
        version: m["version"].as_str().unwrap_or("").to_string(),
        description: s(&m["description"]).unwrap_or_default(),
        enabled: true,
        icon: icon_data_url(dir, m),
        popup: popup.map(str::to_string),
        options: options.map(str::to_string),
        permissions,
        source: "webstore".into(),
        installed_at: chrono::Utc::now().timestamp_millis(),
    }
}

/// Broadcasts the list and re-syncs every profile that has a live tab.
pub fn refresh(app: &Shared) {
    ui_emit(&app.handle, "serica:extensions-updated", app.extensions.list());
    app.extensions.reset_sync();
    #[cfg(windows)]
    {
        let targets: Vec<(String, tauri::Webview)> = {
            let host = app.host.lock();
            let mut seen = HashSet::new();
            host.views.values().filter(|v| seen.insert(v.profile.clone())).map(|v| (v.profile.clone(), v.webview.clone())).collect()
        };
        for (profile, wv) in targets {
            if app.extensions.claim_sync(&profile) {
                let app = app.clone();
                let _ = wv.with_webview(move |pw| unsafe {
                    if let Ok(core) = pw.controller().CoreWebView2() {
                        native::sync(&app, &core);
                    }
                });
            }
        }
    }
}

#[cfg(windows)]
pub mod native {
    use super::*;
    use std::collections::HashMap;
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use webview2_com::*;
    use windows::core::{Interface, HSTRING, PWSTR};

    fn take(p: PWSTR) -> String {
        if p.is_null() {
            return String::new();
        }
        let s = unsafe { p.to_string() }.unwrap_or_default();
        unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _)) };
        s
    }

    /// Brings the webview's profile in line with the registry. UI thread only.
    pub unsafe fn sync(app: &Shared, core: &ICoreWebView2) {
        let Ok(profile) = core.cast::<ICoreWebView2_13>().and_then(|c| c.Profile()).and_then(|p| p.cast::<ICoreWebView2Profile7>()) else {
            tracing::warn!("WebView2 runtime does not support extensions");
            return;
        };
        let app = app.clone();
        let p2 = profile.clone();
        let handler = ProfileGetBrowserExtensionsCompletedHandler::create(Box::new(move |hr, list| {
            if hr.is_err() {
                return Ok(());
            }
            let mut installed: HashMap<String, (ICoreWebView2BrowserExtension, bool)> = HashMap::new();
            if let Some(list) = list {
                let mut count = 0u32;
                list.Count(&mut count)?;
                for i in 0..count {
                    let ext = list.GetValueAtIndex(i)?;
                    let mut id = PWSTR::null();
                    ext.Id(&mut id)?;
                    let mut on = Default::default();
                    ext.IsEnabled(&mut on)?;
                    installed.insert(take(id), (ext, on.as_bool()));
                }
            }
            for rec in app.extensions.list() {
                let found = installed.remove(rec.live_id()).or_else(|| installed.remove(&rec.id));
                match found {
                    Some((ext, on)) => {
                        if on != rec.enabled {
                            let _ = ext.Enable(rec.enabled, &BrowserExtensionEnableCompletedHandler::create(Box::new(|_| Ok(()))));
                        }
                    }
                    None => {
                        let path = app.extensions.path_of(&rec.id);
                        if !path.join("manifest.json").exists() {
                            continue;
                        }
                        let (app2, rec2) = (app.clone(), rec.clone());
                        let done = ProfileAddBrowserExtensionCompletedHandler::create(Box::new(move |hr, ext| {
                            let Some(ext) = ext.filter(|_| hr.is_ok()) else {
                                tracing::warn!("failed to load extension {}: {hr:?}", rec2.id);
                                return Ok(());
                            };
                            let mut id = PWSTR::null();
                            if ext.Id(&mut id).is_ok() {
                                app2.extensions.set_runtime_id(&rec2.id, &take(id));
                            }
                            if !rec2.enabled {
                                let _ = ext.Enable(false, &BrowserExtensionEnableCompletedHandler::create(Box::new(|_| Ok(()))));
                            }
                            Ok(())
                        }));
                        let _ = p2.AddBrowserExtension(&HSTRING::from(path.as_path()), &done);
                    }
                }
            }
            for (_, (ext, _)) in installed {
                let _ = ext.Remove(&BrowserExtensionRemoveCompletedHandler::create(Box::new(|_| Ok(()))));
            }
            Ok(())
        }));
        let _ = profile.GetBrowserExtensions(&handler);
    }
}

/// Runs in every tab's top frame; only acts on Chrome Web Store detail pages,
/// where it adds an "Add to Serica" button (the store's own button needs Chrome).
pub const WEBSTORE_SCRIPT: &str = r#"(() => {
  if (window.top !== window || location.hostname !== 'chromewebstore.google.com' || window.__sericaStore) return;
  const post = (m) => { try { window.chrome.webview.postMessage(JSON.stringify(Object.assign({ __serica: 1 }, m))); } catch (e) {} };
  const states = {};
  const errors = {};
  let host = null, btn = null, note = null, current = null;
  const idFromPath = () => { const m = location.pathname.match(/\/detail\/(?:[^/]+\/)?([a-p]{32})(?:[/?#]|$)/); return m ? m[1] : null; };
  const labels = { idle: 'Add to Serica', installing: 'Adding…', installed: 'Added to Serica', error: 'Try again' };
  const ensure = () => {
    if (host && host.isConnected) return;
    host = document.createElement('serica-store-button');
    host.style.cssText = 'position:fixed;right:24px;bottom:24px;z-index:2147483647;';
    const root = host.attachShadow({ mode: 'closed' });
    // The store enforces Trusted Types: build nodes, never assign innerHTML.
    const style = document.createElement('style');
    style.textContent =
      '.w{display:flex;flex-direction:column;align-items:flex-end;gap:6px;font:500 14px/1.2 "Segoe UI",system-ui,sans-serif}' +
      'button{display:flex;align-items:center;gap:8px;border:0;border-radius:999px;padding:12px 20px;font:600 14px "Segoe UI",system-ui,sans-serif;color:#fff;background:#0b57d0;box-shadow:0 6px 24px rgba(0,0,0,.25);cursor:pointer;transition:background .15s,transform .15s}' +
      'button:hover{background:#0842a0;transform:translateY(-1px)}button[disabled]{cursor:default;transform:none}' +
      'button.installed{background:#1e8e3e}button.installing{background:#5f6368}button.error{background:#c5221f}' +
      '.n{max-width:320px;padding:8px 12px;border-radius:10px;background:rgba(32,33,36,.92);color:#fff;font-size:12.5px;display:none}';
    const wrap = document.createElement('div');
    wrap.className = 'w';
    note = document.createElement('div');
    note.className = 'n';
    btn = document.createElement('button');
    btn.type = 'button';
    wrap.append(note, btn);
    root.append(style, wrap);
    btn.addEventListener('click', () => {
      const s = states[current] || 'idle';
      if (!current || s === 'installing' || s === 'installed') return;
      states[current] = 'installing';
      delete errors[current];
      render();
      post({ t: 'ext-install', id: current });
    });
    (document.body || document.documentElement).appendChild(host);
  };
  const render = () => {
    if (!current) { if (host) host.style.display = 'none'; return; }
    ensure();
    host.style.display = '';
    const s = states[current] || 'idle';
    btn.className = s;
    btn.disabled = s === 'installing' || s === 'installed';
    btn.textContent = (s === 'installed' ? '✓ ' : '') + labels[s];
    note.textContent = errors[current] || '';
    note.style.display = errors[current] ? 'block' : 'none';
  };
  const tick = () => {
    const id = idFromPath();
    if (id !== current) { current = id; if (id && !states[id]) post({ t: 'ext-query', id }); }
    render();
  };
  Object.defineProperty(window, '__sericaStore', { value: { state(id, s, err) { states[id] = s; if (err) errors[id] = err; else delete errors[id]; render(); } } });
  const start = () => { tick(); setInterval(tick, 500); };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start, { once: true });
  else start();
})();"#;

/// JS that updates the Web Store button state in a tab.
pub fn store_state_js(id: &str, state: &str, error: Option<&str>) -> String {
    let err = error.map(|e| serde_json::to_string(e).unwrap_or_default()).unwrap_or_else(|| "null".into());
    format!("window.__sericaStore && window.__sericaStore.state({}, {}, {err});", serde_json::to_string(id).unwrap(), serde_json::to_string(state).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ids() {
        let id = "ddkjiahejlhfcafbddmgiahcphecmpfh";
        assert_eq!(parse_extension_id(id).as_deref(), Some(id));
        assert_eq!(parse_extension_id(&format!("https://chromewebstore.google.com/detail/ublock-origin-lite/{id}")).as_deref(), Some(id));
        assert_eq!(parse_extension_id(&format!("https://chrome.google.com/webstore/detail/x/{id}?hl=en")).as_deref(), Some(id));
        assert_eq!(parse_extension_id(&format!("https://evil.example/detail/{id}")), None);
        assert_eq!(parse_extension_id("hello"), None);
    }

    #[test]
    fn crx3_header() {
        // CrxFileHeader { sha256_with_rsa: [{ public_key: "KEY" }] } + zip magic
        let header = [0x12, 0x05, 0x0a, 0x03, b'K', b'E', b'Y'];
        let mut crx = b"Cr24".to_vec();
        crx.extend(3u32.to_le_bytes());
        crx.extend((header.len() as u32).to_le_bytes());
        crx.extend(header);
        crx.extend(b"PK\x03\x04rest");
        let (zip, keys) = parse_crx(&crx).unwrap();
        assert!(zip.starts_with(b"PK"));
        assert_eq!(keys, vec![b"KEY".to_vec()]);
        assert!(parse_crx(b"nope").is_err());
    }

    /// `SERICA_TEST_CRX=<path to ddkjiahejlhfcafbddmgiahcphecmpfh.crx> cargo test -- --ignored`
    #[test]
    #[ignore]
    fn unpacks_real_crx() {
        let id = "ddkjiahejlhfcafbddmgiahcphecmpfh";
        let crx = std::fs::read(std::env::var("SERICA_TEST_CRX").unwrap()).unwrap();
        let dir = std::env::temp_dir().join("serica-ext-test");
        let _ = std::fs::remove_dir_all(&dir);
        let rec = unpack(&dir, id, &crx).unwrap();
        let manifest = read_manifest(&dir.join(id)).unwrap();
        let key = base64::engine::general_purpose::STANDARD.decode(manifest["key"].as_str().unwrap()).unwrap();
        assert_eq!(id_from_key(&key), id);
        assert!(!dir.join(id).join("_metadata").exists());
        assert!(rec.icon.is_some() && !rec.name.starts_with("__MSG_"));
        eprintln!("{} {} popup={:?} options={:?}", rec.name, rec.version, rec.popup, rec.options);
    }

    #[test]
    fn id_derivation_is_a_to_p() {
        let id = id_from_key(b"anything");
        assert!(is_extension_id(&id));
    }
}
