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

/// MCP protocol revisions this server speaks, newest first.
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

fn is_known_tool(name: &str) -> bool {
    tools().as_array().map(|t| t.iter().any(|x| x["name"] == name)).unwrap_or(false)
}

/// Handles one JSON-RPC message. Returns the response, or `None` for
/// notifications (and stray responses), which get no reply.
pub async fn dispatch(app: &Shared, msg: &Value) -> Option<Value> {
    let method = msg["method"].as_str()?;
    let id = msg.get("id").cloned().filter(|v| !v.is_null())?;
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    app.log_mcp(json!({ "timestamp": now_ms(), "direction": "in", "method": method, "data": params }));
    let ok = |result: Value| json!({ "jsonrpc": "2.0", "id": id.clone(), "result": result });
    let err = |code: i64, message: String| json!({ "jsonrpc": "2.0", "id": id.clone(), "error": { "code": code, "message": message } });
    let resp = match method {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or("");
            let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(PROTOCOL_VERSIONS[0]);
            ok(json!({
                "protocolVersion": version,
                "serverInfo": { "name": "serica-browser", "title": "Serica Browser", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "tools": { "listChanged": false } },
                "instructions": "Controls the user's Serica browser: read status, list/open/close tabs, navigate, hibernate background tabs, control media and read ad-blocker stats. Tab ids come from browser_list_tabs."
            }))
        }
        "tools/list" => ok(json!({ "tools": tools() })),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            if name.is_empty() {
                err(-32602, "Missing tool name".into())
            } else if !is_known_tool(name) {
                err(-32602, format!("Unknown tool: {name}"))
            } else {
                let args = if params["arguments"].is_object() { params["arguments"].clone() } else { json!({}) };
                match call_tool(app, name, &args).await {
                    Ok(v) => ok(json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default() }], "isError": false })),
                    // Tool failures are results the model can read, not protocol errors.
                    Err(e) => ok(json!({ "content": [{ "type": "text", "text": e }], "isError": true })),
                }
            }
        }
        "ping" => ok(json!({})),
        _ => err(-32601, format!("Method not found: {method}")),
    };
    app.log_mcp(json!({ "timestamp": now_ms(), "direction": "out", "method": method, "data": resp }));
    Some(resp)
}

/// In-app entry point (MCP panel): one request, one response.
pub async fn handle(app: &Shared, method: &str, params: Value) -> Value {
    let msg = json!({ "jsonrpc": "2.0", "id": now_ms(), "method": method, "params": params });
    dispatch(app, &msg).await.unwrap_or(Value::Null)
}
