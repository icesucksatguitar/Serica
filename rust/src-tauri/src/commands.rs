//! IPC commands exposed to the UI (`window.neutrino` in `src/adapter/tauriBridge.ts`).

use crate::app::{normalize_input, ui_emit, Notice, Shared};
use crate::history::{HistoryEntry, HistoryQuery};
use crate::model::*;
use crate::settings::{BrowserSettings, PerformanceStatus};
use crate::store::MoveOp;
use crate::tabhost::{Bounds, TabHost};
use serde_json::{json, Value};
use std::collections::HashMap;
use tauri::{State, Window};

type R<T> = Result<T, String>;

fn ok() -> Value {
    json!({ "ok": true })
}
fn err(code: &str) -> Value {
    json!({ "ok": false, "error": code })
}

/// Awaits a callback-style native operation.
async fn await_cb<T: Send + 'static>(start: impl FnOnce(Box<dyn FnOnce(T) + Send>)) -> Option<T> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    start(Box::new(move |v| {
        let _ = tx.send(v);
    }));
    tokio::time::timeout(std::time::Duration::from_secs(5), rx).await.ok()?.ok()
}

// ------------------------------------------------------------------ state

#[tauri::command]
pub async fn get_browser_state(app: State<'_, Shared>) -> R<BrowserAppState> {
    Ok(app.snapshot())
}

#[tauri::command]
pub async fn create_tab(url: Option<String>, space_id: Option<String>, app: State<'_, Shared>) -> R<TabState> {
    let prefix = app.settings().search_prefix();
    let url = url.and_then(|u| normalize_input(&u, prefix));
    Ok(app.mutate(|s| s.create_tab(url, space_id, true, None)))
}

#[tauri::command]
pub async fn close_tab(tab_id: String, app: State<'_, Shared>) -> R<()> {
    app.mutate(|s| s.close_tab(&tab_id));
    Ok(())
}

#[tauri::command]
pub async fn select_tab(tab_id: String, app: State<'_, Shared>) -> R<()> {
    app.mutate(|s| s.activate_tab(&tab_id));
    Ok(())
}

#[tauri::command]
pub async fn reopen_closed_tab(app: State<'_, Shared>) -> R<Option<TabState>> {
    Ok(app.mutate(|s| s.reopen_closed_tab()))
}

#[tauri::command]
pub async fn cycle_tab(direction: i32, app: State<'_, Shared>) -> R<Option<TabState>> {
    Ok(app.mutate(|s| s.cycle_tab(direction)))
}

/// UI-originated tab updates. Navigation state is owned by the webview, so
/// only presentation fields are accepted here.
#[tauri::command]
pub async fn update_tab(tab_id: String, partial: Value, app: State<'_, Shared>) -> R<()> {
    let zoom = partial.get("zoomFactor").and_then(Value::as_f64);
    app.mutate(|s| {
        if let Some(t) = s.tab_mut(&tab_id) {
            if let Some(title) = partial.get("title").and_then(Value::as_str) {
                if is_internal_url(&t.url) {
                    t.title = title.to_string();
                }
            }
            if partial.get("isLoading") == Some(&Value::Bool(false)) {
                t.is_loading = false;
            }
        }
    });
    if let Some(z) = zoom {
        set_zoom_factor(z, Some(tab_id), app).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn toggle_pin_tab(tab_id: String, app: State<'_, Shared>) -> R<()> {
    app.mutate(|s| {
        let pinned = s.tab(&tab_id).map(|t| t.is_pinned).unwrap_or(false);
        s.set_pinned(&tab_id, !pinned)
    });
    Ok(())
}

#[tauri::command]
pub async fn pin_tab(tab_id: String, app: State<'_, Shared>) -> R<Value> {
    Ok(if app.mutate(|s| s.set_pinned(&tab_id, true)) { ok() } else { err("not-found") })
}

#[tauri::command]
pub async fn unpin_tab(tab_id: String, app: State<'_, Shared>) -> R<Value> {
    Ok(if app.mutate(|s| s.set_pinned(&tab_id, false)) { ok() } else { err("not-found") })
}

#[tauri::command]
pub async fn toggle_sleep_tab(tab_id: String, app: State<'_, Shared>) -> R<()> {
    app.mutate(|s| {
        let is_active = s.active_tab_id.as_deref() == Some(tab_id.as_str());
        if let Some(t) = s.tab_mut(&tab_id) {
            if t.is_sleeping {
                t.is_sleeping = false;
                t.last_active_at = Some(now_ms());
            } else if !is_internal_url(&t.url) && !t.unloaded {
                t.is_sleeping = true;
                t.is_loading = false;
                t.has_active_media = false;
            }
            let _ = is_active;
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn reorder_tabs(dragged_tab_id: String, target_tab_id: String, app: State<'_, Shared>) -> R<()> {
    app.mutate(|s| {
        let (Some(d), Some(t)) = (s.tab(&dragged_tab_id).cloned(), s.tab(&target_tab_id).cloned()) else { return };
        let section = if t.is_pinned { "pinned" } else { "today" };
        let order = s.visible_order(&t.space_id);
        let list: Vec<&String> = order.iter().filter(|id| s.tab(id).map(|x| x.is_pinned == t.is_pinned).unwrap_or(false)).collect();
        let to = list.iter().position(|id| **id == target_tab_id).unwrap_or(0) as i64;
        if d.is_pinned == t.is_pinned && d.space_id == t.space_id {
            s.apply_move(&MoveOp::Reorder { section: section.into(), id: dragged_tab_id.clone(), to_index: to });
        } else {
            let from = if d.is_pinned { "pinned" } else { "today" };
            s.apply_move(&MoveOp::Section { id: dragged_tab_id.clone(), from: from.into(), to: section.into(), space_id: t.space_id.clone(), to_index: Some(to) });
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn duplicate_tab(id: String, space_id: Option<String>, app: State<'_, Shared>) -> R<Value> {
    Ok(match app.mutate(|s| s.duplicate_tab(&id, space_id)) {
        Some(tab) => json!({ "ok": true, "tab": tab }),
        None => err("not-found"),
    })
}

#[tauri::command]
pub async fn move_tab(op: MoveOp, app: State<'_, Shared>) -> R<Value> {
    Ok(match app.mutate(|s| s.apply_move(&op)) {
        Some(needs_reload) => json!({ "ok": true, "needsReload": needs_reload }),
        None => err("invalid-move"),
    })
}

#[tauri::command]
pub async fn add_favorite(tab_id: String, app: State<'_, Shared>) -> R<Value> {
    Ok(if app.mutate(|s| s.add_favorite(&tab_id)) { ok() } else { err("not-found") })
}

#[tauri::command]
pub async fn remove_favorite(favorite_id: String, app: State<'_, Shared>) -> R<Value> {
    Ok(if app.mutate(|s| s.remove_favorite(&favorite_id)) { ok() } else { err("not-found") })
}

#[tauri::command]
pub async fn create_folder(space_id: String, name: String, app: State<'_, Shared>) -> R<Value> {
    Ok(match app.mutate(|s| s.create_folder(&space_id, &name)) {
        Some(f) => json!({ "ok": true, "folder": f }),
        None => err("unknown-space"),
    })
}

#[tauri::command]
pub async fn update_folder(folder_id: String, patch: Value, app: State<'_, Shared>) -> R<Value> {
    Ok(match app.mutate(|s| s.update_folder(&folder_id, &patch)) {
        Some(f) => json!({ "ok": true, "folder": f }),
        None => err("not-found"),
    })
}

#[tauri::command]
pub async fn delete_folder(folder_id: String, app: State<'_, Shared>) -> R<Value> {
    Ok(if app.mutate(|s| s.delete_folder(&folder_id)) { ok() } else { err("not-found") })
}

#[tauri::command]
pub async fn move_tab_to_folder(tab_id: String, folder_id: Option<String>, app: State<'_, Shared>) -> R<Value> {
    Ok(if app.mutate(|s| s.move_tab_to_folder(&tab_id, folder_id.as_deref())) { ok() } else { err("not-found") })
}

// ------------------------------------------------------------------ navigation

/// Navigates the active (or given) tab. Accepts URLs or search text.
#[tauri::command]
pub async fn navigate(url: String, tab_id: Option<String>, app: State<'_, Shared>) -> R<()> {
    do_navigate(app.inner(), &url, tab_id);
    Ok(())
}

pub fn do_navigate(app: &Shared, url: &str, tab_id: Option<String>) {
    let prefix = app.settings().search_prefix();
    let Some(target) = normalize_input(url, prefix) else { return };
    let tab_id = match tab_id.or_else(|| app.store.lock().active_tab_id.clone()) {
        Some(id) => id,
        None => return,
    };
    let internal = is_internal_url(&target);
    app.mutate(|s| {
        if let Some(t) = s.tab_mut(&tab_id) {
            t.url = target.clone();
            t.is_sleeping = false;
            t.unloaded = false;
            if internal {
                t.title = "New Tab".into();
                t.favicon = None;
                t.is_loading = false;
                t.can_go_back = false;
                t.can_go_forward = false;
            } else {
                t.is_loading = true;
            }
        }
    });
    if !internal {
        TabHost::navigate(app, &tab_id, &target);
    }
}

/// Back/forward/reload/hard-reload/stop/home for the active tab.
#[tauri::command]
pub async fn site_action(action: String, app: State<'_, Shared>) -> R<()> {
    let app = app.inner().clone();
    if action == "home" {
        do_navigate(&app, NEW_TAB_URL, None);
        return Ok(());
    }
    let Some(wv) = TabHost::active_webview(&app) else {
        // Unloaded/sleeping/new tab: reload re-creates the page.
        if action == "reload" || action == "hard-reload" {
            app.mutate(|s| {
                if let Some(id) = s.active_tab_id.clone() {
                    s.activate_tab(&id);
                }
            });
        }
        return Ok(());
    };
    if action == "stop" {
        app.mutate(|s| {
            if let Some(t) = s.active_tab_id.clone().and_then(|id| s.tab_mut(&id).map(|t| t.id.clone())) {
                if let Some(t) = s.tab_mut(&t) {
                    t.is_loading = false;
                }
            }
        });
    }
    #[cfg(windows)]
    {
        let a: &'static str = match action.as_str() {
            "back" => "back",
            "forward" => "forward",
            "reload" => "reload",
            "hard-reload" => "hard-reload",
            "stop" => "stop",
            _ => return Ok(()),
        };
        crate::webview_win::nav(&wv, a);
    }
    #[cfg(not(windows))]
    {
        match action.as_str() {
            "reload" | "hard-reload" => {
                let _ = wv.reload();
            }
            "back" => {
                let _ = wv.eval("history.back()");
            }
            "forward" => {
                let _ = wv.eval("history.forward()");
            }
            "stop" => {
                let _ = wv.eval("window.stop()");
            }
            _ => {}
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn open_external_url(url: String) -> R<bool> {
    let parsed = url::Url::parse(&url).map_err(|e| e.to_string())?;
    if !matches!(parsed.scheme(), "http" | "https" | "mailto") {
        return Err("unsupported url".into());
    }
    Ok(open::that_detached(parsed.as_str()).is_ok())
}

// ------------------------------------------------------------------ viewport

#[tauri::command]
pub async fn set_viewport(x: f64, y: f64, width: f64, height: f64, app: State<'_, Shared>) -> R<()> {
    if [x, y, width, height].iter().all(|v| v.is_finite()) {
        TabHost::set_bounds(app.inner(), Bounds { x, y, w: width.max(0.0), h: height.max(0.0) });
    }
    Ok(())
}

/// Captures the active page, hides the native webview and returns the image
/// for the UI to show behind an overlay.
#[tauri::command]
pub async fn viewport_freeze(app: State<'_, Shared>) -> R<Option<String>> {
    let app = app.inner().clone();
    let mut image = None;
    #[cfg(windows)]
    if let Some(wv) = TabHost::active_webview(&app) {
        if app.host.lock().visible {
            image = await_cb(|done| crate::webview_win::capture(&wv, done)).await.flatten();
        }
    }
    let a2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || TabHost::set_visible(&a2, false)).await.map_err(|e| e.to_string())?;
    if let (Some(img), Some(id)) = (&image, app.store.lock().active_tab_id.clone()) {
        app.host.lock().previews.insert(id, img.clone());
    }
    Ok(image)
}

#[tauri::command]
pub async fn viewport_thaw(app: State<'_, Shared>) -> R<()> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || TabHost::set_visible(&app, true)).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn find_in_page(query: Option<String>, forward: Option<bool>, find_next: Option<bool>, app: State<'_, Shared>) -> R<()> {
    #[cfg(windows)]
    if let Some(wv) = TabHost::active_webview(app.inner()) {
        let q = query.filter(|q| !q.is_empty());
        crate::webview_win::find(&wv, q, forward.unwrap_or(true), find_next.unwrap_or(false));
    }
    #[cfg(not(windows))]
    let _ = (query, forward, find_next, app);
    Ok(())
}

#[tauri::command]
pub async fn set_zoom_factor(factor: f64, tab_id: Option<String>, app: State<'_, Shared>) -> R<()> {
    let factor = if factor.is_finite() { factor.clamp(0.25, 5.0) } else { 1.0 };
    let app = app.inner().clone();
    let id = tab_id.or_else(|| app.store.lock().active_tab_id.clone());
    if let Some(id) = id {
        app.mutate(|s| {
            if let Some(t) = s.tab_mut(&id) {
                t.zoom_factor = Some(factor);
            }
        });
        if let Some(wv) = TabHost::webview(&app, &id) {
            #[cfg(windows)]
            crate::webview_win::set_zoom(&wv, factor);
            #[cfg(not(windows))]
            let _ = wv.set_zoom(factor);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn capture_tab_preview(tab_id: String, app: State<'_, Shared>) -> R<Option<String>> {
    let app = app.inner().clone();
    #[cfg(windows)]
    {
        let visible = app.host.lock().views.get(&tab_id).map(|v| v.shown).unwrap_or(false);
        if visible {
            if let Some(wv) = TabHost::webview(&app, &tab_id) {
                if let Some(Some(img)) = await_cb(|done| crate::webview_win::capture(&wv, done)).await {
                    app.host.lock().previews.insert(tab_id.clone(), img.clone());
                    return Ok(Some(img));
                }
            }
        }
    }
    let preview = app.host.lock().previews.get(&tab_id).cloned();
    Ok(preview)
}

#[tauri::command]
pub async fn get_tab_preview(tab_id: String, app: State<'_, Shared>) -> R<Option<String>> {
    Ok(app.host.lock().previews.get(&tab_id).cloned())
}

#[tauri::command]
pub async fn open_devtools(app: State<'_, Shared>) -> R<()> {
    if let Some(wv) = TabHost::active_webview(app.inner()) {
        wv.open_devtools();
    }
    Ok(())
}

// ------------------------------------------------------------------ media

const MEDIA_STATE_JS: &str = r#"(() => {
  const els = Array.from(document.querySelectorAll('video, audio'));
  const m = els.find((e) => !e.paused && !e.ended) || els.find((e) => e.currentTime > 0) || null;
  if (!m) return null;
  const d = m.duration;
  const live = d === Infinity || isNaN(d);
  return { hasVideo: m.tagName === 'VIDEO', isPlaying: !m.paused && !m.ended, currentTime: m.currentTime || 0,
           duration: live ? 0 : (d || 0), volume: m.muted ? 0 : (m.volume ?? 1), tabTitle: document.title || 'Media' };
})()"#;

#[tauri::command]
pub async fn get_media_state(tab_id: String, app: State<'_, Shared>) -> R<Option<Value>> {
    #[cfg(windows)]
    if let Some(wv) = TabHost::webview(app.inner(), &tab_id) {
        let v = await_cb(|done| crate::webview_win::eval(&wv, MEDIA_STATE_JS.into(), done)).await.flatten();
        return Ok(v.filter(|v| !v.is_null()));
    }
    #[cfg(not(windows))]
    let _ = (tab_id, app);
    Ok(None)
}

#[tauri::command]
pub async fn media_control(tab_id: String, action: String, value: Option<f64>, app: State<'_, Shared>) -> R<()> {
    let Some(wv) = TabHost::webview(app.inner(), &tab_id) else { return Ok(()) };
    let pick = "const m = Array.from(document.querySelectorAll('video, audio')).find((e) => !e.paused) || document.querySelector('video, audio'); if (!m) return;";
    let body = match (action.as_str(), value) {
        ("play", _) => "m.play();".to_string(),
        ("pause", _) => "m.pause();".to_string(),
        ("toggle", _) => "if (m.paused) m.play(); else m.pause();".to_string(),
        ("seek", Some(v)) if v.is_finite() => format!("m.currentTime = {};", v.max(0.0)),
        ("volume", Some(v)) if v.is_finite() => format!("m.muted = false; m.volume = {};", v.clamp(0.0, 1.0)),
        ("mute", _) => {
            #[cfg(windows)]
            crate::webview_win::set_muted(&wv, true);
            return Ok(());
        }
        ("unmute", _) => {
            #[cfg(windows)]
            crate::webview_win::set_muted(&wv, false);
            return Ok(());
        }
        _ => return Err("unknown media action".into()),
    };
    let _ = wv.eval(format!("(() => {{ {pick} {body} }})()"));
    Ok(())
}

// ------------------------------------------------------------------ spaces & profiles

#[tauri::command]
pub async fn select_space(space_id: String, app: State<'_, Shared>) -> R<()> {
    app.mutate(|s| s.activate_space(&space_id));
    Ok(())
}

#[tauri::command]
pub async fn create_space(name: String, color: Option<String>, profile_id: Option<String>, app: State<'_, Shared>) -> R<Option<Space>> {
    Ok(app.mutate(|s| s.create_space(&name, color, profile_id)))
}

#[tauri::command]
pub async fn delete_space(space_id: String, app: State<'_, Shared>) -> R<Value> {
    Ok(match app.mutate(|s| s.delete_space(&space_id)) {
        Ok(()) => ok(),
        Err(e) => err(e),
    })
}

#[tauri::command]
pub async fn update_space(space_id: String, patch: Value, app: State<'_, Shared>) -> R<Value> {
    Ok(match app.mutate(|s| s.update_space(&space_id, &patch)) {
        Some(space) => json!({ "ok": true, "space": space }),
        None => err("not-found"),
    })
}

#[tauri::command]
pub async fn reorder_space(space_id: String, to_index: usize, app: State<'_, Shared>) -> R<Value> {
    Ok(if app.mutate(|s| s.reorder_space(&space_id, to_index)) { ok() } else { err("not-found") })
}

fn profile_json(app: &Shared, p: &Profile) -> Value {
    let mut v = serde_json::to_value(p).unwrap_or_default();
    v["partition"] = json!(app.host.lock().profile_dir(&p.id).to_string_lossy());
    v
}

#[tauri::command]
pub async fn list_profiles(app: State<'_, Shared>) -> R<Value> {
    let app = app.inner();
    let s = app.store.lock();
    let profiles: Vec<Profile> = s.profiles.clone();
    let spaces = s.spaces.clone();
    let (active_profile, active_space) = (s.active_profile_id(), s.active_space_id.clone());
    drop(s);
    Ok(json!({
        "profiles": profiles.iter().map(|p| profile_json(app, p)).collect::<Vec<_>>(),
        "spaces": spaces,
        "activeProfileId": active_profile,
        "activeSpaceId": active_space,
    }))
}

#[tauri::command]
pub async fn create_profile(name: Option<String>, app: State<'_, Shared>) -> R<Value> {
    let (p, space) = app.mutate(|s| s.create_profile(name.as_deref()));
    Ok(json!({ "ok": true, "profile": profile_json(app.inner(), &p), "space": space }))
}

#[tauri::command]
pub async fn rename_profile(profile_id: String, name: String, app: State<'_, Shared>) -> R<Value> {
    Ok(match app.mutate(|s| s.rename_profile(&profile_id, &name)) {
        Some(p) => json!({ "ok": true, "profile": profile_json(app.inner(), &p) }),
        None => err("invalid-name"),
    })
}

#[tauri::command]
pub async fn delete_profile(profile_id: String, app: State<'_, Shared>) -> R<Value> {
    let app = app.inner().clone();
    match app.mutate(|s| s.delete_profile(&profile_id)) {
        Ok(()) => {
            // Remove the profile's cookies/storage once its webviews are gone.
            let dir = app.host.lock().profile_dir(&profile_id);
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                let _ = std::fs::remove_dir_all(dir);
            });
            Ok(ok())
        }
        Err(e) => Ok(err(e)),
    }
}

// ------------------------------------------------------------------ archive

#[tauri::command]
pub async fn list_archive(app: State<'_, Shared>) -> R<Vec<Value>> {
    Ok(app.store.lock().archive_list())
}

#[tauri::command]
pub async fn restore_archived_tab(archived_id: String, app: State<'_, Shared>) -> R<Value> {
    let r = app.mutate(|s| s.restore_archived(&archived_id));
    let list = app.store.lock().archive_list();
    ui_emit(&app.handle, "neutrino:archive-updated", list);
    Ok(match r {
        Ok(tab) => json!({ "ok": true, "tab": tab }),
        Err(e) => err(e),
    })
}

// ------------------------------------------------------------------ settings

#[tauri::command]
pub async fn get_settings(app: State<'_, Shared>) -> R<BrowserSettings> {
    Ok(app.settings())
}

#[tauri::command]
pub async fn update_settings(patch: Value, app: State<'_, Shared>) -> R<BrowserSettings> {
    let updated = {
        let mut s = app.store.lock();
        s.settings.apply_patch(&patch);
        s.settings.clone()
    };
    app.shields.configure(updated.adblock_enabled, &updated.shield_whitelist);
    app.notify(Notice::Settings);
    Ok(updated)
}

#[tauri::command]
pub async fn set_theme_mode(mode: String, window: Window) -> R<Value> {
    let theme = match mode.as_str() {
        "dark" => Some(tauri::Theme::Dark),
        "light" => Some(tauri::Theme::Light),
        _ => None,
    };
    let _ = window.set_theme(theme);
    Ok(json!({ "success": true, "isDark": mode == "dark" }))
}

#[tauri::command]
pub async fn get_performance_status(app: State<'_, Shared>) -> R<PerformanceStatus> {
    let mode = app.settings().reduced_effects_mode;
    // WebView2 has no API for GPU feature status; software rendering is
    // detectable only through the environment override.
    let software = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").map(|v| v.contains("--disable-gpu")).unwrap_or(false);
    let mut gpu = HashMap::new();
    let status = if software { "disabled_software" } else { "enabled" };
    for k in ["gpu_compositing", "rasterization", "webgl"] {
        gpu.insert(k.to_string(), status.to_string());
    }
    Ok(PerformanceStatus {
        hardware_acceleration: !software,
        gpu_disabled_flag: software,
        software_rasterized: software,
        reduced_effects_active: mode == "on" || (mode == "auto" && software),
        reduced_effects_mode: mode,
        gpu_feature_status: gpu,
    })
}

// ------------------------------------------------------------------ shields

#[tauri::command]
pub async fn toggle_shields(app: State<'_, Shared>) -> R<bool> {
    let enabled = {
        let mut s = app.store.lock();
        s.settings.adblock_enabled = !s.settings.adblock_enabled;
        s.settings.adblock_enabled
    };
    let wl = app.settings().shield_whitelist;
    app.shields.configure(enabled, &wl);
    app.notify(Notice::Settings);
    Ok(enabled)
}

#[tauri::command]
pub async fn toggle_site_whitelist(domain_or_url: String, app: State<'_, Shared>) -> R<Value> {
    let host = crate::settings::normalize_host(&domain_or_url);
    if host.is_empty() {
        return Err("invalid domain".into());
    }
    let (whitelisted, list, enabled) = {
        let mut s = app.store.lock();
        let wl = &mut s.settings.shield_whitelist;
        let whitelisted = if let Some(pos) = wl.iter().position(|w| *w == host) {
            wl.remove(pos);
            false
        } else {
            wl.push(host.clone());
            true
        };
        (whitelisted, wl.clone(), s.settings.adblock_enabled)
    };
    app.shields.configure(enabled, &list);
    app.notify(Notice::Settings);
    Ok(json!({ "domain": host, "whitelisted": whitelisted, "shieldWhitelist": list }))
}

#[tauri::command]
pub async fn get_shield_stats(app: State<'_, Shared>) -> R<ShieldStats> {
    Ok(app.shields.stats())
}

#[tauri::command]
pub async fn reset_shield_stats(app: State<'_, Shared>) -> R<ShieldStats> {
    let s = app.shields.reset_stats();
    app.notify(Notice::State);
    Ok(s)
}

#[tauri::command]
pub async fn get_shields_info(app: State<'_, Shared>) -> R<crate::shields::ShieldsInfo> {
    Ok(app.shields.info.lock().clone())
}

#[tauri::command]
pub async fn update_filter_lists(app: State<'_, Shared>) -> R<()> {
    app.shields.spawn_updater(true);
    Ok(())
}

#[tauri::command]
pub async fn zen_optimize(app: State<'_, Shared>) -> R<Value> {
    let slept = app.mutate(|s| s.sleep_inactive(0));
    Ok(json!({ "memorySavedMb": slept as f64 * crate::store::MB_PER_SLEEPING_TAB, "sleepingCount": slept }))
}

// ------------------------------------------------------------------ window

#[tauri::command]
pub async fn window_control(action: String, window: Window, app: State<'_, Shared>) -> R<()> {
    match action.as_str() {
        "minimize" => {
            let _ = window.minimize();
        }
        "maximize" => {
            if window.is_fullscreen().unwrap_or(false) {
                let _ = window.set_fullscreen(false);
            } else if window.is_maximized().unwrap_or(false) {
                let _ = window.unmaximize();
            } else {
                let _ = window.maximize();
            }
        }
        "close" => {
            app.save_now();
            let _ = window.close();
        }
        _ => return Err("unknown action".into()),
    }
    Ok(())
}

// ------------------------------------------------------------------ history

#[tauri::command]
pub async fn get_history(options: Option<HistoryQuery>, app: State<'_, Shared>) -> R<Vec<HistoryEntry>> {
    Ok(app.history.lock().query(&options.unwrap_or_default()))
}

#[tauri::command]
pub async fn history_suggest(text: String, limit: Option<usize>, app: State<'_, Shared>) -> R<Vec<HistoryEntry>> {
    Ok(app.history.lock().suggest(&text, limit.unwrap_or(6).min(20)))
}

#[tauri::command]
pub async fn delete_history_entry(target: Value, app: State<'_, Shared>) -> R<bool> {
    let changed = app.history.lock().delete(target.get("id").and_then(Value::as_str), target.get("url").and_then(Value::as_str));
    if changed {
        app.notify(Notice::History);
    }
    Ok(changed)
}

#[tauri::command]
pub async fn clear_history(range: Option<String>, app: State<'_, Shared>) -> R<usize> {
    let n = app.history.lock().clear(range.as_deref());
    app.notify(Notice::History);
    Ok(n)
}

// ------------------------------------------------------------------ VPN

fn emit_vpn(app: &Shared, status: &crate::vpn::VpnStatus) {
    ui_emit(&app.handle, "neutrino:vpn-status-updated", status.clone());
}

#[tauri::command]
pub async fn get_vpn_status(app: State<'_, Shared>) -> R<crate::vpn::VpnStatus> {
    Ok(app.vpn.status())
}

#[tauri::command]
pub async fn connect_vpn(patch: Option<Value>, app: State<'_, Shared>) -> R<crate::vpn::VpnStatus> {
    let vpn = app.vpn.clone();
    let st = vpn.connect(patch.as_ref()).await;
    emit_vpn(app.inner(), &st);
    Ok(st)
}

#[tauri::command]
pub async fn disconnect_vpn(app: State<'_, Shared>) -> R<crate::vpn::VpnStatus> {
    let st = app.vpn.disconnect();
    emit_vpn(app.inner(), &st);
    Ok(st)
}

#[tauri::command]
pub async fn update_vpn_config(patch: Value, app: State<'_, Shared>) -> R<crate::vpn::VpnStatus> {
    let vpn = app.vpn.clone();
    let st = vpn.update_config(&patch).await;
    emit_vpn(app.inner(), &st);
    Ok(st)
}

#[tauri::command]
pub async fn check_vpn_ip(app: State<'_, Shared>) -> R<Value> {
    let vpn = app.vpn.clone();
    let r = vpn.check_ip().await;
    emit_vpn(app.inner(), &vpn.status());
    Ok(match r {
        Ok(c) => serde_json::to_value(c).unwrap_or_default(),
        Err(e) => json!({ "error": e }),
    })
}

// ------------------------------------------------------------------ MCP & assistant

#[tauri::command]
pub async fn mcp_call(method: String, params: Option<Value>, app: State<'_, Shared>) -> R<Value> {
    Ok(crate::mcp::handle(app.inner(), &method, params.unwrap_or(Value::Null)).await)
}

/// State of the local MCP endpoint (URL, token, running, last error).
#[tauri::command]
pub async fn mcp_server_status(app: State<'_, Shared>) -> R<Value> {
    let (enabled, port) = {
        let s = app.store.lock();
        (s.settings.mcp_server_enabled, s.settings.mcp_server_port)
    };
    Ok(app.mcp_http.status(enabled, port))
}

/// Issues a new access token; clients using the old one stop working.
#[tauri::command]
pub async fn mcp_regenerate_token(app: State<'_, Shared>) -> R<Value> {
    app.mcp_http.regenerate_token();
    mcp_server_status(app).await
}

#[tauri::command]
pub async fn get_mcp_logs(app: State<'_, Shared>) -> R<Vec<Value>> {
    Ok(app.mcp_logs.lock().iter().rev().take(50).rev().cloned().collect())
}

#[tauri::command]
pub async fn assistant_open(url: String, x: f64, y: f64, width: f64, height: f64, app: State<'_, Shared>) -> R<()> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || TabHost::assistant_open(&app, &url, Bounds { x, y, w: width, h: height }))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn assistant_bounds(x: f64, y: f64, width: f64, height: f64, app: State<'_, Shared>) -> R<()> {
    TabHost::assistant_bounds(app.inner(), Bounds { x, y, w: width, h: height });
    Ok(())
}

#[tauri::command]
pub async fn assistant_close(app: State<'_, Shared>) -> R<()> {
    TabHost::assistant_close(app.inner());
    Ok(())
}

/// Types text into the assistant page's prompt box.
#[tauri::command]
pub async fn assistant_insert_text(text: String, app: State<'_, Shared>) -> R<bool> {
    let Some(wv) = TabHost::assistant_webview(app.inner()) else { return Ok(false) };
    let js = format!(
        "(() => {{ const el = document.querySelector('textarea, div[contenteditable=\"true\"]'); if (!el) return; el.focus(); document.execCommand('insertText', false, {}); }})()",
        serde_json::to_string(&text).unwrap_or_default()
    );
    let _ = wv.eval(js);
    Ok(true)
}

// ------------------------------------------------------------------ passwords

/// The password vault is not part of this build; say so instead of faking it.
#[tauri::command]
pub async fn passwords_status() -> R<Value> {
    Ok(json!({ "ok": false, "error": "unavailable", "message": "The password manager is not available in this build yet." }))
}

#[tauri::command]
pub async fn passwords_list() -> R<Vec<Value>> {
    Ok(Vec::new())
}

// ------------------------------------------------------------------ extensions

#[tauri::command]
pub async fn list_extensions(app: State<'_, Shared>) -> R<Vec<crate::extensions::ExtensionRecord>> {
    Ok(app.extensions.list())
}

/// Installs from a Chrome Web Store URL or a bare extension id.
#[tauri::command]
pub async fn install_extension(input: String, app: State<'_, Shared>) -> R<crate::extensions::ExtensionRecord> {
    let id = crate::extensions::parse_extension_id(&input).ok_or("Enter a Chrome Web Store link or a 32-letter extension ID")?;
    let record = app.extensions.install(&id).await?;
    crate::extensions::refresh(&app);
    Ok(record)
}

#[tauri::command]
pub async fn set_extension_enabled(id: String, enabled: bool, app: State<'_, Shared>) -> R<bool> {
    let changed = app.extensions.set_enabled(&id, enabled);
    if changed {
        crate::extensions::refresh(&app);
    }
    Ok(changed)
}

#[tauri::command]
pub async fn remove_extension(id: String, app: State<'_, Shared>) -> R<bool> {
    let removed = app.extensions.remove(&id);
    if removed {
        crate::extensions::refresh(&app);
        // Give live profiles a moment to unload it before deleting its files.
        let path = app.extensions.path_of(&id);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let _ = std::fs::remove_dir_all(path);
        });
    }
    Ok(removed)
}

/// Opens an extension's popup or options page in a new tab.
#[tauri::command]
pub async fn open_extension_page(id: String, page: String, app: State<'_, Shared>) -> R<bool> {
    let Some(url) = app.extensions.get(&id).and_then(|r| r.page_url(&page)) else { return Ok(false) };
    app.mutate(|s| s.create_tab(Some(url), None, true, None));
    Ok(true)
}

/// Install requested by the "Add to Neutrino" button on a Chrome Web Store page.
pub fn spawn_store_install(app: &Shared, tab_id: &str, ext_id: &str) {
    let (app, tab_id, ext_id) = (app.clone(), tab_id.to_string(), ext_id.to_string());
    tauri::async_runtime::spawn(async move {
        let result = app.extensions.install(&ext_id).await;
        let js = match &result {
            Ok(_) => crate::extensions::store_state_js(&ext_id, "installed", None),
            Err(e) if e == "already installing" => return,
            Err(e) => crate::extensions::store_state_js(&ext_id, "error", Some(e)),
        };
        if let Ok(rec) = &result {
            crate::extensions::refresh(&app);
            ui_emit(&app.handle, "neutrino:extension-installed", rec.clone());
        }
        if let Some(wv) = TabHost::webview(&app, &tab_id) {
            let _ = wv.eval(js);
        }
    });
}

// ------------------------------------------------------------------ downloads

#[tauri::command]
pub async fn list_downloads(app: State<'_, Shared>) -> R<Vec<crate::downloads::DownloadItem>> {
    Ok(app.downloads.list())
}

/// `pause` | `resume` | `cancel` | `open` | `show` | `remove` | `retry`.
#[tauri::command]
pub async fn download_action(id: String, action: String, app: State<'_, Shared>) -> R<bool> {
    let item = app.downloads.get(&id).ok_or("Download not found")?;
    match action.as_str() {
        "pause" | "resume" | "cancel" => {
            #[cfg(windows)]
            {
                let (tx, rx) = tokio::sync::oneshot::channel();
                let (id2, action2) = (id.clone(), action.clone());
                app.handle
                    .run_on_main_thread(move || {
                        let _ = tx.send(crate::downloads::native::control(&id2, &action2));
                    })
                    .map_err(|e| e.to_string())?;
                let done = rx.await.unwrap_or(false);
                if !done && action == "cancel" {
                    // No live operation (e.g. it already failed): just mark it canceled.
                    if let Some(it) = app.downloads.set_state(&id, |i| {
                        if i.is_active() || i.can_resume {
                            i.state = crate::downloads::DownloadState::Canceled;
                            i.can_resume = false;
                        }
                    }) {
                        crate::downloads::emit(&app, &it);
                    }
                    return Ok(true);
                }
                Ok(done)
            }
            #[cfg(not(windows))]
            Ok(false)
        }
        "open" => {
            if !std::path::Path::new(&item.path).exists() {
                return Err("The file was moved or deleted".into());
            }
            open::that_detached(&item.path).map_err(|e| e.to_string())?;
            Ok(true)
        }
        "show" => crate::downloads::reveal(std::path::Path::new(&item.path)).map(|_| true),
        "remove" => {
            let removed = app.downloads.remove(&id);
            if removed {
                ui_emit(&app.handle, "neutrino:downloads-changed", app.downloads.list());
            }
            Ok(removed)
        }
        "retry" => {
            // WebView2 has no "download this URL" API: navigating a page to a
            // download URL starts the download without leaving the page.
            let live = {
                let s = app.store.lock();
                s.active_tab().filter(|t| t.wants_webview()).map(|t| t.id.clone())
            };
            match live.and_then(|tid| TabHost::webview(&app, &tid)) {
                Some(wv) => {
                    let url = url::Url::parse(&item.url).map_err(|e| e.to_string())?;
                    wv.navigate(url).map_err(|e| e.to_string())?;
                }
                None => {
                    let url = item.url.clone();
                    app.mutate(|s| s.create_tab(Some(url), None, true, None));
                }
            }
            app.downloads.remove(&id);
            ui_emit(&app.handle, "neutrino:downloads-changed", app.downloads.list());
            Ok(true)
        }
        _ => Err(format!("unknown action {action}")),
    }
}

#[tauri::command]
pub async fn clear_downloads(app: State<'_, Shared>) -> R<Vec<crate::downloads::DownloadItem>> {
    app.downloads.clear_finished();
    let list = app.downloads.list();
    ui_emit(&app.handle, "neutrino:downloads-changed", list.clone());
    Ok(list)
}

#[tauri::command]
pub async fn open_downloads_folder() -> R<()> {
    open::that_detached(crate::downloads::default_folder()).map_err(|e| e.to_string())
}
