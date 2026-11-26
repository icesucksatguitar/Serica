//! Bookmarks brought over from other browsers, kept in `bookmarks.json`.
//!
//! Serica's own way of keeping sites is pinned tabs; imported bookmarks are
//! found from the address bar (they join its suggestions), and a browser's
//! bookmarks bar also lands in the sidebar as a pinned folder.

use crate::history::HistoryEntry;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MAX: usize = 20_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub title: String,
    pub url: String,
    /// Folder path in the source browser, e.g. "Bookmarks bar / Work".
    #[serde(default)]
    pub folder: String,
    /// Where it came from ("Google Chrome").
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub added: i64,
}

pub struct Bookmarks {
    path: PathBuf,
    list: Vec<Bookmark>,
}

impl Bookmarks {
    pub fn load(path: PathBuf) -> Self {
        let list = crate::store::read_json(&path).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        Self { path, list }
    }

    /// Adds bookmarks not already kept (same URL and folder). Returns how many were added.
    pub fn add(&mut self, items: Vec<Bookmark>) -> usize {
        let mut seen: std::collections::HashSet<(String, String)> = self.list.iter().map(|b| (b.url.clone(), b.folder.clone())).collect();
        let mut n = 0;
        for b in items {
            if !(b.url.starts_with("http://") || b.url.starts_with("https://")) || b.url.len() > 4096 {
                continue;
            }
            if self.list.len() >= MAX {
                break;
            }
            if seen.insert((b.url.clone(), b.folder.clone())) {
                self.list.push(b);
                n += 1;
            }
        }
        if n > 0 {
            if let Ok(bytes) = serde_json::to_vec(&self.list) {
                if let Err(e) = crate::store::write_atomic(&self.path, &bytes) {
                    tracing::error!("failed to save bookmarks: {e}");
                }
            }
        }
        n
    }

    /// Address-bar matches, shaped like history entries.
    pub fn suggest(&self, text: &str, limit: usize) -> Vec<HistoryEntry> {
        let n = text.trim().to_lowercase();
        if n.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(i32, &Bookmark)> = self
            .list
            .iter()
            .filter_map(|b| {
                let url = b.url.to_lowercase();
                let bare = url.trim_start_matches("https://").trim_start_matches("http://").trim_start_matches("www.");
                let score = if bare.starts_with(&n) {
                    900
                } else if b.title.to_lowercase().contains(&n) {
                    500
                } else if url.contains(&n) {
                    250
                } else {
                    return None;
                };
                Some((score, b))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        let mut seen = std::collections::HashSet::new();
        scored
            .into_iter()
            .filter(|(_, b)| seen.insert(b.url.clone()))
            .take(limit)
            .map(|(_, b)| HistoryEntry {
                id: format!("bookmark:{}", b.url),
                url: b.url.clone(),
                title: if b.title.is_empty() { b.url.clone() } else { b.title.clone() },
                timestamp: b.added,
                visit_count: 1,
                space_id: None,
                favicon: None,
            })
            .collect()
    }
}
