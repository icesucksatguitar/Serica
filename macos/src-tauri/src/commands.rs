//! IPC commands exposed to the UI (`window.serica` in `src/adapter/tauriBridge.ts`).

use crate::app::{normalize_input, ui_emit, Notice, Shared};
use crate::history::{HistoryEntry, HistoryQuery};
use crate::model::*;
use crate::settings::{BrowserSettings, PerformanceStatus};
use crate::store::MoveOp;
use crate::tabhost::{Bounds, TabHost};
use serde_json::{json, Value};
use std::collections::HashMap;
use tauri::{Manager, State, Window};

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

/// A private tab (InPrivate: nothing kept once it closes), made active.
#[tauri::command]
pub async fn create_private_tab(url: Option<String>, app: State<'_, Shared>) -> R<TabState> {
    let prefix = app.settings().search_prefix();
    let url = url.and_then(|u| normalize_input(&u, prefix));
    Ok(app.mutate(|s| s.create_private_tab(url)))
}

#[tauri::command]
pub async fn close_tab(tab_id: String, app: State<'_, Shared>) -> R<()> {
    app.mutate(|s| s.close_tab(&tab_id));
    Ok(())
}

#[tauri::command]
pub async fn select_tab(tab_id: String, app: State<'_, Shared>) -> R<()> {
    // A tab in a window of its own: that window comes to the front.
    if crate::detached::focus(&tab_id) {
        return Ok(());
    }
    app.mutate(|s| s.activate_tab(&tab_id));
    Ok(())
}

/// Split view: shows `dragged` beside `target` (`left`: on its left).
#[tauri::command]
pub async fn split_tabs(dragged: String, target: String, left: bool, app: State<'_, Shared>) -> R<bool> {
    Ok(app.mutate(|s| s.split_with(&dragged, &target, left)))
}

/// Moves a tab into a window of its own, near `x`, `y` (screen, physical px).
#[tauri::command]
pub async fn detach_tab(tab_id: String, x: Option<f64>, y: Option<f64>, app: State<'_, Shared>) -> R<()> {
    let app = app.inner().clone();
    let at = x.zip(y);
    tauri::async_runtime::spawn_blocking(move || crate::detached::detach(&app, &tab_id, at)).await.map_err(|e| e.to_string())?
}

/// Brings a tab from its own window back into the main window.
#[tauri::command]
pub async fn reattach_tab(tab_id: String, app: State<'_, Shared>) -> R<()> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || crate::detached::reattach(&app, &tab_id)).await.map_err(|e| e.to_string())?
}

/// Back, forward, reload or stop on one tab's page (a tab window's toolbar).
#[tauri::command]
pub async fn tab_action(tab_id: String, action: String, app: State<'_, Shared>) -> R<()> {
    let Some(wv) = TabHost::webview(app.inner(), &tab_id) else { return Ok(()) };
    let a: &'static str = match action.as_str() {
        "back" => "back",
        "forward" => "forward",
        "reload" => "reload",
        "hard-reload" => "hard-reload",
        "stop" => "stop",
        _ => return Ok(()),
    };
    crate::webview_mac::nav(&wv, a);
    Ok(())
}

/// Ends the split `tab_id` is in; the tab stays the active one.
#[tauri::command]
pub async fn unsplit_tab(tab_id: String, app: State<'_, Shared>) -> R<bool> {
    Ok(app.mutate(|s| s.unsplit(&tab_id)))
}

/// The left pane's share of the width (0.2-0.8). While the divider is being
/// dragged (`commit` false) the panes only resize; the release saves it and
/// tells the UI.
#[tauri::command]
pub async fn set_split_ratio(tab_id: String, ratio: f64, commit: Option<bool>, app: State<'_, Shared>) -> R<bool> {
    let changed = if commit.unwrap_or(true) {
        app.mutate(|s| s.set_split_ratio(&tab_id, ratio))
    } else {
        app.store.lock().set_split_ratio(&tab_id, ratio)
    };
    if changed {
        // Resize at once, not after the next state broadcast.
        let app = app.inner().clone();
        let _ = tauri::async_runtime::spawn_blocking(move || TabHost::reconcile(&app)).await;
    }
    Ok(changed)
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

#[derive(serde::Deserialize)]
pub struct PinnedSiteInput {
    url: String,
    title: String,
}

/// Pins sites in a space without loading them (chosen during onboarding).
#[tauri::command]
pub async fn add_pinned_sites(sites: Vec<PinnedSiteInput>, space_id: Option<String>, app: State<'_, Shared>) -> R<usize> {
    let sites: Vec<(String, String, Option<String>)> = sites
        .into_iter()
        .take(24)
        .filter_map(|s| {
            let parsed = url::Url::parse(&s.url).ok().filter(|u| matches!(u.scheme(), "http" | "https"))?;
            let favicon = parsed.host_str().map(|h| format!("https://www.google.com/s2/favicons?domain={h}&sz=64"));
            let title: String = s.title.trim().chars().take(80).collect();
            Some((parsed.to_string(), if title.is_empty() { parsed.host_str().unwrap_or_default().to_string() } else { title }, favicon))
        })
        .collect();
    Ok(app.mutate(|s| sites.into_iter().filter_map(|(u, t, f)| s.add_pinned_site(u, t, f, space_id.clone())).count()))
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
            s.apply_move(&MoveOp::Reorder { section: section.into(), id: dragged_tab_id.clone(), to_index: to, folder_id: None });
        } else {
            let from = if d.is_pinned { "pinned" } else { "today" };
            s.apply_move(&MoveOp::Section { id: dragged_tab_id.clone(), from: from.into(), to: section.into(), space_id: t.space_id.clone(), to_index: Some(to), folder_id: None });
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
    let a: &'static str = match action.as_str() {
        "back" => "back",
        "forward" => "forward",
        "reload" => "reload",
        "hard-reload" => "hard-reload",
        "stop" => "stop",
        _ => return Ok(()),
    };
    crate::webview_mac::nav(&wv, a);
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

/// Answers a page dialog (alert/confirm/prompt/"Leave site?"/"Open in app?") shown by the UI.
#[tauri::command]
pub async fn answer_page_dialog(id: u64, accept: bool, text: Option<String>, app: State<'_, Shared>) -> R<()> {
    let _ = app.handle.run_on_main_thread(move || crate::webview_mac::answer_dialog(id, accept, text));
    Ok(())
}

/// Answers a site's permission request shown by the UI (`permission-request`
/// event): "allow" / "block" are remembered for the site, "allow-once" and
/// "deny" apply this time only.
#[tauri::command]
pub async fn answer_permission(id: u64, decision: String, app: State<'_, Shared>) -> R<()> {
    if !matches!(decision.as_str(), "allow" | "allow-once" | "block" | "deny") {
        return Err("bad decision".into());
    }
    let a = app.inner().clone();
    let _ = app.handle.run_on_main_thread(move || crate::webview_mac::answer_permission(&a, id, &decision));
    Ok(())
}

/// Opens files dropped on Serica's own interface (not on a page) in new tabs.
#[tauri::command]
pub async fn open_dropped_paths(paths: Vec<String>, app: State<'_, Shared>) -> R<usize> {
    let urls: Vec<String> = paths.iter().take(20).filter_map(|p| crate::files::file_url(std::path::Path::new(p))).collect();
    let n = urls.len();
    let app = app.inner();
    for (i, url) in urls.into_iter().enumerate() {
        app.mutate(|s| s.create_tab(Some(url), None, i == 0, None));
    }
    Ok(n)
}

// ------------------------------------------------------------------ viewport

#[tauri::command]
pub async fn set_viewport(x: f64, y: f64, width: f64, height: f64, app: State<'_, Shared>) -> R<()> {
    if [x, y, width, height].iter().all(|v| v.is_finite()) {
        // The UI reports CSS px; the interface is zoomed by the text size.
        let z = app.settings().text_scale;
        TabHost::set_bounds(app.inner(), Bounds { x: x * z, y: y * z, w: (width * z).max(0.0), h: (height * z).max(0.0) });
    }
    Ok(())
}

/// Lets a piece of UI show through the page (see `TabHost::set_hole`).
/// `rect`/`from` are `[x, y, w, h]` in CSS px; `ease` a CSS cubic-bezier.
#[tauri::command]
pub async fn set_viewport_hole(
    id: String,
    rect: [f64; 4],
    from: Option<[f64; 4]>,
    ms: Option<u32>,
    ease: Option<[f64; 4]>,
    remove: Option<bool>,
    lead: Option<f64>,
    radius: Option<f64>,
    app: State<'_, Shared>,
) -> R<()> {
    let rad = radius.filter(|r| r.is_finite()).unwrap_or(0.0).clamp(0.0, 200.0);
    let h = |r: [f64; 4]| crate::tabhost::Hole { x: r[0], y: r[1], w: r[2], h: r[3].max(0.0), r: rad };
    if !rect.iter().all(|v| v.is_finite()) || id.len() > 64 {
        return Err("bad hole".into());
    }
    let seq = TabHost::set_hole(
        app.inner(),
        id,
        h(rect),
        from.filter(|r| r.iter().all(|v| v.is_finite())).map(h),
        ms.unwrap_or(0).min(2000),
        ease.unwrap_or([0.25, 0.1, 0.25, 1.0]),
        remove.unwrap_or(false),
        lead.filter(|l| l.is_finite()).unwrap_or(0.0).clamp(-100.0, 100.0),
    );
    // Resolve once the page really is cut (the clip worker applied it), so
    // "ready" means UI under the hole is visible.
    let app = app.inner().clone();
    let _ = tauri::async_runtime::spawn_blocking(move || TabHost::wait_clip(&app, seq, std::time::Duration::from_millis(500))).await;
    Ok(())
}

/// The pick from Serica's page context menu (`page-context-menu` event):
/// an engine command id, one of Serica's own entries, or neither (dismissed).
#[tauri::command]
pub fn page_menu_select(id: u64, command: Option<i32>, custom: Option<usize>, refocus: Option<bool>, app: State<'_, Shared>) {
    let app = app.inner().clone();
    let handle = app.handle.clone();
    let refocus = refocus.unwrap_or(false);
    let _ = handle.run_on_main_thread(move || crate::webview_mac::finish_page_menu(&app, id, command, custom, refocus));
}

/// Where the cursor is over the app window: `[x, y, clientWidth]` in points
/// of the window's content, or `None` when it is outside. The UI cannot see
/// the pointer over a page (a separate native view), so floating UI asks
/// this to know when the pointer has left it. Reads the system cursor
/// directly: never waits on the main thread.
#[tauri::command]
pub fn ui_cursor(app: State<'_, Shared>) -> Option<[f64; 3]> {
    let _ = app;
    crate::edge_watch_mac::cursor_in_window().map(|(x, y, w)| [x, y, w])
}

/// Captures a page while it is on screen (for the UI to show in its place),
/// without hiding it: the active tab's, or `tab_id`'s (a pane of a split).
#[tauri::command]
pub async fn viewport_capture(tab_id: Option<String>, app: State<'_, Shared>) -> R<Option<String>> {
    let app = app.inner().clone();
    // The tab being captured (read before: a switch may happen meanwhile).
    let active_id = tab_id.or_else(|| app.store.lock().active_tab_id.clone());
    let mut image = None;
    if let Some(id) = &active_id {
        let wv = {
            let host = app.host.lock();
            host.views.get(id).filter(|v| host.visible && v.shown).map(|v| v.webview.clone())
        };
        if let Some(wv) = wv {
            image = await_cb(|done| crate::webview_mac::capture(&wv, done)).await.flatten();
        }
    }
    Ok(match (image, active_id) {
        (Some(img), Some(id)) => Some(app.previews.put(&id, img)),
        _ => None,
    })
}

/// Hides the native webview so UI overlays can cover the page. With `capture`
/// (the default) it first captures the page and returns the image; the UI
/// passes `false` after painting a capture from `viewport_capture`.
#[tauri::command]
pub async fn viewport_freeze(capture: Option<bool>, app: State<'_, Shared>) -> R<Option<String>> {
    let app = app.inner().clone();
    let active_id = app.store.lock().active_tab_id.clone();
    let mut image = None;
    if capture.unwrap_or(true) {
        if let Some(wv) = TabHost::active_webview(&app) {
            if app.host.lock().visible {
                image = await_cb(|done| crate::webview_mac::capture(&wv, done)).await.flatten();
            }
        }
    }
    let a2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || TabHost::set_visible(&a2, false)).await.map_err(|e| e.to_string())?;
    Ok(match (image, active_id) {
        (Some(img), Some(id)) => Some(app.previews.put(&id, img)),
        _ => None,
    })
}

#[tauri::command]
pub async fn viewport_thaw(app: State<'_, Shared>) -> R<()> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || TabHost::set_visible(&app, true)).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn find_in_page(query: Option<String>, forward: Option<bool>, find_next: Option<bool>, app: State<'_, Shared>) -> R<()> {
    let id = app.store.lock().active_tab_id.clone().unwrap_or_default();
    if let Some(wv) = TabHost::active_webview(app.inner()) {
        let q = query.filter(|q| !q.is_empty());
        crate::webview_mac::find(app.inner(), &wv, &id, q, forward.unwrap_or(true), find_next.unwrap_or(false));
    }
    Ok(())
}

#[tauri::command]
pub async fn set_zoom_factor(factor: f64, tab_id: Option<String>, app: State<'_, Shared>) -> R<()> {
    let factor = if factor.is_finite() { factor.clamp(0.25, 5.0) } else { 1.0 };
    let app = app.inner().clone();
    let id = tab_id.or_else(|| app.store.lock().active_tab_id.clone());
    if let Some(id) = id {
        let remembered = app.mutate(|s| {
            let url = s.tab_mut(&id).map(|t| {
                t.zoom_factor = Some(factor);
                t.url.clone()
            });
            // Zoom is per site, as in other browsers.
            url.is_some_and(|u| s.settings.remember_zoom(&u, factor))
        });
        if remembered {
            app.notify(Notice::Settings);
        }
        if let Some(wv) = TabHost::webview(&app, &id) {
            crate::webview_mac::set_zoom(&wv, factor);
        }
        // WebKit reports no zoom change of its own: tell the UI.
        ui_emit(&app.handle, "serica:zoom-updated", json!({ "tabId": id, "zoomFactor": factor }));
    }
    Ok(())
}

#[tauri::command]
pub async fn capture_tab_preview(tab_id: String, app: State<'_, Shared>) -> R<Option<String>> {
    let app = app.inner().clone();
    let visible = app.host.lock().views.get(&tab_id).map(|v| v.shown).unwrap_or(false);
    if visible {
        if let Some(wv) = TabHost::webview(&app, &tab_id) {
            if let Some(Some(img)) = await_cb(|done| crate::webview_mac::capture(&wv, done)).await {
                return Ok(Some(app.previews.put(&tab_id, img)));
            }
        }
    }
    Ok(app.previews.url(&tab_id))
}

#[tauri::command]
pub async fn get_tab_preview(tab_id: String, app: State<'_, Shared>) -> R<Option<String>> {
    Ok(app.previews.url(&tab_id))
}

#[tauri::command]
pub async fn open_devtools(app: State<'_, Shared>) -> R<()> {
    if let Some(wv) = TabHost::active_webview(app.inner()) {
        wv.open_devtools();
    }
    Ok(())
}

// ------------------------------------------------------------------ media

#[tauri::command]
pub async fn get_media_state(tab_id: String, app: State<'_, Shared>) -> R<Option<Value>> {
    if let Some(wv) = TabHost::webview(app.inner(), &tab_id) {
        let js = crate::tabhost::media_state_js();
        let v = await_cb(|done| crate::webview_mac::media_eval(&wv, tab_id, js, true, done)).await.flatten();
        return Ok(v.filter(|v| v.is_object()));
    }
    Ok(None)
}

/// Mini player controls. They act on the playing media element wherever it
/// is: in the page, in a web component, or in a player embedded from another
/// site (see `webview_mac::media_eval`).
#[tauri::command]
pub async fn media_control(tab_id: String, action: String, value: Option<f64>, app: State<'_, Shared>) -> R<()> {
    let Some(wv) = TabHost::webview(app.inner(), &tab_id) else { return Ok(()) };
    let js = match (action.as_str(), value) {
        ("play", _) => crate::tabhost::media_action_js("m.play();"),
        ("pause", _) => crate::tabhost::media_action_js("m.pause();"),
        ("toggle", _) => crate::tabhost::media_action_js("if (m.paused) m.play(); else m.pause();"),
        ("pip", _) => crate::tabhost::pip_enter_js(),
        ("exit-pip", _) => crate::tabhost::PIP_EXIT_JS.to_string(),
        ("seek", Some(v)) if v.is_finite() => crate::tabhost::media_action_js(&format!("m.currentTime = {};", v.max(0.0))),
        ("volume", Some(v)) if v.is_finite() => crate::tabhost::media_action_js(&format!("m.muted = false; m.volume = {};", v.clamp(0.0, 1.0))),
        ("mute", _) => {
            crate::webview_mac::set_muted(&wv, true);
            return Ok(());
        }
        ("unmute", _) => {
            crate::webview_mac::set_muted(&wv, false);
            return Ok(());
        }
        _ => return Err("unknown media action".into()),
    };
    crate::webview_mac::media_eval(&wv, tab_id, js, true, |_| {});
    Ok(())
}

// ------------------------------------------------------------------ spaces & profiles

/// `keep_ui_focus`: the switch was made from the open sidebar, so the page
/// that comes on screen leaves focus in the UI (see `TabHost::keep_ui_focus`).
#[tauri::command]
pub async fn select_space(space_id: String, keep_ui_focus: Option<bool>, app: State<'_, Shared>) -> R<()> {
    if keep_ui_focus == Some(true) {
        app.host.lock().keep_ui_focus = true;
    }
    app.mutate(|s| s.activate_space(&space_id));
    Ok(())
}

/// Ends a `select_space(.., keep_ui_focus)` session: pages take focus as usual
/// again, and the page on screen gets it now unless `focus` is false (the
/// UI has moved on to something else that has focus, like the address bar).
#[tauri::command]
pub fn focus_page(focus: Option<bool>, app: State<'_, Shared>) {
    TabHost::focus_page(app.inner(), focus.unwrap_or(true));
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
            let handle = app.handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                crate::webview_mac::remove_profile_data(&handle, &profile_id);
                if let Some(app) = crate::app::global() {
                    app.passwords.remove_profile(&profile_id);
                }
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
    ui_emit(&app.handle, "serica:archive-updated", list);
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
    let (updated, old_scale) = {
        let mut s = app.store.lock();
        let old = s.settings.text_scale;
        s.settings.apply_patch(&patch);
        (s.settings.clone(), old)
    };
    if (updated.text_scale - old_scale).abs() > 0.001 {
        apply_text_scale(app.inner(), old_scale, updated.text_scale);
    }
    app.shields.configure(updated.adblock_enabled, &updated.shield_whitelist);
    app.notify(Notice::Settings);
    Ok(updated)
}

/// Zooms the interface to the text size.
pub fn apply_ui_zoom(app: &Shared, scale: f64) {
    if let Some(main) = app.handle.get_webview("main") {
        let _ = main.set_zoom(scale);
    }
}

/// A new text size: zooms the interface, and moves pages still at the old
/// default size to the new one (pages the user zoomed by hand keep their zoom).
fn apply_text_scale(app: &Shared, old: f64, new: f64) {
    apply_ui_zoom(app, new);
    let moved: Vec<String> = {
        let mut s = app.store.lock();
        s.tabs
            .iter_mut()
            .filter(|t| t.zoom_factor.map(|z| (z - old).abs() < 0.001).unwrap_or(true))
            .map(|t| {
                t.zoom_factor = Some(new);
                t.id.clone()
            })
            .collect()
    };
    for id in moved {
        if let Some(wv) = TabHost::webview(app, &id) {
            crate::webview_mac::set_zoom(&wv, new);
        }
    }
    app.notify(Notice::State);
}

#[tauri::command]
pub async fn set_theme_mode(mode: String, window: Window, app: State<'_, Shared>) -> R<Value> {
    let theme = match mode.as_str() {
        "dark" => Some(tauri::Theme::Dark),
        "light" => Some(tauri::Theme::Light),
        _ => None,
    };
    let _ = window.set_theme(theme);
    let dark = match theme {
        Some(t) => t == tauri::Theme::Dark,
        None => window.theme().map(|t| t == tauri::Theme::Dark).unwrap_or(true),
    };
    app.set_ui_dark(dark);
    Ok(json!({ "success": true, "isDark": mode == "dark" }))
}

// ------------------------------------------------------------------ new-tab widgets

#[tauri::command]
pub async fn widget_github(user: String) -> Result<crate::widgets::Contributions, String> {
    crate::widgets::github_contributions(&user).await
}

#[tauri::command]
pub async fn widget_geocode(query: String) -> Result<Value, String> {
    crate::widgets::geocode(&query).await
}

#[tauri::command]
pub async fn widget_weather(lat: f64, lon: f64, fahrenheit: bool) -> Result<Value, String> {
    crate::widgets::weather(lat, lon, fahrenheit).await
}

/// Dark-PDF state of a tab (top-bar toggle on PDF tabs).
#[tauri::command]
pub fn get_pdf_dark_tab(tab_id: String, app: State<'_, Shared>) -> Value {
    json!({ "available": app.pdf_dark_global(), "dark": app.pdf_dark_for(&tab_id) })
}

#[tauri::command]
pub fn set_pdf_dark_tab(tab_id: String, dark: bool, app: State<'_, Shared>) {
    app.set_pdf_dark_tab(&tab_id, dark);
}

#[tauri::command]
pub async fn get_performance_status(app: State<'_, Shared>) -> R<PerformanceStatus> {
    let mode = app.settings().reduced_effects_mode;
    // WebKit always composites on the GPU (Metal) on supported Macs; there is
    // no software path to detect.
    let software = false;
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
    crate::content_rules::set_enabled(&app.handle, enabled);
    app.notify(Notice::Settings);
    // WebKit applies rule lists and cosmetic filters from a page's start:
    // the page on screen reloads so the change shows at once.
    reload_shielded(app.inner(), None);
    Ok(enabled)
}

/// Reloads open pages whose Shields state just changed (all of them, or
/// those on `host` and its subdomains).
fn reload_shielded(app: &Shared, host: Option<&str>) {
    let views: Vec<(String, tauri::Webview)> = app.host.lock().views.iter().map(|(id, v)| (id.clone(), v.webview.clone())).collect();
    let active = app.store.lock().active_tab_id.clone();
    for (id, wv) in views {
        let url = app.store.lock().tab(&id).map(|t| t.url.clone()).unwrap_or_default();
        let on_host = match host {
            Some(h) => {
                let u = crate::settings::normalize_host(&url);
                u == h || u.ends_with(&format!(".{h}"))
            }
            None => Some(&id) == active.as_ref(),
        };
        if on_host && url.starts_with("http") {
            crate::webview_mac::nav(&wv, "reload");
        }
    }
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
    reload_shielded(app.inner(), Some(&host));
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

/// The window's state the UI lays out around: full screen, and where the
/// traffic lights end (points from the window's left edge) and their middle
/// (from its top), so the top bar's controls start clear of them.
#[tauri::command]
pub fn get_window_state(window: Window) -> Value {
    let fullscreen = window.is_fullscreen().unwrap_or(false);
    let mut lights = Value::Null;
    if let Ok(ptr) = window.ns_window() {
        if !ptr.is_null() {
            // SAFETY: the live NSWindow of `window`; sync commands run on the main thread.
            let ns_window = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
            let size = ns_window.frame().size;
            let height = size.height;
            if let Some(zoom) = ns_window.standardWindowButton(objc2_app_kit::NSWindowButton::ZoomButton) {
                let r = zoom.convertRect_toView(zoom.bounds(), None);
                lights = json!({ "right": r.origin.x + r.size.width, "centerY": height - (r.origin.y + r.size.height / 2.0), "windowWidth": size.width });
            }
        }
    }
    json!({ "fullscreen": fullscreen, "platform": "macos", "trafficLights": lights })
}

/// Shows or hides the traffic lights: hidden while an auto-hidden top bar
/// (and the sidebar) are out of sight, so they never sit on a page. In full
/// screen macOS shows them with the menu bar.
#[tauri::command]
pub fn set_traffic_lights(visible: bool, window: Window) {
    let fullscreen = window.is_fullscreen().unwrap_or(false);
    let Ok(ptr) = window.ns_window() else { return };
    if ptr.is_null() {
        return;
    }
    // SAFETY: the live NSWindow of `window`; sync commands run on the main thread.
    let ns_window = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
    for b in [objc2_app_kit::NSWindowButton::CloseButton, objc2_app_kit::NSWindowButton::MiniaturizeButton, objc2_app_kit::NSWindowButton::ZoomButton] {
        if let Some(button) = ns_window.standardWindowButton(b) {
            button.setHidden(!(visible || fullscreen));
        }
    }
}

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
        // A press in a drag region of the top bar or the sidebar's header.
        "drag" => {
            let _ = window.start_dragging();
        }
        // A double-click there: what the user chose in System Settings ›
        // Desktop & Dock ("Double-click a window's title bar to"), as for any
        // Mac window.
        "titlebar-double-click" => crate::window_state::title_bar_double_click(&window),
        "fullscreen" => {
            let fs = window.is_fullscreen().unwrap_or(false);
            let _ = window.set_fullscreen(!fs);
        }
        "close" => {
            app.save_now();
            clear_cookies_for_exit(app.inner()).await;
            let _ = window.close();
        }
        _ => return Err("unknown action".into()),
    }
    Ok(())
}

// ------------------------------------------------------------------ cookies

/// The active space's profile.
fn active_profile(app: &Shared) -> String {
    let s = app.store.lock();
    s.space_profile(&s.active_space_id)
}

/// Sites with cookies in the current profile.
#[tauri::command]
pub async fn cookie_sites(app: State<'_, Shared>) -> R<Value> {
    let profile = active_profile(app.inner());
    let (tx, rx) = tokio::sync::oneshot::channel();
    crate::webview_mac::cookie_sites(&app.handle, &profile, move |v| {
        let _ = tx.send(v);
    });
    let sites = tokio::time::timeout(std::time::Duration::from_secs(5), rx).await.ok().and_then(Result::ok).unwrap_or_default();
    let list: Vec<Value> = sites.into_iter().map(|(site, count)| json!({ "site": site, "count": count })).collect();
    Ok(json!({ "ok": true, "sites": list }))
}

/// Deletes the current profile's cookies for `site` (and its subdomains), or
/// all of them without a site.
#[tauri::command]
pub async fn delete_cookies(site: Option<String>, app: State<'_, Shared>) -> R<Value> {
    let profile = active_profile(app.inner());
    let sites: Option<Vec<String>> = site.map(|s| vec![s]);
    let (tx, rx) = tokio::sync::oneshot::channel();
    crate::webview_mac::delete_cookies(
        &app.handle,
        &profile,
        move |domain| sites.as_ref().map(|s| crate::webview_mac::cookie_in_sites(domain, s)).unwrap_or(true),
        move |n| {
            let _ = tx.send(n);
        },
    );
    let n = tokio::time::timeout(std::time::Duration::from_secs(5), rx).await.ok().and_then(Result::ok).unwrap_or(0);
    Ok(json!({ "ok": true, "deleted": n }))
}

/// "Clear cookies when Serica closes": deletes every profile's cookies
/// except the kept sites. Bounded, so closing never hangs on it; whatever is
/// missed is cleared before the next session's first page loads.
pub async fn clear_cookies_for_exit(app: &Shared) {
    let s = app.settings();
    if !s.clear_cookies_on_exit {
        return;
    }
    let profiles: Vec<String> = app.store.lock().profiles.iter().map(|p| p.id.clone()).collect();
    let mut waits = Vec::new();
    for profile in profiles {
        let keep = s.cookie_keep_sites.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        crate::webview_mac::delete_cookies(&app.handle, &profile, move |d| !crate::webview_mac::cookie_in_sites(d, &keep), move |n| {
            let _ = tx.send(n);
        });
        waits.push(rx);
    }
    let _ = tokio::time::timeout(std::time::Duration::from_millis(1500), futures_join(waits)).await;
}

async fn futures_join(waits: Vec<tokio::sync::oneshot::Receiver<usize>>) {
    for w in waits {
        let _ = w.await;
    }
}

// ------------------------------------------------------------------ history

#[tauri::command]
pub async fn get_history(options: Option<HistoryQuery>, app: State<'_, Shared>) -> R<Vec<HistoryEntry>> {
    Ok(app.history.lock().query(&options.unwrap_or_default()))
}

#[tauri::command]
pub async fn history_suggest(text: String, limit: Option<usize>, app: State<'_, Shared>) -> R<Vec<HistoryEntry>> {
    let limit = limit.unwrap_or(6).min(20);
    let mut out = app.history.lock().suggest(&text, limit);
    // Bookmarks brought over from other browsers fill in after history.
    if out.len() < limit {
        let have: std::collections::HashSet<String> = out.iter().map(|e| e.url.clone()).collect();
        let extra: Vec<HistoryEntry> = app.bookmarks.lock().suggest(&text, limit).into_iter().filter(|b| !have.contains(&b.url)).collect();
        out.extend(extra.into_iter().take(limit - out.len()));
    }
    Ok(out)
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
    ui_emit(&app.handle, "serica:vpn-status-updated", status.clone());
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || TabHost::sync_network(&app));
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
    let z = app.settings().text_scale;
    let b = Bounds { x: x * z, y: y * z, w: width * z, h: height * z };
    tauri::async_runtime::spawn_blocking(move || TabHost::assistant_open(&app, &url, b))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn assistant_bounds(x: f64, y: f64, width: f64, height: f64, app: State<'_, Shared>) -> R<()> {
    let z = app.settings().text_scale;
    TabHost::assistant_bounds(app.inner(), Bounds { x: x * z, y: y * z, w: width * z, h: height * z });
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

/// Opens an extension's popup or options page in a new tab (WebKit's
/// extension runtime serves them; see `extensions::webkit`).
#[tauri::command]
pub async fn open_extension_page(id: String, page: String, app: State<'_, Shared>) -> R<bool> {
    let Some(rec) = app.extensions.get(&id) else { return Ok(false) };
    let Some(url) = crate::extensions::webkit::page_url(app.inner(), &rec, &page).await else { return Ok(false) };
    app.mutate(|s| s.create_tab(Some(url), None, true, None));
    Ok(true)
}

/// Install requested by the "Add to Serica" button on a Chrome Web Store page.
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
            ui_emit(&app.handle, "serica:extension-installed", rec.clone());
        }
        if let Some(wv) = TabHost::webview(&app, &tab_id) {
            // The store button lives in Serica's script world.
            crate::webview_mac::eval_serica(&wv, js);
        }
    });
}

// ------------------------------------------------------------------ downloads

#[tauri::command]
pub async fn list_downloads(app: State<'_, Shared>) -> R<Vec<crate::downloads::DownloadItem>> {
    Ok(app.downloads.list())
}

/// Starts dragging finished downloads out of the Downloads panel as files
/// (to a folder, the desktop, or a web page). Only files Serica downloaded
/// and that still exist can be dragged.
#[tauri::command]
pub async fn start_download_drag(ids: Vec<String>, app: State<'_, Shared>) -> R<bool> {
    let paths: Vec<std::path::PathBuf> = ids
        .iter()
        .filter_map(|id| app.downloads.get(id))
        .filter(|d| d.state == crate::downloads::DownloadState::Completed)
        .map(|d| std::path::PathBuf::from(d.path))
        .filter(|p| p.is_file())
        .collect();
    if paths.is_empty() {
        return Ok(false);
    }
    crate::dragdrop_mac::start_file_drag(app.inner(), paths);
    Ok(true)
}

/// `pause` | `resume` | `cancel` | `open` | `show` | `remove` | `retry`.
#[tauri::command]
pub async fn download_action(id: String, action: String, app: State<'_, Shared>) -> R<bool> {
    let item = app.downloads.get(&id).ok_or("Download not found")?;
    match action.as_str() {
        "pause" | "resume" | "cancel" => {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let (id2, action2, a2) = (id.clone(), action.clone(), app.inner().clone());
            app.handle
                .run_on_main_thread(move || {
                    let _ = tx.send(crate::downloads::native::control(&a2, &id2, &action2));
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
        "open" => {
            let path = std::path::Path::new(&item.path);
            if !path.exists() {
                return Err("The file was moved or deleted".into());
            }
            // PDFs open in a Serica tab; everything else in its own app.
            let is_pdf = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) || item.mime == "application/pdf";
            if let Some(url) = crate::files::file_url(path).filter(|_| is_pdf) {
                app.mutate(|s| s.create_tab(Some(url), None, true, None));
                return Ok(true);
            }
            open::that_detached(&item.path).map_err(|e| e.to_string())?;
            Ok(true)
        }
        "show" => crate::downloads::reveal(std::path::Path::new(&item.path)).map(|_| true),
        "remove" => {
            let removed = app.downloads.remove(&id);
            if removed {
                ui_emit(&app.handle, "serica:downloads-changed", app.downloads.list());
            }
            Ok(removed)
        }
        "retry" => {
            // WebKit downloads a URL straight from a page of the same
            // profile (`startDownloadUsingRequest`), keeping its cookies.
            let live = {
                let s = app.store.lock();
                s.active_tab().filter(|t| t.wants_webview()).map(|t| t.id.clone())
            };
            url::Url::parse(&item.url).map_err(|e| e.to_string())?;
            match live.and_then(|tid| TabHost::webview(&app, &tid).map(|wv| (tid, wv))) {
                Some((tid, wv)) => crate::downloads::native::start(app.inner(), &tid, &wv, &item.url),
                None => {
                    let url = item.url.clone();
                    app.mutate(|s| s.create_tab(Some(url), None, true, None));
                }
            }
            app.downloads.remove(&id);
            ui_emit(&app.handle, "serica:downloads-changed", app.downloads.list());
            Ok(true)
        }
        _ => Err(format!("unknown action {action}")),
    }
}

#[tauri::command]
pub async fn clear_downloads(app: State<'_, Shared>) -> R<Vec<crate::downloads::DownloadItem>> {
    app.downloads.clear_finished();
    let list = app.downloads.list();
    ui_emit(&app.handle, "serica:downloads-changed", list.clone());
    Ok(list)
}

/// "Open file…" (Ctrl+O): picks local files and opens them in new tabs.
#[tauri::command]
pub async fn open_file(app: State<'_, Shared>) -> R<()> {
    crate::files::pick_and_open(&app);
    Ok(())
}

/// Text on the system clipboard, for Paste in the app's own context menu
/// (the UI page may not read the clipboard itself).
#[tauri::command]
pub fn read_clipboard_text() -> Option<String> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    let pb = NSPasteboard::generalPasteboard();
    unsafe { pb.stringForType(NSPasteboardTypeString) }.map(|s| s.to_string())
}

#[tauri::command]
pub async fn open_downloads_folder() -> R<()> {
    open::that_detached(crate::downloads::default_folder()).map_err(|e| e.to_string())
}

// ── Updates ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_update_status(app: State<'_, Shared>) -> crate::updater::UpdateStatus {
    crate::updater::status(&app.handle)
}

#[tauri::command]
pub async fn check_for_update(app: State<'_, Shared>) -> R<crate::updater::UpdateStatus> {
    let handle = app.handle.clone();
    Ok(crate::updater::check(&handle).await)
}

#[tauri::command]
pub fn get_ui_patch_status() -> crate::ui_patch::UiPatchStatus {
    crate::ui_patch::status()
}

/// Called once the UI has rendered: the UI patch in use works.
#[tauri::command]
pub fn confirm_ui_patch() {
    crate::ui_patch::confirm()
}

/// Switches to the downloaded UI fix; the UI reloads right after.
#[tauri::command]
pub fn apply_ui_patch() -> R<()> {
    crate::ui_patch::apply()
}

#[tauri::command]
pub fn install_update(app: State<'_, Shared>) -> R<()> {
    crate::updater::install(&app.handle)
}
