//! Local MCP endpoint so external agents (Claude Code, Claude Desktop via
//! `mcp-remote`, any MCP client) can drive the browser.
//!
//! Streamable HTTP transport, JSON responses only (no SSE stream), stateless:
//! `POST http://127.0.0.1:<port>/mcp` with a JSON-RPC message. Off by default
//! (`mcpServerEnabled`). Every request must carry `Authorization: Bearer
//! <token>`; requests with a non-local `Origin` are refused so web pages can't
//! reach it (DNS-rebinding protection, as the MCP spec requires).

use crate::app::Shared;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::async_runtime::JoinHandle;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub const DEFAULT_PORT: u16 = 7531;
const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

struct Running {
    port: u16,
    task: JoinHandle<()>,
}

pub struct McpHttp {
    token_file: PathBuf,
    token: Mutex<String>,
    running: Mutex<Option<Running>>,
    last_error: Mutex<Option<String>>,
}

fn new_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// Constant-time comparison (the token is a credential).
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// True for `Origin` values a local client may send.
fn local_origin(origin: &str) -> bool {
    let Ok(u) = url::Url::parse(origin) else { return false };
    matches!(u.host_str(), Some("127.0.0.1") | Some("localhost") | Some("[::1]"))
}

impl McpHttp {
    pub fn new(token_file: PathBuf) -> Self {
        let token = std::fs::read_to_string(&token_file).ok().map(|t| t.trim().to_string()).filter(|t| t.len() >= 32);
        let token = token.unwrap_or_else(|| {
            let t = new_token();
            let _ = crate::store::write_atomic(&token_file, t.as_bytes());
            t
        });
        Self { token_file, token: Mutex::new(token), running: Mutex::new(None), last_error: Mutex::new(None) }
    }

    pub fn token(&self) -> String {
        self.token.lock().clone()
    }

    pub fn regenerate_token(&self) -> String {
        let t = new_token();
        let _ = crate::store::write_atomic(&self.token_file, t.as_bytes());
        *self.token.lock() = t.clone();
        t
    }

    pub fn status(&self, enabled: bool, port: u16) -> Value {
        let running = self.running.lock().as_ref().map(|r| r.port);
        json!({
            "enabled": enabled,
            "running": running.is_some(),
            "port": running.unwrap_or(port),
            "url": format!("http://127.0.0.1:{}/mcp", running.unwrap_or(port)),
            "token": self.token(),
            "error": self.last_error.lock().clone(),
        })
    }
}

/// Starts or stops the endpoint to match settings. Safe to call repeatedly.
pub fn sync(app: &Shared) {
    let (enabled, port) = {
        let s = app.store.lock();
        (s.settings.mcp_server_enabled, s.settings.mcp_server_port)
    };
    let mut running = app.mcp_http.running.lock();
    if let Some(r) = running.as_ref() {
        if enabled && r.port == port {
            return;
        }
        r.task.abort();
        *running = None;
    }
    *app.mcp_http.last_error.lock() = None;
    if !enabled {
        return;
    }
    // Bind synchronously so a busy port is reported right away.
    let std_listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("MCP endpoint could not listen on 127.0.0.1:{port}: {e}");
            *app.mcp_http.last_error.lock() = Some(format!("Port {port} is unavailable: {e}"));
            return;
        }
    };
    let _ = std_listener.set_nonblocking(true);
    let app2 = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        let listener = match TcpListener::from_std(std_listener) {
            Ok(l) => l,
            Err(e) => {
                *app2.mcp_http.last_error.lock() = Some(e.to_string());
                return;
            }
        };
        tracing::info!("MCP endpoint listening on http://127.0.0.1:{port}/mcp");
        loop {
            let Ok((stream, _)) = listener.accept().await else { continue };
            let app3 = app2.clone();
            tokio::spawn(async move {
                let _ = serve_connection(app3, stream).await;
            });
        }
    });
    *running = Some(Running { port, task });
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Reads one HTTP/1.1 request; `Ok(None)` when the client closed the connection.
async fn read_request(stream: &mut TcpStream, buf: &mut Vec<u8>) -> std::io::Result<Option<Request>> {
    let header_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEADER_BYTES {
            return Err(std::io::Error::other("headers too large"));
        }
        let mut chunk = [0u8; 8192];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let method = first.next().unwrap_or("").to_string();
    let path = first.next().unwrap_or("").to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .collect();
    let len: usize = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    if len > MAX_BODY_BYTES {
        return Err(std::io::Error::other("body too large"));
    }
    let body_start = header_end + 4;
    while buf.len() < body_start + len {
        let mut chunk = [0u8; 8192];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body = buf[body_start..body_start + len].to_vec();
    buf.drain(..body_start + len);
    Ok(Some(Request { method, path, headers, body }))
}

async fn respond(stream: &mut TcpStream, status: &str, extra: &[(&str, &str)], body: &[u8]) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n", body.len());
    if !body.is_empty() {
        head.push_str("Content-Type: application/json\r\n");
    }
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

fn rpc_error(id: Value, code: i64, message: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })).unwrap_or_default()
}

async fn serve_connection(app: Shared, mut stream: TcpStream) -> std::io::Result<()> {
    let mut buf = Vec::new();
    while let Some(req) = read_request(&mut stream, &mut buf).await? {
        let close = req.header("connection").map(|v| v.eq_ignore_ascii_case("close")).unwrap_or(false);
        handle_http(&app, &mut stream, req).await?;
        if close {
            break;
        }
    }
    Ok(())
}

async fn handle_http(app: &Shared, stream: &mut TcpStream, req: Request) -> std::io::Result<()> {
    if let Some(origin) = req.header("origin") {
        if !local_origin(origin) {
            return respond(stream, "403 Forbidden", &[], &rpc_error(Value::Null, -32000, "Origin not allowed")).await;
        }
    }
    let path = req.path.split('?').next().unwrap_or("");
    if path != "/mcp" {
        return respond(stream, "404 Not Found", &[], &rpc_error(Value::Null, -32000, "Not found: use /mcp")).await;
    }
    let token = app.mcp_http.token();
    let authorized = req
        .header("authorization")
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
        .map(|t| same(t.trim(), &token))
        .unwrap_or(false);
    if !authorized {
        return respond(stream, "401 Unauthorized", &[("WWW-Authenticate", "Bearer")], &rpc_error(Value::Null, -32001, "Missing or invalid access token")).await;
    }
    match req.method.as_str() {
        "POST" => {}
        // No server-initiated stream and no sessions to end.
        "GET" | "DELETE" => return respond(stream, "405 Method Not Allowed", &[("Allow", "POST")], b"").await,
        _ => return respond(stream, "405 Method Not Allowed", &[("Allow", "POST")], b"").await,
    }
    let Ok(msg) = serde_json::from_slice::<Value>(&req.body) else {
        return respond(stream, "400 Bad Request", &[], &rpc_error(Value::Null, -32700, "Parse error")).await;
    };
    let replies: Vec<Value> = match &msg {
        Value::Array(batch) => {
            let mut out = Vec::new();
            for m in batch {
                if let Some(r) = crate::mcp::dispatch(app, m).await {
                    out.push(r);
                }
            }
            out
        }
        _ => crate::mcp::dispatch(app, &msg).await.into_iter().collect(),
    };
    if replies.is_empty() {
        // Only notifications/responses were sent.
        return respond(stream, "202 Accepted", &[], b"").await;
    }
    let body = if msg.is_array() { json!(replies) } else { replies.into_iter().next().unwrap_or(Value::Null) };
    respond(stream, "200 OK", &[], &serde_json::to_vec(&body).unwrap_or_default()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins() {
        assert!(local_origin("http://localhost:6274"));
        assert!(local_origin("http://127.0.0.1"));
        assert!(!local_origin("https://evil.example"));
        assert!(!local_origin("http://127.0.0.1.evil.example"));
        assert!(!local_origin("null"));
    }

    #[test]
    fn token_compare() {
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd"));
        assert!(!same("abc", "abcd"));
        assert_eq!(new_token().len(), 64);
    }
}
