//! Bringing things over from other browsers on this PC: bookmarks, history,
//! saved passwords and profiles, from Chrome, Edge, Brave, Vivaldi, Opera,
//! Arc, Chromium, Firefox, Zen, LibreWolf and Waterfox. Nothing leaves the
//! computer and nothing in the other browser changes (its files are copied
//! before they are read).
//!
//! Where things land in Serica:
//! - history joins Serica's history (address-bar suggestions included);
//! - bookmarks are kept in `bookmarks.json` (found from the address bar) and
//!   the bookmarks bar becomes a pinned folder in the sidebar;
//! - passwords go into the WebView2 password store of the chosen Serica
//!   profile, so pages fill them in just like passwords saved in Serica;
//! - a browser profile can come over as a new Serica profile of its own.
//!
//! Chrome 127+ protects some passwords with app-bound encryption ("v20")
//! that only Chrome itself can read. Those are counted and the user is told
//! to export them from Chrome as a file, which `import_passwords_file` reads.

use crate::app::Shared;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ------------------------------------------------------------------ sources

#[derive(Clone, Copy, PartialEq, Debug)]
enum Engine {
    Chromium,
    /// Chromium with the profile in the user-data folder itself (Opera).
    ChromiumSingle,
    Gecko,
}

struct Known {
    id: &'static str,
    name: &'static str,
    engine: Engine,
    /// "LOCALAPPDATA" or "APPDATA"
    base: &'static str,
    rel: &'static str,
}

const KNOWN: &[Known] = &[
    Known { id: "chrome", name: "Google Chrome", engine: Engine::Chromium, base: "LOCALAPPDATA", rel: r"Google\Chrome\User Data" },
    Known { id: "edge", name: "Microsoft Edge", engine: Engine::Chromium, base: "LOCALAPPDATA", rel: r"Microsoft\Edge\User Data" },
    Known { id: "brave", name: "Brave", engine: Engine::Chromium, base: "LOCALAPPDATA", rel: r"BraveSoftware\Brave-Browser\User Data" },
    Known { id: "vivaldi", name: "Vivaldi", engine: Engine::Chromium, base: "LOCALAPPDATA", rel: r"Vivaldi\User Data" },
    Known { id: "opera", name: "Opera", engine: Engine::ChromiumSingle, base: "APPDATA", rel: r"Opera Software\Opera Stable" },
    Known { id: "operagx", name: "Opera GX", engine: Engine::ChromiumSingle, base: "APPDATA", rel: r"Opera Software\Opera GX Stable" },
    Known { id: "chromium", name: "Chromium", engine: Engine::Chromium, base: "LOCALAPPDATA", rel: r"Chromium\User Data" },
    Known { id: "firefox", name: "Firefox", engine: Engine::Gecko, base: "APPDATA", rel: r"Mozilla\Firefox" },
    Known { id: "zen", name: "Zen", engine: Engine::Gecko, base: "APPDATA", rel: r"zen" },
    Known { id: "librewolf", name: "LibreWolf", engine: Engine::Gecko, base: "APPDATA", rel: r"librewolf" },
    Known { id: "waterfox", name: "Waterfox", engine: Engine::Gecko, base: "APPDATA", rel: r"Waterfox" },
];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceProfile {
    /// Folder of the profile, relative to the browser's data folder.
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    pub name: String,
    /// "chromium" | "gecko"
    pub engine: &'static str,
    pub profiles: Vec<SourceProfile>,
}

struct Located {
    id: String,
    name: String,
    engine: Engine,
    root: PathBuf,
}

fn locate_all() -> Vec<Located> {
    let mut out = Vec::new();
    for k in KNOWN {
        let Some(base) = std::env::var_os(k.base) else { continue };
        let root = PathBuf::from(base).join(k.rel);
        if root.is_dir() {
            out.push(Located { id: k.id.into(), name: k.name.into(), engine: k.engine, root });
        }
    }
    // Arc on Windows lives in its app package's folder.
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        if let Ok(list) = std::fs::read_dir(PathBuf::from(local).join("Packages")) {
            for e in list.flatten() {
                if e.file_name().to_string_lossy().starts_with("TheBrowserCompany.Arc_") {
                    let root = e.path().join(r"LocalCache\Local\Arc\User Data");
                    if root.is_dir() {
                        out.push(Located { id: "arc".into(), name: "Arc".into(), engine: Engine::Chromium, root });
                        break;
                    }
                }
            }
        }
    }
    out
}

fn chromium_profiles(root: &Path) -> Vec<SourceProfile> {
    let state: serde_json::Value = std::fs::read_to_string(root.join("Local State")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let mut list: Vec<SourceProfile> = Vec::new();
    if let Some(cache) = state.pointer("/profile/info_cache").and_then(|v| v.as_object()) {
        for (dir, info) in cache {
            if root.join(dir).is_dir() {
                let name = info.get("name").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or(dir).to_string();
                list.push(SourceProfile { id: dir.clone(), name });
            }
        }
    }
    if list.is_empty() && root.join("Default").is_dir() {
        list.push(SourceProfile { id: "Default".into(), name: "Default".into() });
    }
    // Default first, then the rest in their folder order.
    list.sort_by_key(|p| (p.id != "Default", p.id.clone()));
    list
}

/// `[ProfileN]` sections of a Gecko browser's profiles.ini.
fn gecko_profiles(root: &Path) -> Vec<SourceProfile> {
    let Ok(text) = std::fs::read_to_string(root.join("profiles.ini")) else { return Vec::new() };
    let mut out = Vec::new();
    let mut default_paths: Vec<String> = Vec::new();
    let (mut section, mut name, mut path, mut relative, mut is_default) = (String::new(), String::new(), String::new(), true, false);
    let flush = |out: &mut Vec<(SourceProfile, bool)>, section: &str, name: &str, path: &str, relative: bool, is_default: bool| {
        if section.starts_with("Profile") && !path.is_empty() {
            let full = if relative { root.join(path) } else { PathBuf::from(path) };
            if full.join("places.sqlite").is_file() || full.join("logins.json").is_file() {
                let id = if relative { path.to_string() } else { full.to_string_lossy().to_string() };
                out.push((SourceProfile { id, name: if name.is_empty() { path.to_string() } else { name.to_string() } }, is_default));
            }
        }
    };
    let mut found: Vec<(SourceProfile, bool)> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            flush(&mut found, &section, &name, &path, relative, is_default);
            section = line[1..line.len() - 1].to_string();
            name.clear();
            path.clear();
            relative = true;
            is_default = false;
        } else if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "Name" => name = v.trim().to_string(),
                "Path" => path = v.trim().replace('/', "\\"),
                "IsRelative" => relative = v.trim() != "0",
                "Default" if section.starts_with("Profile") => is_default = v.trim() == "1",
                "Default" if section.starts_with("Install") => default_paths.push(v.trim().replace('/', "\\")),
                _ => {}
            }
        }
    }
    flush(&mut found, &section, &name, &path, relative, is_default);
    // The installation's default profile first.
    found.sort_by_key(|(p, d)| (!(default_paths.contains(&p.id) || *d), p.name.clone()));
    for (p, _) in found {
        out.push(p);
    }
    out
}

pub fn sources() -> Vec<Source> {
    locate_all()
        .into_iter()
        .filter_map(|l| {
            let profiles = match l.engine {
                Engine::Chromium => chromium_profiles(&l.root),
                Engine::ChromiumSingle => vec![SourceProfile { id: ".".into(), name: "Default".into() }],
                Engine::Gecko => gecko_profiles(&l.root),
            };
            (!profiles.is_empty()).then(|| Source {
                id: l.id,
                name: l.name,
                engine: if l.engine == Engine::Gecko { "gecko" } else { "chromium" },
                profiles,
            })
        })
        .collect()
}

// ------------------------------------------------------------------ reading

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Login {
    pub origin: String,
    pub action: String,
    pub realm: String,
    pub username: String,
    pub password: String,
    pub created_ms: i64,
}

#[derive(Default)]
struct Read {
    bookmarks: Vec<crate::bookmarks::Bookmark>,
    /// The bookmarks bar, in order: (title, url).
    bar: Vec<(String, String)>,
    history: Vec<(String, String, i64, i64)>,
    logins: Vec<Login>,
    /// Passwords only the other browser can read (Chrome's app-bound encryption).
    logins_locked: usize,
    errors: Vec<String>,
}

/// Copies an SQLite database (and its journal) aside so it can be read while
/// the other browser is open.
fn copy_db(src: &Path) -> Result<(tempdir::Dir, PathBuf), String> {
    let dir = tempdir::Dir::new()?;
    let name = src.file_name().ok_or("bad path")?;
    let dst = dir.path.join(name);
    std::fs::copy(src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
    for suffix in ["-wal", "-journal", "-shm"] {
        let side = PathBuf::from(format!("{}{suffix}", src.display()));
        if side.is_file() {
            let _ = std::fs::copy(&side, PathBuf::from(format!("{}{suffix}", dst.display())));
        }
    }
    Ok((dir, dst))
}

mod tempdir {
    /// A temporary folder removed when dropped.
    pub struct Dir {
        pub path: std::path::PathBuf,
    }
    impl Dir {
        pub fn new() -> Result<Self, String> {
            let path = std::env::temp_dir().join(format!("serica-import-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            Ok(Self { path })
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn open_ro(path: &Path) -> Result<rusqlite::Connection, String> {
    rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|e| e.to_string())
}

/// Chromium time (µs since 1601) to ms since 1970.
fn webkit_to_ms(t: i64) -> i64 {
    if t <= 0 {
        return 0;
    }
    t / 1000 - 11_644_473_600_000
}

fn ms_to_webkit(ms: i64) -> i64 {
    (ms + 11_644_473_600_000) * 1000
}

fn chromium_bookmarks(profile: &Path, source: &str, r: &mut Read) {
    let Ok(text) = std::fs::read_to_string(profile.join("Bookmarks")) else { return };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        r.errors.push("bookmarks-unreadable".into());
        return;
    };
    fn walk(node: &serde_json::Value, folder: &str, out: &mut Vec<(String, String, String)>) {
        match node.get("type").and_then(|t| t.as_str()) {
            Some("url") => {
                let url = node.get("url").and_then(|u| u.as_str()).unwrap_or_default();
                let name = node.get("name").and_then(|u| u.as_str()).unwrap_or_default();
                out.push((name.to_string(), url.to_string(), folder.to_string()));
            }
            Some("folder") => {
                let name = node.get("name").and_then(|u| u.as_str()).unwrap_or_default();
                let path = if folder.is_empty() { name.to_string() } else { format!("{folder} / {name}") };
                for c in node.get("children").and_then(|c| c.as_array()).into_iter().flatten() {
                    walk(c, &path, out);
                }
            }
            _ => {}
        }
    }
    let now = crate::model::now_ms();
    for (key, label) in [("bookmark_bar", "Bookmarks bar"), ("other", "Other bookmarks"), ("synced", "Mobile bookmarks")] {
        let Some(root) = v.pointer(&format!("/roots/{key}")) else { continue };
        let mut found = Vec::new();
        for c in root.get("children").and_then(|c| c.as_array()).into_iter().flatten() {
            walk(c, label, &mut found);
        }
        for (title, url, folder) in found {
            if key == "bookmark_bar" {
                r.bar.push((title.clone(), url.clone()));
            }
            r.bookmarks.push(crate::bookmarks::Bookmark { title, url, folder, source: source.to_string(), added: now });
        }
    }
}

fn chromium_history(profile: &Path, r: &mut Read) {
    let path = profile.join("History");
    if !path.is_file() {
        return;
    }
    let res = (|| -> Result<Vec<(String, String, i64, i64)>, String> {
        let (_tmp, copy) = copy_db(&path)?;
        let db = open_ro(&copy)?;
        let mut q = db
            .prepare("SELECT url, IFNULL(title, ''), visit_count, last_visit_time FROM urls WHERE hidden = 0 AND last_visit_time > 0 ORDER BY last_visit_time DESC LIMIT 20000")
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, webkit_to_ms(row.get::<_, i64>(3)?), row.get::<_, i64>(2)?)))
            .map_err(|e| e.to_string())?;
        Ok(rows.flatten().collect())
    })();
    match res {
        Ok(h) => r.history.extend(h),
        Err(e) => r.errors.push(format!("history: {e}")),
    }
}

/// The AES key a Chromium-based browser (or WebView2) encrypts passwords
/// with, from its "Local State" (protected by Windows for this user).
#[cfg(windows)]
pub fn chromium_key(local_state: &Path) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    let text = std::fs::read_to_string(local_state).map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let enc = v.pointer("/os_crypt/encrypted_key").and_then(|k| k.as_str()).ok_or("no key")?;
    let raw = base64::engine::general_purpose::STANDARD.decode(enc).map_err(|e| e.to_string())?;
    let raw = raw.strip_prefix(b"DPAPI").ok_or("unexpected key format")?;
    let key = dpapi_unprotect(raw).ok_or("key could not be unlocked")?;
    if key.len() != 32 {
        return Err("unexpected key length".into());
    }
    Ok(key)
}

#[cfg(windows)]
fn dpapi_unprotect(data: &[u8]) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        CryptUnprotectData(&input, None, None, None, None, 0, &mut out).ok()?;
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
        Some(v)
    }
}

/// Decrypts a Chromium `password_value`. `Err(true)` = app-bound (v20),
/// which only the browser itself can read.
#[cfg(windows)]
fn chromium_decrypt(key: &[u8], value: &[u8]) -> Result<String, bool> {
    use aes_gcm::aead::{Aead, KeyInit};
    if value.starts_with(b"v20") {
        return Err(true);
    }
    if value.starts_with(b"v10") || value.starts_with(b"v11") {
        if value.len() < 3 + 12 + 16 {
            return Err(false);
        }
        let cipher = aes_gcm::Aes256Gcm::new_from_slice(key).map_err(|_| false)?;
        let nonce = aes_gcm::Nonce::from_slice(&value[3..15]);
        let plain = cipher.decrypt(nonce, &value[15..]).map_err(|_| false)?;
        return String::from_utf8(plain).map_err(|_| false);
    }
    // Very old profiles: the value itself is protected by Windows.
    dpapi_unprotect(value).and_then(|p| String::from_utf8(p).ok()).ok_or(false)
}

/// Encrypts a password the way Chromium/WebView2 store it ("v10" AES-GCM).
#[cfg(windows)]
pub fn chromium_encrypt(key: &[u8], password: &str) -> Option<Vec<u8>> {
    use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
    let cipher = aes_gcm::Aes256Gcm::new_from_slice(key).ok()?;
    let nonce = aes_gcm::Aes256Gcm::generate_nonce(&mut OsRng);
    let mut out = b"v10".to_vec();
    out.extend_from_slice(&nonce);
    out.extend(cipher.encrypt(&nonce, password.as_bytes()).ok()?);
    Some(out)
}

#[cfg(windows)]
fn chromium_logins(root: &Path, profile: &Path, r: &mut Read) {
    let path = profile.join("Login Data");
    if !path.is_file() {
        return;
    }
    let key = match chromium_key(&root.join("Local State")) {
        Ok(k) => k,
        Err(e) => {
            r.errors.push(format!("passwords: {e}"));
            return;
        }
    };
    let res = (|| -> Result<(), String> {
        let (_tmp, copy) = copy_db(&path)?;
        let db = open_ro(&copy)?;
        let mut q = db
            .prepare("SELECT origin_url, IFNULL(action_url, ''), IFNULL(username_value, ''), password_value, signon_realm, date_created FROM logins WHERE blacklisted_by_user = 0")
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, Vec<u8>>(3)?, row.get::<_, String>(4)?, row.get::<_, i64>(5)?))
            })
            .map_err(|e| e.to_string())?;
        for (origin, action, username, value, realm, created) in rows.flatten() {
            match chromium_decrypt(&key, &value) {
                Ok(password) if !password.is_empty() => {
                    r.logins.push(Login { origin, action, realm, username, password, created_ms: webkit_to_ms(created) });
                }
                Ok(_) => {}
                Err(true) => r.logins_locked += 1,
                Err(false) => {}
            }
        }
        Ok(())
    })();
    if let Err(e) = res {
        r.errors.push(format!("passwords: {e}"));
    }
}

fn gecko_places(profile: &Path, source: &str, want_bookmarks: bool, want_history: bool, r: &mut Read) {
    let path = profile.join("places.sqlite");
    if !path.is_file() {
        return;
    }
    let res = (|| -> Result<(), String> {
        let (_tmp, copy) = copy_db(&path)?;
        let db = open_ro(&copy)?;
        if want_history {
            let mut q = db
                .prepare("SELECT url, IFNULL(title, ''), visit_count, last_visit_date FROM moz_places WHERE last_visit_date IS NOT NULL AND hidden = 0 ORDER BY last_visit_date DESC LIMIT 20000")
                .map_err(|e| e.to_string())?;
            let rows = q.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(3)? / 1000, row.get::<_, i64>(2)?))).map_err(|e| e.to_string())?;
            r.history.extend(rows.flatten());
        }
        if want_bookmarks {
            struct Node {
                parent: i64,
                kind: i64,
                title: String,
                url: String,
                guid: String,
            }
            let mut q = db
                .prepare("SELECT b.id, b.parent, b.type, IFNULL(b.title, ''), IFNULL(p.url, ''), IFNULL(b.guid, '') FROM moz_bookmarks b LEFT JOIN moz_places p ON b.fk = p.id ORDER BY b.parent, b.position")
                .map_err(|e| e.to_string())?;
            let rows = q
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, Node { parent: row.get(1)?, kind: row.get(2)?, title: row.get(3)?, url: row.get(4)?, guid: row.get(5)? }))
                })
                .map_err(|e| e.to_string())?;
            let ordered: Vec<(i64, Node)> = rows.flatten().collect();
            let by_id: std::collections::HashMap<i64, &Node> = ordered.iter().map(|(id, n)| (*id, n)).collect();
            let root_name = |guid: &str| match guid {
                "toolbar_____" => Some("Bookmarks toolbar"),
                "menu________" => Some("Bookmarks menu"),
                "unfiled_____" => Some("Other bookmarks"),
                "mobile______" => Some("Mobile bookmarks"),
                _ => None,
            };
            let now = crate::model::now_ms();
            for (_, n) in &ordered {
                if n.kind != 1 || !(n.url.starts_with("http://") || n.url.starts_with("https://")) {
                    continue;
                }
                // Folder path up to one of the roots.
                let mut parts: Vec<String> = Vec::new();
                let mut at = n.parent;
                let mut on_bar = false;
                for _ in 0..32 {
                    let Some(p) = by_id.get(&at) else { break };
                    if let Some(name) = root_name(&p.guid) {
                        on_bar = p.guid == "toolbar_____";
                        parts.push(name.to_string());
                        break;
                    }
                    if p.guid == "root________" {
                        break;
                    }
                    parts.push(p.title.clone());
                    at = p.parent;
                }
                parts.reverse();
                if on_bar {
                    r.bar.push((n.title.clone(), n.url.clone()));
                }
                r.bookmarks.push(crate::bookmarks::Bookmark { title: n.title.clone(), url: n.url.clone(), folder: parts.join(" / "), source: source.to_string(), added: now });
            }
        }
        Ok(())
    })();
    if let Err(e) = res {
        r.errors.push(format!("places: {e}"));
    }
}

// ---- Firefox passwords (NSS): logins.json, keys in key4.db

mod der {
    /// One DER element: tag and contents.
    pub struct El<'a> {
        #[allow(dead_code)]
        pub tag: u8,
        pub body: &'a [u8],
    }

    pub fn read(b: &[u8]) -> Option<(El<'_>, &[u8])> {
        if b.len() < 2 {
            return None;
        }
        let tag = b[0];
        let (len, head) = if b[1] < 0x80 {
            (b[1] as usize, 2)
        } else {
            let n = (b[1] & 0x7f) as usize;
            if n == 0 || n > 4 || b.len() < 2 + n {
                return None;
            }
            (b[2..2 + n].iter().fold(0usize, |a, x| (a << 8) | *x as usize), 2 + n)
        };
        if b.len() < head + len {
            return None;
        }
        Some((El { tag, body: &b[head..head + len] }, &b[head + len..]))
    }

    pub fn children(body: &[u8]) -> Vec<El<'_>> {
        let mut out = Vec::new();
        let mut rest = body;
        while !rest.is_empty() {
            let Some((el, r)) = read(rest) else { break };
            out.push(el);
            rest = r;
        }
        out
    }

    pub fn int(body: &[u8]) -> usize {
        body.iter().fold(0usize, |a, x| (a << 8) | *x as usize)
    }
}

const OID_PBES2: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x05, 0x0D];
const OID_PBE_SHA1_3DES: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x0C, 0x05, 0x01, 0x03];
const OID_AES256_CBC: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x2A];
const OID_DES_EDE3_CBC: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x03, 0x07];
const CKA_ID: &[u8] = &[0xF8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];

fn unpad(mut v: Vec<u8>) -> Option<Vec<u8>> {
    let n = *v.last()? as usize;
    if n == 0 || n > 16 || n > v.len() || !v[v.len() - n..].iter().all(|b| *b as usize == n) {
        return None;
    }
    v.truncate(v.len() - n);
    Some(v)
}

fn aes256_cbc(key: &[u8], iv: &[u8], ct: &[u8]) -> Option<Vec<u8>> {
    use cbc::cipher::{block_padding::NoPadding, BlockDecryptMut, KeyIvInit};
    if key.len() < 32 || iv.len() != 16 || ct.len() % 16 != 0 {
        return None;
    }
    let mut buf = ct.to_vec();
    cbc::Decryptor::<aes::Aes256>::new_from_slices(&key[..32], iv).ok()?.decrypt_padded_mut::<NoPadding>(&mut buf).ok()?;
    Some(buf)
}

fn des3_cbc(key: &[u8], iv: &[u8], ct: &[u8]) -> Option<Vec<u8>> {
    use cbc::cipher::{block_padding::NoPadding, BlockDecryptMut, KeyIvInit};
    if key.len() < 24 || iv.len() != 8 || ct.len() % 8 != 0 {
        return None;
    }
    let mut buf = ct.to_vec();
    cbc::Decryptor::<des::TdesEde3>::new_from_slices(&key[..24], iv).ok()?.decrypt_padded_mut::<NoPadding>(&mut buf).ok()?;
    Some(buf)
}

/// Decrypts an NSS password-based encrypted item from key4.db (PBES2 with
/// AES-256, or the older SHA-1 + 3DES scheme). `None` with a wrong (or a
/// set) primary password.
fn nss_pbe(item: &[u8], global_salt: &[u8], primary_password: &[u8]) -> Option<Vec<u8>> {
    use hmac::{Hmac, Mac};
    use sha1::{Digest, Sha1};
    let (top, _) = der::read(item)?;
    let parts = der::children(top.body);
    let algo = der::children(parts.first()?.body);
    let ct = parts.get(1)?.body;
    let oid = algo.first()?.body;
    if oid == OID_PBES2 {
        let params = der::children(algo.get(1)?.body);
        let kdf = der::children(params.first()?.body);
        let kdf_params = der::children(kdf.get(1)?.body);
        let salt = kdf_params.first()?.body;
        let iterations = der::int(kdf_params.get(1)?.body) as u32;
        let enc = der::children(params.get(1)?.body);
        if enc.first()?.body != OID_AES256_CBC {
            return None;
        }
        let iv_raw = enc.get(1)?.body;
        let mut iv = Vec::with_capacity(16);
        if iv_raw.len() == 14 {
            // NSS stores the IV with its own DER header cut off.
            iv.extend_from_slice(&[0x04, 0x0E]);
        }
        iv.extend_from_slice(iv_raw);
        let mut h = Sha1::new();
        h.update(global_salt);
        h.update(primary_password);
        let k = h.finalize();
        let mut key = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<sha2::Sha256>(&k, salt, iterations.max(1), &mut key);
        return aes256_cbc(&key, &iv, ct).and_then(unpad);
    }
    if oid == OID_PBE_SHA1_3DES {
        let params = der::children(algo.get(1)?.body);
        let entry_salt = params.first()?.body;
        let mut h = Sha1::new();
        h.update(global_salt);
        h.update(primary_password);
        let hp = h.finalize();
        let mut pes = entry_salt.to_vec();
        pes.resize(20.max(entry_salt.len()), 0);
        let mut h = Sha1::new();
        h.update(hp);
        h.update(entry_salt);
        let chp = h.finalize();
        let hmac = |data: &[&[u8]]| {
            let mut m = <Hmac<Sha1> as Mac>::new_from_slice(&chp).expect("hmac key");
            for d in data {
                m.update(d);
            }
            m.finalize().into_bytes().to_vec()
        };
        let k1 = hmac(&[&pes, entry_salt]);
        let tk = hmac(&[&pes]);
        let k2 = hmac(&[&tk, entry_salt]);
        let k: Vec<u8> = k1.into_iter().chain(k2).collect();
        return des3_cbc(&k[..24], &k[k.len() - 8..], ct).and_then(unpad);
    }
    None
}

/// The key Firefox encrypts logins with. `Err("primary-password")` when the
/// profile has a primary password set.
fn gecko_key(key4: &Path) -> Result<Vec<u8>, String> {
    let (_tmp, copy) = copy_db(key4)?;
    let db = open_ro(&copy)?;
    let (salt, check): (Vec<u8>, Vec<u8>) = db
        .query_row("SELECT item1, item2 FROM metadata WHERE id = 'password'", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?;
    let ok = nss_pbe(&check, &salt, b"").is_some_and(|p| p.starts_with(b"password-check"));
    if !ok {
        return Err("primary-password".into());
    }
    let mut q = db.prepare("SELECT a11, a102 FROM nssPrivate").map_err(|e| e.to_string())?;
    let rows = q.query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))).map_err(|e| e.to_string())?;
    for (a11, a102) in rows.flatten() {
        if a102 == CKA_ID {
            return nss_pbe(&a11, &salt, b"").ok_or_else(|| "key-unreadable".to_string());
        }
    }
    Err("no-key".into())
}

/// Decrypts one `encryptedUsername` / `encryptedPassword` value.
fn gecko_decrypt(key: &[u8], b64: &str) -> Option<String> {
    use base64::Engine as _;
    let raw = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let (top, _) = der::read(&raw)?;
    let parts = der::children(top.body);
    let algo = der::children(parts.get(1)?.body);
    let iv = algo.get(1)?.body;
    let ct = parts.get(2)?.body;
    let plain = match algo.first()?.body {
        o if o == OID_DES_EDE3_CBC => des3_cbc(key, iv, ct)?,
        o if o == OID_AES256_CBC => aes256_cbc(key, iv, ct)?,
        _ => return None,
    };
    String::from_utf8(unpad(plain)?).ok()
}

fn gecko_logins(profile: &Path, r: &mut Read) {
    let (logins, key4) = (profile.join("logins.json"), profile.join("key4.db"));
    if !logins.is_file() || !key4.is_file() {
        return;
    }
    let key = match gecko_key(&key4) {
        Ok(k) => k,
        Err(e) => {
            r.errors.push(format!("passwords: {e}"));
            return;
        }
    };
    let Ok(v) = std::fs::read_to_string(&logins).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|e| e.to_string())) else {
        r.errors.push("passwords: logins.json unreadable".into());
        return;
    };
    for l in v.get("logins").and_then(|l| l.as_array()).into_iter().flatten() {
        let host = l.get("hostname").and_then(|h| h.as_str()).unwrap_or_default();
        if !(host.starts_with("http://") || host.starts_with("https://")) {
            continue;
        }
        let (Some(user), Some(pass)) = (
            l.get("encryptedUsername").and_then(|x| x.as_str()).and_then(|x| gecko_decrypt(&key, x)),
            l.get("encryptedPassword").and_then(|x| x.as_str()).and_then(|x| gecko_decrypt(&key, x)),
        ) else {
            continue;
        };
        if pass.is_empty() {
            continue;
        }
        let action = l.get("formSubmitURL").and_then(|x| x.as_str()).unwrap_or_default().to_string();
        let created = l.get("timeCreated").and_then(|x| x.as_i64()).unwrap_or_else(crate::model::now_ms);
        let base = host.trim_end_matches('/');
        r.logins.push(Login { origin: format!("{base}/"), action, realm: format!("{base}/"), username: user, password: pass, created_ms: created });
    }
}

// ---- Password export files (CSV)

/// Splits CSV text into rows (quotes, doubled quotes and line breaks inside
/// quotes handled).
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// Logins from a password export (Chrome, Edge, Brave, Firefox, Bitwarden
/// and 1Password all write url/username/password columns).
pub fn logins_from_csv(text: &str) -> Vec<Login> {
    let rows = parse_csv(text);
    let Some(head) = rows.first() else { return Vec::new() };
    let col = |names: &[&str]| head.iter().position(|h| names.contains(&h.trim().to_ascii_lowercase().as_str()));
    let (Some(u), Some(n), Some(p)) = (col(&["url", "origin", "hostname", "login_uri", "website", "web site"]), col(&["username", "login", "login_username", "user", "email"]), col(&["password", "login_password"])) else {
        return Vec::new();
    };
    let action = col(&["formactionorigin", "action_url"]);
    let now = crate::model::now_ms();
    rows.iter()
        .skip(1)
        .filter_map(|r| {
            let url = r.get(u)?.trim();
            let parsed = url::Url::parse(url).ok()?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return None;
            }
            let password = r.get(p)?.to_string();
            if password.is_empty() {
                return None;
            }
            let origin = parsed.origin().ascii_serialization();
            Some(Login {
                origin: format!("{origin}{}", if parsed.path().is_empty() { "/" } else { parsed.path() }),
                action: action.and_then(|a| r.get(a)).cloned().unwrap_or_default(),
                realm: format!("{origin}/"),
                username: r.get(n).cloned().unwrap_or_default(),
                password,
                created_ms: now,
            })
        })
        .collect()
}

// ------------------------------------------------------------------ writing passwords

/// Writes logins into a Serica profile's WebView2 password store. The
/// profile's pages must be closed (its browser process gone): the database
/// is locked while it runs, so this waits for the lock up to `wait`.
#[cfg(windows)]
pub fn write_logins(webview_data: &Path, logins: &[Login], wait: std::time::Duration) -> Result<usize, String> {
    let key = chromium_key(&webview_data.join("Local State"))?;
    let db_path = webview_data.join("Default").join("Login Data");
    if !db_path.is_file() {
        return Err("profile-not-ready".into());
    }
    let _ = std::fs::copy(&db_path, webview_data.join("Default").join("Login Data.serica-backup"));
    let mut db = rusqlite::Connection::open(&db_path).map_err(|e| e.to_string())?;
    db.busy_timeout(wait).map_err(|e| e.to_string())?;
    let cols: std::collections::HashSet<String> = {
        let mut q = db.prepare("PRAGMA table_info(logins)").map_err(|e| e.to_string())?;
        let names = q.query_map([], |r| r.get::<_, String>(1)).map_err(|e| e.to_string())?;
        names.flatten().collect()
    };
    for needed in ["origin_url", "signon_realm", "password_value", "username_value", "date_created", "blacklisted_by_user", "scheme"] {
        if !cols.contains(needed) {
            return Err(format!("unexpected password store ({needed})"));
        }
    }
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Exclusive).map_err(|e| format!("busy: {e}"))?;
    let mut added = 0;
    {
        // Columns this engine version has, besides the always-present ones.
        let optional: Vec<&str> = ["action_url", "username_element", "password_element", "submit_element", "password_type", "times_used", "date_last_used", "date_password_modified", "display_name", "icon_url", "federation_url", "skip_zero_click", "generation_upload_status"]
            .into_iter()
            .filter(|c| cols.contains(*c))
            .collect();
        let mut names = vec!["origin_url", "signon_realm", "username_value", "password_value", "date_created", "blacklisted_by_user", "scheme"];
        names.extend(optional.iter().copied());
        let sql = format!("INSERT OR IGNORE INTO logins ({}) VALUES ({})", names.join(", "), (1..=names.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", "));
        let mut stmt = tx.prepare(&sql).map_err(|e| e.to_string())?;
        for l in logins {
            let Some(blob) = chromium_encrypt(&key, &l.password) else { continue };
            let created = ms_to_webkit(if l.created_ms > 0 { l.created_ms } else { crate::model::now_ms() });
            let mut values: Vec<rusqlite::types::Value> = vec![
                l.origin.clone().into(),
                l.realm.clone().into(),
                l.username.clone().into(),
                blob.into(),
                created.into(),
                0i64.into(),
                0i64.into(),
            ];
            for c in &optional {
                values.push(match *c {
                    "action_url" => l.action.clone().into(),
                    "date_password_modified" => created.into(),
                    "username_element" | "password_element" | "submit_element" | "display_name" | "icon_url" | "federation_url" => String::new().into(),
                    _ => 0i64.into(),
                });
            }
            if stmt.execute(rusqlite::params_from_iter(values)).map_err(|e| e.to_string())? > 0 {
                added += 1;
            }
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(added)
}

// ------------------------------------------------------------------ running an import

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub source: String,
    pub profile: String,
    pub bookmarks: bool,
    pub history: bool,
    pub passwords: bool,
    /// Bring the browser profile over as a new Serica profile.
    #[serde(default)]
    pub as_new_profile: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub bookmarks: usize,
    pub pinned: usize,
    pub history: usize,
    pub passwords: usize,
    /// Passwords only the other browser can read (export them as a file).
    pub passwords_locked: usize,
    /// The other browser has a primary password (export them as a file).
    pub primary_password: bool,
    pub profile_id: Option<String>,
    pub profile_name: Option<String>,
    pub errors: Vec<String>,
}

/// Puts logins into a Serica profile's password store: its pages close for a
/// moment (the store is locked while they run) and reload after.
#[cfg(windows)]
fn store_logins(app: &Shared, profile: &str, logins: &[Login]) -> Result<usize, String> {
    use crate::tabhost::TabHost;
    if logins.is_empty() {
        return Ok(0);
    }
    let data = app.host.lock().profile_dir(profile).join("EBWebView");
    TabHost::pause_profile(app, profile, true);
    let result = (|| {
        // A profile never opened has no store yet: open it once, hidden.
        if !data.join("Default").join("Login Data").is_file() {
            let probe = TabHost::probe_profile(app, profile);
            let start = std::time::Instant::now();
            while !data.join("Default").join("Login Data").is_file() && start.elapsed() < std::time::Duration::from_secs(15) {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            if let Some(p) = probe {
                let _ = p.close();
            }
        }
        write_logins(&data, logins, std::time::Duration::from_secs(20))
    })();
    TabHost::pause_profile(app, profile, false);
    result
}

#[cfg(windows)]
fn run(app: &Shared, req: &ImportRequest) -> Result<ImportReport, String> {
    let located = locate_all().into_iter().find(|l| l.id == req.source).ok_or("browser-not-found")?;
    let profile_dir = match located.engine {
        Engine::ChromiumSingle => located.root.clone(),
        Engine::Chromium => located.root.join(&req.profile),
        Engine::Gecko => {
            let p = PathBuf::from(&req.profile);
            if p.is_absolute() { p } else { located.root.join(&req.profile) }
        }
    };
    // Only folders the browser itself lists.
    let listed = match located.engine {
        Engine::ChromiumSingle => req.profile == ".",
        Engine::Chromium => chromium_profiles(&located.root).iter().any(|p| p.id == req.profile),
        Engine::Gecko => gecko_profiles(&located.root).iter().any(|p| p.id == req.profile),
    };
    if !listed || !profile_dir.is_dir() {
        return Err("profile-not-found".into());
    }
    let profile_label = match located.engine {
        Engine::Chromium => chromium_profiles(&located.root).into_iter().find(|p| p.id == req.profile).map(|p| p.name),
        Engine::Gecko => gecko_profiles(&located.root).into_iter().find(|p| p.id == req.profile).map(|p| p.name),
        Engine::ChromiumSingle => None,
    };

    let mut read = Read::default();
    match located.engine {
        Engine::Chromium | Engine::ChromiumSingle => {
            if req.bookmarks {
                chromium_bookmarks(&profile_dir, &located.name, &mut read);
            }
            if req.history {
                chromium_history(&profile_dir, &mut read);
            }
            if req.passwords {
                chromium_logins(&located.root, &profile_dir, &mut read);
            }
        }
        Engine::Gecko => {
            gecko_places(&profile_dir, &located.name, req.bookmarks, req.history, &mut read);
            if req.passwords {
                gecko_logins(&profile_dir, &mut read);
            }
        }
    }

    let mut report = ImportReport { passwords_locked: read.logins_locked, ..Default::default() };
    report.primary_password = read.errors.iter().any(|e| e.contains("primary-password"));
    report.errors = read.errors.into_iter().filter(|e| !e.contains("primary-password")).collect();

    // Where it goes: the active profile and space, or a new profile.
    let (profile_id, space_id, profile_name) = if req.as_new_profile {
        let name = match &profile_label {
            Some(l) if l != "Default" && !l.is_empty() => format!("{} · {l}", located.name),
            _ => located.name.clone(),
        };
        let (p, s) = app.mutate(|st| st.create_profile(Some(&name)));
        (p.id, s.id, Some(p.name))
    } else {
        let s = app.store.lock();
        (s.active_profile_id(), s.active_space_id.clone(), None)
    };
    report.profile_id = Some(profile_id.clone());
    report.profile_name = profile_name;

    if req.history && !read.history.is_empty() {
        report.history = app.history.lock().import(read.history);
        app.notify(crate::app::Notice::History);
    }
    if req.bookmarks && !read.bookmarks.is_empty() {
        report.bookmarks = app.bookmarks.lock().add(read.bookmarks);
        // The bookmarks bar, as a pinned folder in the sidebar.
        let bar: Vec<(String, String)> = read.bar.into_iter().filter(|(_, u)| u.starts_with("http")).take(40).collect();
        if !bar.is_empty() {
            let folder_name = format!("{} bookmarks", located.name);
            report.pinned = app.mutate(|st| {
                let Some(folder) = st.create_folder(&space_id, &folder_name) else { return 0 };
                let _ = st.update_folder(&folder.id, &serde_json::json!({ "collapsed": true }));
                let mut n = 0;
                for (title, url) in bar {
                    let title = if title.trim().is_empty() { url.clone() } else { title };
                    if let Some(t) = st.add_pinned_site(url, title, None, Some(space_id.clone())) {
                        st.move_tab_to_folder(&t.id, Some(&folder.id));
                        n += 1;
                    }
                }
                n
            });
        }
    }
    if req.passwords && !read.logins.is_empty() {
        match store_logins(app, &profile_id, &read.logins) {
            Ok(n) => report.passwords = n,
            Err(e) => report.errors.push(format!("passwords: {e}")),
        }
    }
    Ok(report)
}

#[tauri::command]
pub async fn import_sources() -> Vec<Source> {
    tauri::async_runtime::spawn_blocking(sources).await.unwrap_or_default()
}

#[tauri::command]
pub async fn import_from_browser(request: ImportRequest, app: tauri::State<'_, Shared>) -> Result<ImportReport, String> {
    #[cfg(windows)]
    {
        let app = app.inner().clone();
        tauri::async_runtime::spawn_blocking(move || run(&app, &request)).await.map_err(|e| e.to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = (request, app);
        Err("unsupported".into())
    }
}

/// Reads a password export (.csv) chosen by the user into the active profile.
#[tauri::command]
pub async fn import_passwords_file(app: tauri::State<'_, Shared>) -> Result<Option<ImportReport>, String> {
    #[cfg(windows)]
    {
        use tauri::Manager;
        let app = app.inner().clone();
        let owner = app.handle.get_webview_window("main").and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize).unwrap_or(0);
        tauri::async_runtime::spawn_blocking(move || {
            let Some(path) = (unsafe { crate::files::pick_one_file(owner, &[("Password exports (CSV)", "*.csv"), ("All files", "*.*")], "Import passwords") }) else {
                return Ok(None);
            };
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let logins = logins_from_csv(&text);
            if logins.is_empty() {
                return Err("no-passwords-in-file".into());
            }
            let profile = app.store.lock().active_profile_id();
            let n = store_logins(&app, &profile, &logins)?;
            Ok(Some(ImportReport { passwords: n, profile_id: Some(profile), ..Default::default() }))
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("unsupported".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_exports() {
        let chrome = "name,url,username,password,note\r\nGitHub,https://github.com/login,octo,\"pa,ss\"\"word\",\nBad,ftp://x,a,b,\n";
        let l = logins_from_csv(chrome);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].origin, "https://github.com/login");
        assert_eq!(l[0].realm, "https://github.com/");
        assert_eq!(l[0].username, "octo");
        assert_eq!(l[0].password, "pa,ss\"word");
        let firefox = "\"url\",\"username\",\"password\",\"httpRealm\",\"formActionOrigin\"\n\"https://a.test\",\"me\",\"pw\",,\"https://a.test\"\n";
        let l = logins_from_csv(firefox);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].action, "https://a.test");
        assert!(logins_from_csv("nothing,here\n1,2\n").is_empty());
    }


    /// Makes a Chromium-style "Local State" whose key is protected by Windows
    /// for this user, as Chrome writes it. Returns the raw key.
    #[cfg(windows)]
    fn fake_local_state(dir: &Path) -> Vec<u8> {
        use base64::Engine as _;
        use windows::Win32::Foundation::{LocalFree, HLOCAL};
        use windows::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};
        let key: Vec<u8> = (0..32u8).map(|i| i.wrapping_mul(7).wrapping_add(3)).collect();
        let protected = unsafe {
            let input = CRYPT_INTEGER_BLOB { cbData: 32, pbData: key.as_ptr() as *mut u8 };
            let mut out = CRYPT_INTEGER_BLOB::default();
            CryptProtectData(&input, None, None, None, None, 0, &mut out).unwrap();
            let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
            v
        };
        let mut enc = b"DPAPI".to_vec();
        enc.extend(protected);
        let state = serde_json::json!({
            "os_crypt": { "encrypted_key": base64::engine::general_purpose::STANDARD.encode(enc) },
            "profile": { "info_cache": { "Default": { "name": "Person 1" }, "Profile 2": { "name": "Work" } } }
        });
        std::fs::write(dir.join("Local State"), state.to_string()).unwrap();
        key
    }

    /// The `logins` table as WebView2 (Edge 14x) creates it.
    const WEBVIEW2_LOGINS: &str = "CREATE TABLE logins (origin_url VARCHAR NOT NULL, action_url VARCHAR, username_element VARCHAR, username_value VARCHAR, password_element VARCHAR, password_value BLOB, submit_element VARCHAR, signon_realm VARCHAR NOT NULL, date_created INTEGER NOT NULL, blacklisted_by_user INTEGER NOT NULL, scheme INTEGER NOT NULL, password_type INTEGER, times_used INTEGER, form_data BLOB, display_name VARCHAR, icon_url VARCHAR, federation_url VARCHAR, skip_zero_click INTEGER, generation_upload_status INTEGER, possible_username_pairs BLOB, id INTEGER PRIMARY KEY AUTOINCREMENT, date_last_used INTEGER NOT NULL DEFAULT 0, moving_blocked_for BLOB, date_password_modified INTEGER NOT NULL DEFAULT 0, sender_email VARCHAR, sender_name VARCHAR, date_received INTEGER, sharing_notification_displayed INTEGER NOT NULL DEFAULT 0, keychain_identifier BLOB, sender_profile_image_url VARCHAR, date_last_filled INTEGER NOT NULL DEFAULT 0, actor_login_approved INTEGER NOT NULL DEFAULT 0, UNIQUE (origin_url, username_element, username_value, password_element, signon_realm))";

    /// A whole fake Chrome profile goes through the readers.
    #[cfg(windows)]
    #[test]
    fn reads_a_chromium_profile() {
        let root = tempdir::Dir::new().unwrap();
        let key = fake_local_state(&root.path);
        assert_eq!(chromium_key(&root.path.join("Local State")).unwrap(), key);
        assert_eq!(chromium_profiles(&root.path).len(), 0, "profile folders don't exist yet");
        let profile = root.path.join("Default");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::create_dir_all(root.path.join("Profile 2")).unwrap();
        let names: Vec<String> = chromium_profiles(&root.path).into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["Person 1".to_string(), "Work".to_string()]);

        std::fs::write(
            profile.join("Bookmarks"),
            serde_json::json!({ "roots": {
                "bookmark_bar": { "type": "folder", "name": "Bookmarks bar", "children": [
                    { "type": "url", "name": "Docs", "url": "https://docs.rs/" },
                    { "type": "folder", "name": "Dev", "children": [ { "type": "url", "name": "Crates", "url": "https://crates.io/" } ] }
                ]},
                "other": { "type": "folder", "name": "Other", "children": [ { "type": "url", "name": "Rust", "url": "https://rust-lang.org/" } ] }
            }})
            .to_string(),
        )
        .unwrap();
        {
            let db = rusqlite::Connection::open(profile.join("History")).unwrap();
            db.execute_batch("CREATE TABLE urls (id INTEGER PRIMARY KEY, url LONGVARCHAR, title LONGVARCHAR, visit_count INTEGER DEFAULT 0 NOT NULL, typed_count INTEGER DEFAULT 0 NOT NULL, last_visit_time INTEGER NOT NULL, hidden INTEGER DEFAULT 0 NOT NULL);").unwrap();
            db.execute("INSERT INTO urls (url, title, visit_count, last_visit_time) VALUES ('https://example.com/', 'Example', 4, ?1)", [ms_to_webkit(1_764_000_000_000)]).unwrap();
            db.execute("INSERT INTO urls (url, title, visit_count, last_visit_time, hidden) VALUES ('https://hidden.test/', 'H', 1, ?1, 1)", [ms_to_webkit(1_764_000_000_000)]).unwrap();
        }
        {
            let db = rusqlite::Connection::open(profile.join("Login Data")).unwrap();
            db.execute_batch(WEBVIEW2_LOGINS).unwrap();
            let ok = chromium_encrypt(&key, "s3cret").unwrap();
            db.execute("INSERT INTO logins (origin_url, signon_realm, username_value, password_value, date_created, blacklisted_by_user, scheme) VALUES ('https://github.com/login', 'https://github.com/', 'octo', ?1, ?2, 0, 0)", rusqlite::params![ok, ms_to_webkit(1_764_000_000_000)]).unwrap();
            db.execute("INSERT INTO logins (origin_url, signon_realm, username_value, password_value, date_created, blacklisted_by_user, scheme) VALUES ('https://bank.test/', 'https://bank.test/', 'me', ?1, 0, 0, 0)", rusqlite::params![b"v20xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".to_vec()]).unwrap();
            db.execute("INSERT INTO logins (origin_url, signon_realm, username_value, password_value, date_created, blacklisted_by_user, scheme) VALUES ('https://never.test/', 'https://never.test/', '', x'', 0, 1, 0)", []).unwrap();
        }
        let mut r = Read::default();
        chromium_bookmarks(&profile, "Google Chrome", &mut r);
        chromium_history(&profile, &mut r);
        chromium_logins(&root.path, &profile, &mut r);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.bookmarks.len(), 3);
        assert_eq!(r.bar, vec![("Docs".to_string(), "https://docs.rs/".to_string()), ("Crates".to_string(), "https://crates.io/".to_string())]);
        assert!(r.bookmarks.iter().any(|b| b.folder == "Bookmarks bar / Dev" && b.url == "https://crates.io/"));
        assert_eq!(r.history, vec![("https://example.com/".to_string(), "Example".to_string(), 1_764_000_000_000, 4)]);
        assert_eq!(r.logins.len(), 1);
        assert_eq!((r.logins[0].username.as_str(), r.logins[0].password.as_str()), ("octo", "s3cret"));
        assert_eq!(r.logins_locked, 1, "app-bound passwords are counted, not read");
    }

    /// Logins written into a WebView2-shaped store read back with its key,
    /// and importing twice doesn't duplicate them.
    #[cfg(windows)]
    #[test]
    fn writes_into_a_webview2_store() {
        let data = tempdir::Dir::new().unwrap();
        let key = fake_local_state(&data.path);
        std::fs::create_dir_all(data.path.join("Default")).unwrap();
        let db_path = data.path.join("Default").join("Login Data");
        rusqlite::Connection::open(&db_path).unwrap().execute_batch(WEBVIEW2_LOGINS).unwrap();
        let logins = logins_from_csv("url,username,password\nhttps://github.com/login,octo,pw1\nhttps://a.test/,me,pw2\n");
        assert_eq!(write_logins(&data.path, &logins, std::time::Duration::from_secs(2)).unwrap(), 2);
        assert_eq!(write_logins(&data.path, &logins, std::time::Duration::from_secs(2)).unwrap(), 0, "no duplicates");
        let db = rusqlite::Connection::open(&db_path).unwrap();
        let mut q = db.prepare("SELECT origin_url, signon_realm, username_value, password_value, blacklisted_by_user, date_created FROM logins ORDER BY id").unwrap();
        let rows: Vec<(String, String, String, Vec<u8>, i64, i64)> = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))).unwrap().flatten().collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "https://github.com/login");
        assert_eq!(rows[0].1, "https://github.com/");
        assert_eq!(chromium_decrypt(&key, &rows[0].3), Ok("pw1".to_string()));
        assert_eq!(rows[0].4, 0);
        assert!(rows[0].5 > ms_to_webkit(1_700_000_000_000));
        assert!(data.path.join("Default").join("Login Data.serica-backup").is_file());
    }

    /// Firefox logins encrypted with AES-256 (current Firefox) decrypt.
    #[test]
    fn gecko_login_values() {
        use base64::Engine as _;
        use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
        let key = [5u8; 32];
        let iv = [6u8; 16];
        let ct = cbc::Encryptor::<aes::Aes256>::new_from_slices(&key, &iv).unwrap().encrypt_padded_vec_mut::<Pkcs7>(b"swordfish");
        fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
            let mut v = vec![tag, body.len() as u8];
            v.extend_from_slice(body);
            v
        }
        let der = tlv(0x30, &[tlv(0x04, CKA_ID), tlv(0x30, &[tlv(0x06, OID_AES256_CBC), tlv(0x04, &iv)].concat()), tlv(0x04, &ct)].concat());
        let b64 = base64::engine::general_purpose::STANDARD.encode(der);
        assert_eq!(gecko_decrypt(&key, &b64).as_deref(), Some("swordfish"));
        assert_ne!(gecko_decrypt(&[1u8; 32], &b64).as_deref(), Some("swordfish"));
    }

    #[test]
    fn gecko_profiles_ini() {
        let root = tempdir::Dir::new().unwrap();
        for p in ["Profiles\\a.default", "Profiles\\b.default-release"] {
            let d = root.path.join(p);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("places.sqlite"), b"").unwrap();
        }
        std::fs::write(
            root.path.join("profiles.ini"),
            "[Install308046B0AF4A39CB]\nDefault=Profiles/b.default-release\n\n[Profile1]\nName=default\nIsRelative=1\nPath=Profiles/a.default\n\n[Profile0]\nName=default-release\nIsRelative=1\nPath=Profiles/b.default-release\n",
        )
        .unwrap();
        let list = gecko_profiles(&root.path);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "default-release", "the install's default comes first");
        assert_eq!(list[0].id, "Profiles\\b.default-release");
    }

    #[test]
    fn webkit_time_round_trips() {
        let ms = 1_764_000_000_000;
        assert_eq!(webkit_to_ms(ms_to_webkit(ms)), ms);
        assert_eq!(webkit_to_ms(0), 0);
    }

    #[cfg(windows)]
    #[test]
    fn chromium_password_round_trip() {
        let key = [7u8; 32];
        let blob = chromium_encrypt(&key, "hunter2").unwrap();
        assert!(blob.starts_with(b"v10"));
        assert_eq!(chromium_decrypt(&key, &blob), Ok("hunter2".to_string()));
        assert_eq!(chromium_decrypt(&key, b"v20xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"), Err(true));
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_round_trip() {
        use windows::Win32::Foundation::{LocalFree, HLOCAL};
        use windows::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};
        let data = b"serica-secret";
        let protected = unsafe {
            let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
            let mut out = CRYPT_INTEGER_BLOB::default();
            CryptProtectData(&input, None, None, None, None, 0, &mut out).unwrap();
            let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
            v
        };
        assert_eq!(dpapi_unprotect(&protected).unwrap(), data);
    }

    /// An NSS PBES2 item built the way Firefox writes it decrypts.
    #[test]
    fn nss_pbes2() {
        use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
        use sha1::{Digest, Sha1};
        let global_salt = b"global-salt-0123";
        let entry_salt = [9u8; 32];
        let iv14 = [3u8; 14];
        let iterations: u32 = 1000;
        let mut k = Sha1::new();
        k.update(global_salt);
        let k = k.finalize();
        let mut key = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<sha2::Sha256>(&k, &entry_salt, iterations, &mut key);
        let mut iv = vec![0x04, 0x0E];
        iv.extend_from_slice(&iv14);
        let plain = b"password-check\x02\x02";
        let ct = cbc::Encryptor::<aes::Aes256>::new_from_slices(&key, &iv).unwrap().encrypt_padded_vec_mut::<Pkcs7>(&plain[..14]);
        // DER: SEQ { SEQ { OID pbes2, SEQ { SEQ { OID pbkdf2, SEQ { OCTET salt, INT iter, INT 32, SEQ { OID hmac } } }, SEQ { OID aes, OCTET iv } } }, OCTET ct }
        fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
            let mut v = vec![tag];
            if body.len() < 128 {
                v.push(body.len() as u8);
            } else {
                v.push(0x81);
                v.push(body.len() as u8);
            }
            v.extend_from_slice(body);
            v
        }
        let hmac_oid = [0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x02, 0x09];
        let pbkdf2_oid = [0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x05, 0x0C];
        let kdf_params = tlv(0x30, &[tlv(0x04, &entry_salt), tlv(0x02, &[0x03, 0xE8]), tlv(0x02, &[32]), tlv(0x30, &tlv(0x06, &hmac_oid))].concat());
        let kdf = tlv(0x30, &[tlv(0x06, &pbkdf2_oid), kdf_params].concat());
        let enc = tlv(0x30, &[tlv(0x06, OID_AES256_CBC), tlv(0x04, &iv14)].concat());
        let algo = tlv(0x30, &[tlv(0x06, OID_PBES2), tlv(0x30, &[kdf, enc].concat())].concat());
        let item = tlv(0x30, &[algo, tlv(0x04, &ct)].concat());
        assert_eq!(nss_pbe(&item, global_salt, b"").unwrap(), b"password-check".to_vec());
        assert_ne!(nss_pbe(&item, global_salt, b"wrong"), Some(b"password-check".to_vec()));
    }
}
