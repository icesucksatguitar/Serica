//! Model Context Protocol tool server (JSON-RPC methods `initialize`,
//! `tools/list`, `tools/call`) letting assistants drive the browser.

use crate::app::{normalize_input, Shared};
use crate::model::now_ms;
use crate::tabhost::TabHost;
use serde_json::{json, Value};

fn tools() -> Value {
    let obj = |props: Value, req: &[&str]| json!({ "type": "object", "properties": props, "required": req });
    json!([
        { "name": "browser_get_status", "description": "Summary of the browser: active space, active tab, tab count, shields state.", "inputSchema": obj(json!({}), &[]) },
        { "name": "browser_navigate", "description": "Navigate the active tab to a URL or search query.", "inputSchema": obj(json!({ "url": { "type": "string" } }), &["url"]) },
        { "name": "browser_list_tabs", "description": "List open tabs in the active space.", "inputSchema": obj(json!({ "allSpaces": { "type": "boolean" } }), &[]) },
        { "name": "browser_create_tab", "description": "Open a new tab in the active space.", "inputSchema": obj(json!({ "url": { "type": "string" } }), &[]) },
        { "name": "browser_close_tab", "description": "Close a tab by id.", "inputSchema": obj(json!({ "tabId": { "type": "string" } }), &["tabId"]) },
        { "name": "browser_get_adblock_stats", "description": "Shields (Brave adblock engine) statistics.", "inputSchema": obj(json!({}), &[]) },
        { "name": "browser_zen_optimize", "description": "Put inactive tabs to sleep to free memory.", "inputSchema": obj(json!({}), &[]) },
        { "name": "browser_media_control", "description": "Control media in a tab: play, pause, toggle, mute, unmute, seek, volume.",
          "inputSchema": obj(json!({ "action": { "type": "string" }, "tabId": { "type": "string" }, "value": { "type": "number" } }), &["action"]) },
    ])
}

async fn call_tool(app: &Shared, name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "browser_get_status" => {
            let s = app.store.lock();
            let active = s.active_tab().cloned();
            Ok(json!({
                "activeSpace": s.space(&s.active_space_id).map(|sp| sp.name.clone()),
                "activeTab": active.map(|t| json!({ "id": t.id, "title": t.title, "url": t.url })),
                "tabCount": s.tabs.len(),
                "shieldsEnabled": s.settings.adblock_enabled,
            }))
        }
        "browser_navigate" => {
            let url = args["url"].as_str().ok_or("missing url")?;
            let prefix = app.settings().search_prefix();
            let target = normalize_input(url, prefix).ok_or("invalid url")?;
            let id = app.store.lock().active_tab_id.clone().ok_or("no active tab")?;
            app.mutate(|s| {
                if let Some(t) = s.tab_mut(&id) {
                    t.url = target.clone();
                    t.is_loading = true;
                    t.is_sleeping = false;
                }
            });
            TabHost::navigate(app, &id, &target);
            Ok(json!({ "ok": true, "tabId": id, "url": target }))
        }
        "browser_list_tabs" => {
            let s = app.store.lock();
            let all = args["allSpaces"].as_bool().unwrap_or(false);
            let tabs: Vec<Value> = s
                .tabs
                .iter()
                .filter(|t| all || t.space_id == s.active_space_id)
                .map(|t| json!({ "id": t.id, "title": t.title, "url": t.url, "pinned": t.is_pinned, "sleeping": t.is_sleeping, "spaceId": t.space_id, "active": s.active_tab_id.as_deref() == Some(t.id.as_str()) }))
                .collect();
            Ok(json!({ "tabs": tabs }))
        }
        "browser_create_tab" => {
            let prefix = app.settings().search_prefix();
            let url = args["url"].as_str().and_then(|u| normalize_input(u, prefix));
            let tab = app.mutate(|s| s.create_tab(url, None, true, None));
            Ok(json!({ "ok": true, "tab": { "id": tab.id, "url": tab.url } }))
        }
        "browser_close_tab" => {
            let id = args["tabId"].as_str().ok_or("missing tabId")?.to_string();
            let closed = app.mutate(|s| s.close_tab(&id));
            Ok(json!({ "ok": closed }))
        }
        "browser_get_adblock_stats" => Ok(json!({ "stats": app.shields.stats(), "engine": app.shields.info.lock().clone() })),
        "browser_zen_optimize" => {
            let n = app.mutate(|s| s.sleep_inactive(0));
            Ok(json!({ "sleepingCount": n, "memorySavedMb": n as f64 * crate::store::MB_PER_SLEEPING_TAB }))
        }
        "browser_media_control" => {
            let action = args["action"].as_str().ok_or("missing action")?.to_string();
            let tab_id = match args["tabId"].as_str() {
                Some(t) => t.to_string(),
                None => {
                    let s = app.store.lock();
                    s.tabs.iter().find(|t| t.has_active_media).or_else(|| s.active_tab()).map(|t| t.id.clone()).ok_or("no tab")?
                }
            };
            let state: tauri::State<'_, Shared> = tauri::Manager::state(&app.handle);
            crate::commands::media_control(tab_id.clone(), action, args["value"].as_f64(), state).await?;
            Ok(json!({ "ok": true, "tabId": tab_id }))
        }
        _ => Err(format!("Tool not found: {name}")),
    }
}

/// Handles one JSON-RPC request and returns the JSON-RPC response.
pub async fn handle(app: &Shared, method: &str, params: Value) -> Value {
    let id = now_ms();
    app.log_mcp(json!({ "timestamp": now_ms(), "direction": "in", "method": method, "data": params }));
    let resp = match method {
        "initialize" => json!({ "jsonrpc": "2.0", "id": id, "result": {
            "protocolVersion": "2024-11-05",
            "serverInfo": { "name": "neutrino-browser-mcp", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": { "tools": {} } } }),
        "tools/list" => json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tools() } }),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            if name.is_empty() {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32602, "message": "Missing tool name parameter" } })
            } else {
                match call_tool(app, name, &params["arguments"]).await {
                    Ok(v) => json!({ "jsonrpc": "2.0", "id": id, "result": { "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default() }] } }),
                    Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": e } }),
                }
            }
        }
        "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
        _ => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("Method not found: {method}") } }),
    };
    app.log_mcp(json!({ "timestamp": now_ms(), "direction": "out", "method": method, "data": resp }));
    resp
}
