//! Browsing history, persisted to `history.json`.
//!
//! One entry per URL (most recent visit wins), so repeated visits bump
//! `visitCount` and `timestamp` instead of flooding the list.

use crate::model::{now_ms, new_id};
use crate::store::{read_json, write_atomic};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MAX_ENTRIES: usize = 5000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub title: String,
    pub timestamp: i64,
    #[serde(default = "one")]
    pub visit_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,
}

fn one() -> i64 {
    1
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HistoryQuery {
    pub query: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub space_id: Option<String>,
}

pub struct History {
    entries: Vec<HistoryEntry>,
    path: PathBuf,
    pub dirty: bool,
}

/// Only real web pages are recorded.
fn recordable(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://")) && url.len() < 4096
}

impl History {
    pub fn load(path: PathBuf) -> Self {
        let entries = read_json(&path)
            .and_then(|v| serde_json::from_value::<Vec<HistoryEntry>>(v).ok())
            .unwrap_or_default();
        Self { entries, path, dirty: false }
    }

    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        if let Ok(bytes) = serde_json::to_vec(&self.entries) {
            if let Err(e) = write_atomic(&self.path, &bytes) {
                tracing::error!("failed to save history: {e}");
                return;
            }
        }
        self.dirty = false;
    }

    pub fn record_visit(&mut self, url: &str, title: &str, space_id: Option<&str>) {
        if !recordable(url) {
            return;
        }
        let now = now_ms();
        if let Some(pos) = self.entries.iter().position(|e| e.url == url) {
            let mut e = self.entries.remove(pos);
            // Ignore reloads/redirect echoes within a few seconds.
            if now - e.timestamp > 5_000 {
                e.visit_count += 1;
            }
            e.timestamp = now;
            if !title.is_empty() {
                e.title = title.to_string();
            }
            if space_id.is_some() {
                e.space_id = space_id.map(str::to_string);
            }
            self.entries.push(e);
        } else {
            self.entries.push(HistoryEntry {
                id: new_id("hist"),
                url: url.to_string(),
                title: if title.is_empty() { url.to_string() } else { title.to_string() },
                timestamp: now,
                visit_count: 1,
                space_id: space_id.map(str::to_string),
                favicon: None,
            });
        }
        if self.entries.len() > MAX_ENTRIES {
            let extra = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(0..extra);
        }
        self.dirty = true;
    }

    /// Updates title/favicon of the latest entry for `url` (they arrive after the visit).
    pub fn update_meta(&mut self, url: &str, title: Option<&str>, favicon: Option<&str>) {
        if let Some(e) = self.entries.iter_mut().rev().find(|e| e.url == url) {
            if let Some(t) = title.filter(|t| !t.is_empty()) {
                if e.title != t {
                    e.title = t.to_string();
                    self.dirty = true;
                }
            }
            if let Some(f) = favicon {
                if e.favicon.as_deref() != Some(f) {
                    e.favicon = Some(f.to_string());
                    self.dirty = true;
                }
            }
        }
    }

    pub fn query(&self, q: &HistoryQuery) -> Vec<HistoryEntry> {
        let needle = q.query.as_deref().map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
        let mut list: Vec<&HistoryEntry> = self
            .entries
            .iter()
            .filter(|e| match &needle {
                Some(n) => e.title.to_lowercase().contains(n) || e.url.to_lowercase().contains(n),
                None => true,
            })
            .filter(|e| q.space_id.is_none() || e.space_id == q.space_id)
            .collect();
        list.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        list.into_iter()
            .skip(q.offset.unwrap_or(0))
            .take(q.limit.unwrap_or(100).min(MAX_ENTRIES))
            .cloned()
            .collect()
    }

    /// Top matches for omnibox suggestions: prefix/host matches first, then by visits.
    pub fn suggest(&self, text: &str, limit: usize) -> Vec<HistoryEntry> {
        let n = text.trim().to_lowercase();
        if n.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(i64, &HistoryEntry)> = self
            .entries
            .iter()
            .filter_map(|e| {
                let url = e.url.to_lowercase();
                let bare = url.trim_start_matches("https://").trim_start_matches("http://").trim_start_matches("www.");
                let title = e.title.to_lowercase();
                let score = if bare.starts_with(&n) {
                    1000
                } else if url.contains(&n) {
                    300
                } else if title.contains(&n) {
                    200
                } else {
                    return None;
                };
                Some((score + e.visit_count.min(200) * 3, e))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.timestamp.cmp(&a.1.timestamp)));
        scored.into_iter().take(limit).map(|(_, e)| e.clone()).collect()
    }

    pub fn delete(&mut self, id: Option<&str>, url: Option<&str>) -> bool {
        let before = self.entries.len();
        if let Some(id) = id {
            self.entries.retain(|e| e.id != id);
        } else if let Some(url) = url {
            self.entries.retain(|e| e.url != url);
        }
        let changed = before != self.entries.len();
        self.dirty |= changed;
        changed
    }

    /// Clears the last hour, the last day, or everything. Returns entries removed.
    pub fn clear(&mut self, range: Option<&str>) -> usize {
        let before = self.entries.len();
        let now = now_ms();
        match range {
            Some("hour") => self.entries.retain(|e| e.timestamp < now - 3_600_000),
            Some("day") => self.entries.retain(|e| e.timestamp < now - 86_400_000),
            _ => self.entries.clear(),
        }
        let removed = before - self.entries.len();
        self.dirty |= removed > 0;
        removed
    }

    pub fn recent(&self, limit: usize) -> Vec<HistoryEntry> {
        self.query(&HistoryQuery { limit: Some(limit), ..Default::default() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h() -> History {
        History::load(std::env::temp_dir().join(format!("hist-{}.json", uuid::Uuid::new_v4())))
    }

    #[test]
    fn dedupes_and_counts() {
        let mut h = h();
        h.record_visit("https://a.com/", "A", None);
        h.entries[0].timestamp -= 10_000;
        h.record_visit("https://a.com/", "", None);
        h.record_visit("neutrino://newtab", "", None);
        let all = h.recent(10);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].visit_count, 2);
        assert_eq!(all[0].title, "A");
    }

    #[test]
    fn clear_ranges_and_persist() {
        let mut h = h();
        h.record_visit("https://old.com/", "Old", None);
        h.entries[0].timestamp -= 2 * 3_600_000;
        h.record_visit("https://new.com/", "New", None);
        assert_eq!(h.clear(Some("hour")), 1);
        h.save();
        let again = History::load(h.path.clone());
        assert_eq!(again.recent(10)[0].url, "https://old.com/");
    }

    #[test]
    fn suggest_prefers_prefix() {
        let mut h = h();
        h.record_visit("https://docs.rs/", "Docs", None);
        h.record_visit("https://example.com/rust", "Rust example", None);
        assert_eq!(h.suggest("doc", 5)[0].url, "https://docs.rs/");
    }
}
