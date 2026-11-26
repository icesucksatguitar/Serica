//! Page captures (tab previews, and the page drawn under UI that covers it),
//! kept as JPEG bytes and served to the UI over the `sericacap` protocol:
//! `http://sericacap.localhost/<tab id>/<version>.jpg`.
//!
//! They used to travel as base64 data URLs inside IPC replies: a capture is
//! 150 KB-1 MB, and parsing that string cost the UI's main thread 40-60 ms
//! per tab switch. An image URL is loaded and decoded off the main thread.
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Default)]
pub struct Previews {
    /// Tab id -> (version, JPEG bytes). Its own small lock: the protocol
    /// handler runs on the main thread and must never wait on the host lock.
    map: Mutex<HashMap<String, (u64, Arc<Vec<u8>>)>>,
    seq: AtomicU64,
}

fn url(tab_id: &str, ver: u64) -> String {
    format!("http://sericacap.localhost/{tab_id}/{ver}.jpg")
}

impl Previews {
    /// Stores a capture of `tab_id`; returns its URL.
    pub fn put(&self, tab_id: &str, jpeg: Vec<u8>) -> String {
        let ver = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        self.map.lock().insert(tab_id.to_string(), (ver, Arc::new(jpeg)));
        url(tab_id, ver)
    }

    /// URL of the latest capture of `tab_id`.
    pub fn url(&self, tab_id: &str) -> Option<String> {
        self.map.lock().get(tab_id).map(|(v, _)| url(tab_id, *v))
    }

    pub fn remove(&self, tab_id: &str) {
        self.map.lock().remove(tab_id);
    }

    /// Serves `/<tab id>/<version>.jpg` (the latest capture of that tab).
    pub fn serve(&self, request: &tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
        let path = request.uri().path().trim_start_matches('/');
        let tab = path.split('/').next().unwrap_or("");
        let bytes = self.map.lock().get(tab).map(|(_, b)| b.clone());
        let builder = tauri::http::Response::builder().header("Access-Control-Allow-Origin", "*");
        match bytes {
            Some(b) => builder
                .status(200)
                .header("Content-Type", "image/jpeg")
                .header("Cache-Control", "private, max-age=31536000, immutable")
                .body(b.as_ref().clone()),
            None => builder.status(404).body(Vec::new()),
        }
        .unwrap_or_else(|_| tauri::http::Response::new(Vec::new()))
    }
}
