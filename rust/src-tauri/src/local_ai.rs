//! Local models (Ollama, LM Studio), reached from the backend.
//!
//! The AI panel used to call them from the page, but a page's requests carry
//! Serica's origin (`http://tauri.localhost`), which Ollama refuses with 403
//! unless OLLAMA_ORIGINS is set, and LM Studio only answers with CORS turned
//! on. From here there is no origin, so both work as installed. Replies are
//! streamed to the UI as `serica:local-ai` events and can be stopped.

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const LMSTUDIO: &str = "http://127.0.0.1:1234";
const EVENT: &str = "serica:local-ai";

/// Replies being written, by request id, with their stop flag.
static RUNNING: Lazy<Mutex<HashMap<String, Arc<AtomicBool>>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// Ollama's address: OLLAMA_HOST if set (`0.0.0.0`, `:11500`, `host:port`,
/// `http://host:port`), else 127.0.0.1:11434. A wildcard host is reached
/// on 127.0.0.1.
pub fn ollama_base(env: Option<&str>) -> String {
    let raw = env.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("127.0.0.1:11434");
    let (scheme, rest) = match raw.split_once("://") {
        Some((s, r)) => (s, r),
        None => ("http", raw),
    };
    let rest = rest.trim_end_matches('/');
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => (h, p),
        _ => (rest, "11434"),
    };
    let host = match host {
        "" | "0.0.0.0" | "::" | "[::]" => "127.0.0.1",
        h => h,
    };
    format!("{scheme}://{host}:{port}")
}

fn client(timeout: Option<Duration>) -> reqwest::Client {
    let mut b = reqwest::Client::builder().no_proxy().connect_timeout(Duration::from_secs(3));
    if let Some(t) = timeout {
        b = b.timeout(t);
    }
    b.build().unwrap_or_default()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModel {
    pub name: String,
    pub runtime: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalScan {
    pub ollama: bool,
    pub lmstudio: bool,
    pub models: Vec<LocalModel>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Chunk<'a> {
    id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    delta: Option<String>,
    done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Which runtimes answer, and their models.
#[tauri::command]
pub async fn local_ai_scan() -> LocalScan {
    let c = client(Some(Duration::from_secs(3)));
    let ollama = ollama_base(std::env::var("OLLAMA_HOST").ok().as_deref());
    let (o, l) = tokio::join!(c.get(format!("{ollama}/api/tags")).send(), c.get(format!("{LMSTUDIO}/v1/models")).send());
    let mut models = Vec::new();
    let mut ollama_on = false;
    if let Ok(r) = o {
        if r.status().is_success() {
            ollama_on = true;
            if let Ok(v) = r.json::<serde_json::Value>().await {
                for m in v["models"].as_array().into_iter().flatten() {
                    if let Some(n) = m["name"].as_str().or_else(|| m["model"].as_str()) {
                        models.push(LocalModel { name: n.to_string(), runtime: "ollama" });
                    }
                }
            }
        }
    }
    let mut lm_on = false;
    if let Ok(r) = l {
        if r.status().is_success() {
            lm_on = true;
            if let Ok(v) = r.json::<serde_json::Value>().await {
                for m in v["data"].as_array().into_iter().flatten() {
                    if let Some(n) = m["id"].as_str() {
                        models.push(LocalModel { name: n.to_string(), runtime: "lmstudio" });
                    }
                }
            }
        }
    }
    LocalScan { ollama: ollama_on, lmstudio: lm_on, models }
}

/// What one streamed line adds, for each runtime's format. `Err` is an
/// error the runtime reported; `Ok(None)` a line with nothing to show.
fn parse_line(runtime: &str, line: &str) -> Result<(Option<String>, bool), String> {
    let line = line.trim();
    if line.is_empty() {
        return Ok((None, false));
    }
    if runtime == "lmstudio" {
        let Some(data) = line.strip_prefix("data:") else { return Ok((None, false)) };
        let data = data.trim();
        if data == "[DONE]" {
            return Ok((None, true));
        }
        let v: serde_json::Value = serde_json::from_str(data).map_err(|_| String::new())?;
        if let Some(e) = v["error"]["message"].as_str().or_else(|| v["error"].as_str()) {
            return Err(e.to_string());
        }
        let delta = v["choices"][0]["delta"]["content"].as_str().map(str::to_string);
        let done = v["choices"][0]["finish_reason"].as_str().is_some();
        Ok((delta.filter(|d| !d.is_empty()), done))
    } else {
        let v: serde_json::Value = serde_json::from_str(line).map_err(|_| String::new())?;
        if let Some(e) = v["error"].as_str() {
            return Err(e.to_string());
        }
        let delta = v["message"]["content"].as_str().map(str::to_string);
        Ok((delta.filter(|d| !d.is_empty()), v["done"].as_bool() == Some(true)))
    }
}

async fn stream_reply(id: &str, runtime: &str, model: &str, messages: &[ChatMessage], stop: &AtomicBool, emit: &impl Fn(Chunk)) -> Result<(), String> {
    let c = client(None);
    let req = if runtime == "lmstudio" {
        c.post(format!("{LMSTUDIO}/v1/chat/completions")).json(&serde_json::json!({ "model": model, "messages": messages, "stream": true }))
    } else {
        let base = ollama_base(std::env::var("OLLAMA_HOST").ok().as_deref());
        c.post(format!("{base}/api/chat")).json(&serde_json::json!({ "model": model, "messages": messages, "stream": true }))
    };
    let name = if runtime == "lmstudio" { "LM Studio" } else { "Ollama" };
    let mut resp = req.send().await.map_err(|_| format!("{name} isn’t answering. Is it still running?"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let msg = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().or_else(|| v["error"].as_str()).map(str::to_string))
            .unwrap_or_else(|| format!("{name} answered {status}"));
        return Err(msg);
    }
    let mut buf = String::new();
    while let Some(bytes) = resp.chunk().await.map_err(|_| format!("{name} stopped answering."))? {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        buf.push_str(&String::from_utf8_lossy(&bytes));
        while let Some(nl) = buf.find('\n') {
            let line: String = buf.drain(..=nl).collect();
            match parse_line(runtime, &line) {
                Ok((delta, done)) => {
                    if delta.is_some() {
                        emit(Chunk { id, delta, done: false, error: None });
                    }
                    if done {
                        return Ok(());
                    }
                }
                Err(e) if !e.is_empty() => return Err(e),
                Err(_) => {}
            }
        }
    }
    if let Ok((delta, _)) = parse_line(runtime, &buf) {
        if delta.is_some() {
            emit(Chunk { id, delta, done: false, error: None });
        }
    }
    Ok(())
}

/// Starts a reply; its text arrives as `serica:local-ai` events with this id.
#[tauri::command]
pub fn local_ai_chat(id: String, runtime: String, model: String, messages: Vec<ChatMessage>, app: tauri::State<'_, crate::app::Shared>) {
    let stop = Arc::new(AtomicBool::new(false));
    RUNNING.lock().insert(id.clone(), stop.clone());
    let handle = app.handle.clone();
    tauri::async_runtime::spawn(async move {
        let emit = |c: Chunk| crate::app::ui_emit(&handle, EVENT, c);
        let result = stream_reply(&id, &runtime, &model, &messages, &stop, &emit).await;
        RUNNING.lock().remove(&id);
        emit(Chunk { id: &id, delta: None, done: true, error: result.err() });
    });
}

/// Stops a reply being written.
#[tauri::command]
pub fn local_ai_stop(id: String) {
    if let Some(flag) = RUNNING.lock().get(&id) {
        flag.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_host_forms() {
        assert_eq!(ollama_base(None), "http://127.0.0.1:11434");
        assert_eq!(ollama_base(Some("0.0.0.0")), "http://127.0.0.1:11434");
        assert_eq!(ollama_base(Some("0.0.0.0:11500")), "http://127.0.0.1:11500");
        assert_eq!(ollama_base(Some(":9000")), "http://127.0.0.1:9000");
        assert_eq!(ollama_base(Some("http://192.168.1.5:11434/")), "http://192.168.1.5:11434");
        assert_eq!(ollama_base(Some("https://ollama.lan")), "https://ollama.lan:11434");
    }

    #[test]
    fn parses_both_stream_formats() {
        assert_eq!(parse_line("ollama", r#"{"message":{"role":"assistant","content":"Hi"},"done":false}"#), Ok((Some("Hi".into()), false)));
        assert_eq!(parse_line("ollama", r#"{"message":{"role":"assistant","content":""},"done":true}"#), Ok((None, true)));
        assert_eq!(parse_line("ollama", r#"{"error":"model 'x' not found"}"#), Err("model 'x' not found".into()));
        assert_eq!(parse_line("lmstudio", r#"data: {"choices":[{"delta":{"content":"Yo"}}]}"#), Ok((Some("Yo".into()), false)));
        assert_eq!(parse_line("lmstudio", "data: [DONE]"), Ok((None, true)));
        assert_eq!(parse_line("lmstudio", ": keep-alive"), Ok((None, false)));
        assert_eq!(parse_line("ollama", "   "), Ok((None, false)));
    }

    /// Talks to a real Ollama on this PC: `cargo test local_ai -- --ignored`.
    #[test]
    #[ignore]
    fn live_ollama() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let scan = rt.block_on(local_ai_scan());
        assert!(scan.ollama, "Ollama isn't running");
        let model = scan.models.iter().find(|m| m.runtime == "ollama").expect("no Ollama model").name.clone();
        let out = Mutex::new(String::new());
        let stop = AtomicBool::new(false);
        let msgs = vec![ChatMessage { role: "user".into(), content: "Say hello in three words.".into() }];
        rt.block_on(stream_reply("t", "ollama", &model, &msgs, &stop, &|c: Chunk| {
            if let Some(d) = c.delta {
                out.lock().push_str(&d);
            }
        }))
        .unwrap();
        assert!(!out.lock().trim().is_empty());
        println!("{model}: {}", out.lock());
    }
}
