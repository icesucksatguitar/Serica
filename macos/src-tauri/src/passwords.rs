//! Saved passwords on macOS.
//!
//! On Windows, WebView2's own password manager saves sign-ins and fills them
//! in (and imported passwords go into its store). WebKit gives apps no such
//! thing, so Serica keeps them itself, the way Chromium does on the Mac: one
//! random key in the user's login Keychain ("Serica Safe Storage"; the test
//! copy has its own, "Serica Test Safe Storage"), and each profile's
//! passwords in a file encrypted with it (AES-256-GCM).
//!
//! A small script in Serica's own script world (out of the page's reach)
//! fills a site's saved sign-in into its login form, and offers to save or
//! update a password after a sign-in, through the UI's password prompt.
//! The site a password belongs to is WebKit's origin of the frame, never
//! what a page says, and sign-ins are filled only into frames of the page's
//! own site.

use crate::app::{ui_emit, Shared};
use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use base64::Engine as _;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// `errSecItemNotFound`: no key yet (the only case a new one is made).
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    /// `https://host[:port]`
    pub origin: String,
    #[serde(default)]
    pub action: String,
    pub username: String,
    pub password: String,
    pub created_at: i64,
    #[serde(default)]
    pub last_used_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Vault {
    entries: Vec<Entry>,
    /// Sites the user said never to save passwords for.
    never: Vec<String>,
}

struct Pending {
    profile: String,
    origin: String,
    username: String,
    password: String,
    update_id: Option<String>,
}

pub struct Passwords {
    dir: PathBuf,
    /// The Keychain item holding the key (service, account).
    service: String,
    account: String,
    key: Mutex<Option<Option<[u8; 32]>>>,
    vaults: Mutex<HashMap<String, Vault>>,
    prompts: Mutex<HashMap<String, Pending>>,
}

/// `scheme://host[:port]` of a URL, for http(s) only.
pub fn origin_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    matches!(u.scheme(), "http" | "https").then(|| u.origin().ascii_serialization())
}

fn origin_from_parts(proto: &str, host: &str, port: u16) -> Option<String> {
    if !matches!(proto, "http" | "https") || host.is_empty() {
        return None;
    }
    let default = (proto == "https" && port == 443) || (proto == "http" && port == 80) || port == 0;
    Some(if default { format!("{proto}://{host}") } else { format!("{proto}://{host}:{port}") })
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

impl Passwords {
    /// `app_name` names the Keychain item ("Serica" → "Serica Safe Storage"),
    /// so the test copy never touches the real browser's key.
    pub fn new(dir: PathBuf, app_name: &str) -> Self {
        Self {
            dir,
            service: format!("{app_name} Safe Storage"),
            account: app_name.to_string(),
            key: Mutex::new(None),
            vaults: Mutex::new(HashMap::new()),
            prompts: Mutex::new(HashMap::new()),
        }
    }

    /// The vault key from the Keychain, made on first use. Only a missing
    /// item gets a new key: any other failure (access denied, a locked
    /// keychain) leaves the stored key alone and passwords off for now, as
    /// replacing it would make every saved password unreadable.
    fn key(&self) -> Option<[u8; 32]> {
        let mut k = self.key.lock();
        if let Some(v) = *k {
            return v;
        }
        let key = match security_framework::passwords::get_generic_password(&self.service, &self.account) {
            Ok(b) => {
                let key = base64::engine::general_purpose::STANDARD.decode(b).ok().and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok());
                if key.is_none() {
                    tracing::warn!("the password key in the Keychain is malformed; passwords stay off");
                }
                key
            }
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => {
                let fresh: [u8; 32] = aes_gcm::Aes256Gcm::generate_key(&mut OsRng).into();
                let enc = base64::engine::general_purpose::STANDARD.encode(fresh);
                match security_framework::passwords::set_generic_password(&self.service, &self.account, enc.as_bytes()) {
                    Ok(()) => Some(fresh),
                    Err(e) => {
                        tracing::warn!("could not store the password key in the Keychain: {e}");
                        None
                    }
                }
            }
            Err(e) => {
                // Not remembered: the next use asks the Keychain again.
                tracing::warn!("could not read the password key from the Keychain: {e}");
                return None;
            }
        };
        *k = Some(key);
        key
    }

    fn file(&self, profile: &str) -> PathBuf {
        self.dir.join(format!("{}.bin", sanitize(profile)))
    }

    fn with_vault<T>(&self, profile: &str, f: impl FnOnce(&mut Vault) -> T) -> Option<T> {
        let key = self.key()?;
        let mut vaults = self.vaults.lock();
        if !vaults.contains_key(profile) {
            let v = std::fs::read(self.file(profile)).ok().and_then(|bytes| decrypt(&key, &bytes)).unwrap_or_default();
            vaults.insert(profile.to_string(), v);
        }
        let vault = vaults.get_mut(profile)?;
        Some(f(vault))
    }

    fn save(&self, profile: &str) {
        let Some(key) = self.key() else { return };
        let Some(v) = self.vaults.lock().get(profile).cloned() else { return };
        let Some(bytes) = encrypt(&key, &v) else { return };
        let _ = std::fs::create_dir_all(&self.dir);
        let file = self.file(profile);
        if let Err(e) = crate::store::write_atomic(&file, &bytes) {
            tracing::warn!("could not save passwords: {e}");
            return;
        }
        // Encrypted, and still only the user's to read.
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700));
        let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
    }

    pub fn available(&self) -> bool {
        self.key().is_some()
    }

    pub fn list(&self, profile: &str) -> Vec<Entry> {
        self.with_vault(profile, |v| v.entries.clone()).unwrap_or_default()
    }

    /// Adds logins brought over from another browser or a file; a site and
    /// user name already saved are kept as they are. Returns how many were added.
    pub fn import(&self, profile: &str, logins: &[crate::browser_import::Login]) -> Result<usize, String> {
        if !self.available() {
            return Err("encryption-unavailable".into());
        }
        let now = crate::model::now_ms();
        let added = self
            .with_vault(profile, |v| {
                let mut n = 0;
                for l in logins {
                    let Some(origin) = origin_of(&l.origin).or_else(|| origin_of(&l.realm)) else { continue };
                    if l.password.is_empty() || v.entries.iter().any(|e| e.origin == origin && e.username == l.username) {
                        continue;
                    }
                    v.entries.push(Entry {
                        id: uuid::Uuid::new_v4().to_string(),
                        origin,
                        action: l.action.clone(),
                        username: l.username.clone(),
                        password: l.password.clone(),
                        created_at: if l.created_ms > 0 { l.created_ms } else { now },
                        last_used_at: 0,
                    });
                    n += 1;
                }
                n
            })
            .unwrap_or(0);
        if added > 0 {
            self.save(profile);
        }
        Ok(added)
    }

    pub fn delete(&self, profile: &str, id: &str) -> bool {
        let removed = self
            .with_vault(profile, |v| {
                let before = v.entries.len();
                v.entries.retain(|e| e.id != id);
                before != v.entries.len()
            })
            .unwrap_or(false);
        if removed {
            self.save(profile);
        }
        removed
    }

    /// A profile was deleted: its passwords go with it.
    pub fn remove_profile(&self, profile: &str) {
        self.vaults.lock().remove(profile);
        let _ = std::fs::remove_file(self.file(profile));
    }
}

fn encrypt(key: &[u8; 32], v: &Vault) -> Option<Vec<u8>> {
    let cipher = aes_gcm::Aes256Gcm::new_from_slice(key).ok()?;
    let nonce = aes_gcm::Aes256Gcm::generate_nonce(&mut OsRng);
    let plain = serde_json::to_vec(v).ok()?;
    let mut out = b"SPW1".to_vec();
    out.extend_from_slice(&nonce);
    out.extend(cipher.encrypt(&nonce, plain.as_slice()).ok()?);
    Some(out)
}

fn decrypt(key: &[u8; 32], bytes: &[u8]) -> Option<Vault> {
    let rest = bytes.strip_prefix(b"SPW1")?;
    if rest.len() < 12 + 16 {
        return None;
    }
    let cipher = aes_gcm::Aes256Gcm::new_from_slice(key).ok()?;
    let plain = cipher.decrypt(aes_gcm::Nonce::from_slice(&rest[..12]), &rest[12..]).ok()?;
    serde_json::from_slice(&plain).ok()
}

// ------------------------------------------------------------------ pages

/// Serica's sign-in helper for pages (Serica world, every frame).
fn script() -> String {
    r#"(() => {
  if (window.__sericaPw || !/^https?:$/.test(location.protocol)) return;
  Object.defineProperty(window, '__sericaPw', { value: true });
  const post = (m) => { try { return window.webkit.messageHandlers.serica.postMessage(Object.assign({ __serica: 1 }, m)).catch(() => null); } catch (e) { return Promise.resolve(null); } };
  const visible = (el) => !!(el && (el.offsetWidth || el.offsetHeight || el.getClientRects().length));
  const isUser = (el) => el && el.tagName === 'INPUT' && /^(text|email|tel|)$/i.test(el.getAttribute('type') || '') && !el.disabled && !el.readOnly && visible(el);
  const passwords = () => Array.from(document.querySelectorAll('input[type=password]')).filter((p) => !p.disabled && !p.readOnly && visible(p));
  const userFor = (pw) => {
    const scope = pw.form || document;
    const named = scope.querySelector('input[autocomplete~="username" i]');
    if (named && visible(named)) return named;
    const inputs = Array.from(scope.querySelectorAll('input'));
    for (let j = inputs.indexOf(pw) - 1; j >= 0; j--) if (isUser(inputs[j])) return inputs[j];
    return null;
  };
  const setValue = (el, v) => {
    const desc = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value');
    desc.set.call(el, v);
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  };
  const filled = new WeakSet();
  let creds, asked = false, busy = false;
  const fill = async () => {
    if (busy) return;
    const pws = passwords().filter((p) => !filled.has(p) && !p.value && !/new-password/i.test(p.getAttribute('autocomplete') || ''));
    if (!pws.length) return;
    busy = true;
    try {
      if (!asked) { asked = true; creds = await post({ t: 'pw-query' }); }
      if (!Array.isArray(creds) || !creds.length) return;
      const c = await post({ t: 'pw-get', id: creds[0].id });
      if (!c || !c.password) return;
      for (const p of pws) {
        filled.add(p);
        const u = userFor(p);
        if (u && !u.value && c.username) setValue(u, c.username);
        setValue(p, c.password);
      }
    } finally { busy = false; }
  };
  let last = null;
  const capture = (pw) => { if (pw.value) { const u = userFor(pw); last = { username: u ? u.value : '', password: pw.value }; } };
  const submit = () => { const l = last; last = null; if (l && l.password) post({ t: 'pw-submit', username: l.username, password: l.password }); };
  document.addEventListener('input', (e) => { const t = e.target; if (t && t.tagName === 'INPUT' && t.type === 'password') capture(t); }, true);
  document.addEventListener('change', (e) => { const t = e.target; if (t && t.tagName === 'INPUT' && t.type === 'password') capture(t); }, true);
  document.addEventListener('submit', () => { for (const p of passwords()) capture(p); submit(); }, true);
  document.addEventListener('click', (e) => {
    const b = e.target && e.target.closest && e.target.closest('button, input[type=submit], [role=button]');
    if (b && last) setTimeout(submit, 0);
  }, true);
  document.addEventListener('keydown', (e) => { if (e.key === 'Enter' && e.target && e.target.type === 'password') { capture(e.target); setTimeout(submit, 0); } }, true);
  addEventListener('pagehide', submit);
  let timer = 0;
  const start = () => {
    fill();
    new MutationObserver(() => { clearTimeout(timer); timer = setTimeout(fill, 250); }).observe(document.documentElement, { childList: true, subtree: true });
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start, { once: true });
  else start();
})();"#
        .to_string()
}

/// Puts the sign-in helper in a page configuration (not in private tabs:
/// nothing is saved there).
pub fn install(ucc: &objc2_web_kit::WKUserContentController, enabled: bool) {
    if !enabled {
        return;
    }
    let mtm = objc2::MainThreadMarker::new().expect("main thread");
    let world = unsafe { objc2_web_kit::WKContentWorld::worldWithName(&crate::mac_util::ns("serica"), mtm) };
    unsafe {
        let s = objc2_web_kit::WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
            mtm.alloc::<objc2_web_kit::WKUserScript>(),
            &crate::mac_util::ns(&script()),
            objc2_web_kit::WKUserScriptInjectionTime::AtDocumentEnd,
            false,
            &world,
        );
        ucc.addUserScript(&s);
    }
}

/// A sign-in message from a page (`pw-query`, `pw-get`, `pw-submit`). The
/// frame's origin comes from WebKit; `top_url` is the tab's page.
#[allow(clippy::too_many_arguments)]
pub fn message(app: &Shared, tab_id: &str, profile: &str, proto: &str, host: &str, port: u16, top_url: &str, body: &serde_json::Value) -> Option<serde_json::Value> {
    let origin = origin_from_parts(proto, host, port)?;
    // Filling only on the page's own site (not into frames from elsewhere).
    let same_site = origin_of(top_url).as_deref() == Some(origin.as_str());
    let pw = &app.passwords;
    match body["t"].as_str()? {
        "pw-query" => {
            if !same_site {
                return Some(serde_json::json!([]));
            }
            let mut list: Vec<Entry> = pw.list(profile).into_iter().filter(|e| e.origin == origin).collect();
            list.sort_by(|a, b| b.last_used_at.max(b.created_at).cmp(&a.last_used_at.max(a.created_at)));
            Some(serde_json::Value::Array(list.into_iter().map(|e| serde_json::json!({ "id": e.id, "username": e.username })).collect()))
        }
        "pw-get" => {
            if !same_site {
                return None;
            }
            let id = body["id"].as_str()?.to_string();
            let found = pw.with_vault(profile, |v| {
                let e = v.entries.iter_mut().find(|e| e.id == id && e.origin == origin)?;
                e.last_used_at = crate::model::now_ms();
                Some(serde_json::json!({ "username": e.username, "password": e.password }))
            })??;
            pw.save(profile);
            Some(found)
        }
        "pw-submit" => {
            let username: String = body["username"].as_str().unwrap_or("").chars().take(500).collect();
            let password: String = body["password"].as_str().unwrap_or("").chars().take(1000).collect();
            if password.is_empty() || !pw.available() {
                return None;
            }
            let (never, existing) = pw
                .with_vault(profile, |v| {
                    let never = v.never.contains(&origin);
                    let existing = v.entries.iter().find(|e| e.origin == origin && e.username == username).map(|e| (e.id.clone(), e.password == password));
                    (never, existing)
                })
                .unwrap_or((true, None));
            if never || matches!(existing, Some((_, true))) {
                return None;
            }
            let (kind, update_id) = match existing {
                Some((id, false)) => ("update", Some(id)),
                _ => ("save", None),
            };
            let prompt_id = uuid::Uuid::new_v4().to_string();
            let replaced: Vec<String> = {
                let mut prompts = pw.prompts.lock();
                // One unanswered prompt per site and user.
                let replaced: Vec<String> = prompts.iter().filter(|(_, p)| p.profile == profile && p.origin == origin && p.username == username).map(|(id, _)| id.clone()).collect();
                for id in &replaced {
                    prompts.remove(id);
                }
                prompts.insert(prompt_id.clone(), Pending { profile: profile.to_string(), origin: origin.clone(), username: username.clone(), password, update_id });
                replaced
            };
            for id in replaced {
                ui_emit(&app.handle, "serica:password-prompt-closed", serde_json::json!({ "promptId": id }));
            }
            ui_emit(
                &app.handle,
                "serica:password-prompt",
                serde_json::json!({ "promptId": prompt_id, "kind": kind, "profileId": profile, "webContentsId": -1, "origin": origin, "username": username, "tabId": tab_id }),
            );
            None
        }
        _ => None,
    }
}

// ------------------------------------------------------------------ commands

fn active_profile(app: &Shared) -> String {
    let s = app.store.lock();
    s.space_profile(&s.active_space_id)
}

#[tauri::command]
pub async fn passwords_status(app: tauri::State<'_, Shared>) -> Result<serde_json::Value, String> {
    let ok = app.passwords.available();
    Ok(if ok {
        serde_json::json!({ "ok": true, "encryptionAvailable": true })
    } else {
        serde_json::json!({ "ok": false, "error": "encryption-unavailable", "message": "Serica couldn't reach your Keychain to keep passwords safe." })
    })
}

#[tauri::command]
pub async fn passwords_list(app: tauri::State<'_, Shared>) -> Result<Vec<serde_json::Value>, String> {
    let profile = active_profile(app.inner());
    Ok(app
        .passwords
        .list(&profile)
        .into_iter()
        .map(|e| serde_json::json!({ "id": e.id, "origin": e.origin, "username": e.username, "createdAt": e.created_at, "lastUsedAt": e.last_used_at, "unreadable": false }))
        .collect())
}

#[tauri::command]
pub async fn delete_password(id: String, app: tauri::State<'_, Shared>) -> Result<bool, String> {
    let profile = active_profile(app.inner());
    Ok(app.passwords.delete(&profile, &id))
}

/// The user's answer to a save / update prompt.
#[tauri::command]
pub async fn respond_to_password_prompt(prompt_id: String, action: String, app: tauri::State<'_, Shared>) -> Result<serde_json::Value, String> {
    let pw = &app.passwords;
    let Some(p) = pw.prompts.lock().remove(&prompt_id) else {
        return Ok(serde_json::json!({ "ok": false, "error": "prompt-expired", "message": "This prompt is no longer active." }));
    };
    let now = crate::model::now_ms();
    let credential = match action.as_str() {
        "save" | "update" => pw.with_vault(&p.profile, |v| {
            let id = match p.update_id.as_ref().and_then(|id| v.entries.iter_mut().find(|e| &e.id == id)) {
                Some(e) => {
                    e.password = p.password.clone();
                    e.last_used_at = now;
                    e.id.clone()
                }
                None => {
                    let id = uuid::Uuid::new_v4().to_string();
                    v.entries.push(Entry { id: id.clone(), origin: p.origin.clone(), action: String::new(), username: p.username.clone(), password: p.password.clone(), created_at: now, last_used_at: now });
                    id
                }
            };
            serde_json::json!({ "id": id, "origin": p.origin, "username": p.username, "createdAt": now, "lastUsedAt": now, "unreadable": false })
        }),
        "never" => pw.with_vault(&p.profile, |v| {
            if !v.never.contains(&p.origin) {
                v.never.push(p.origin.clone());
            }
            serde_json::Value::Null
        }),
        "not-now" | "dismiss" => return Ok(serde_json::json!({ "ok": true, "action": action })),
        _ => return Ok(serde_json::json!({ "ok": false, "error": "invalid-action", "message": "Unknown action." })),
    };
    let Some(credential) = credential else {
        return Ok(serde_json::json!({ "ok": false, "error": "encryption-unavailable", "message": "Serica couldn't reach your Keychain to keep passwords safe." }));
    };
    pw.save(&p.profile);
    let mut out = serde_json::json!({ "ok": true, "action": action });
    if !credential.is_null() {
        out["credential"] = credential;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_round_trip_and_tamper() {
        let key = [9u8; 32];
        let v = Vault {
            entries: vec![Entry { id: "1".into(), origin: "https://a.test".into(), action: String::new(), username: "me".into(), password: "pw".into(), created_at: 1, last_used_at: 0 }],
            never: vec!["https://b.test".into()],
        };
        let mut bytes = encrypt(&key, &v).unwrap();
        let back = decrypt(&key, &bytes).unwrap();
        assert_eq!(back.entries, v.entries);
        assert_eq!(back.never, v.never);
        assert!(decrypt(&[1u8; 32], &bytes).is_none());
        let n = bytes.len() - 1;
        bytes[n] ^= 1;
        assert!(decrypt(&key, &bytes).is_none());
    }

    #[test]
    fn origins() {
        assert_eq!(origin_of("https://github.com/login?x=1").as_deref(), Some("https://github.com"));
        assert_eq!(origin_of("http://localhost:8080/").as_deref(), Some("http://localhost:8080"));
        assert!(origin_of("file:///x").is_none());
        assert_eq!(origin_from_parts("https", "a.test", 443).as_deref(), Some("https://a.test"));
        assert_eq!(origin_from_parts("https", "a.test", 8443).as_deref(), Some("https://a.test:8443"));
        assert!(origin_from_parts("file", "", 0).is_none());
    }

    #[test]
    fn script_uses_the_serica_world_bridge() {
        let s = script();
        assert!(s.contains("window.webkit.messageHandlers.serica.postMessage"));
        assert!(s.contains("pw-submit"));
    }
}
