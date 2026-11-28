//! Core data model shared by persistence, IPC and the tab host.
//!
//! Every struct is serialized in camelCase to match the TypeScript types in
//! `src/types.ts`. All non-identity fields carry serde defaults so state written
//! by older builds (or imported from the Electron app) still loads instead of
//! silently falling back to a fresh profile.

use serde::{Deserialize, Serialize};

pub const NEW_TAB_URL: &str = "serica://newtab";
pub const DEFAULT_PROFILE_ID: &str = "default";

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
}

/// True for URLs rendered by the app itself instead of a web page.
pub fn is_internal_url(url: &str) -> bool {
    let u = url.trim();
    u.is_empty() || u.starts_with("serica://") || u.starts_with("neutrino://") || u == "about:blank"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Section {
    Pinned,
    #[default]
    Today,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Space {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_space_icon")]
    pub icon: String,
    #[serde(default = "default_space_color")]
    pub color: String,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub order: Option<i32>,
}

fn default_space_icon() -> String {
    "Sparkles".into()
}
fn default_space_color() -> String {
    "#8B5CF6".into()
}

fn default_new_tab_url() -> String {
    NEW_TAB_URL.into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabState {
    pub id: String,
    pub space_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default = "default_new_tab_url")]
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,
    #[serde(default)]
    pub is_loading: bool,
    #[serde(default)]
    pub can_go_back: bool,
    #[serde(default)]
    pub can_go_forward: bool,
    #[serde(default)]
    pub is_pinned: bool,
    #[serde(default)]
    pub is_sleeping: bool,
    #[serde(default)]
    pub has_active_media: bool,
    #[serde(default)]
    pub blocked_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom_factor: Option<f64>,
    /// Sidebar section; `is_pinned` always mirrors `section == Pinned`.
    #[serde(default)]
    pub section: Option<Section>,
    /// Closed pinned tab: the entry stays in the sidebar but no page is loaded.
    #[serde(default)]
    pub unloaded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
    /// A private tab: an InPrivate page that keeps no history, cookies, site
    /// data or cache once closed, and is never saved, slept, archived,
    /// pinned or reopened.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub private: bool,
}

impl TabState {
    pub fn new(space_id: &str, url: Option<String>) -> Self {
        let url = url
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty())
            .unwrap_or_else(default_new_tab_url);
        let title = if is_internal_url(&url) { "New Tab".to_string() } else { url.clone() };
        Self {
            id: new_id("tab"),
            space_id: space_id.to_string(),
            title,
            url,
            favicon: None,
            is_loading: false,
            can_go_back: false,
            can_go_forward: false,
            is_pinned: false,
            is_sleeping: false,
            has_active_media: false,
            blocked_count: 0,
            zoom_factor: Some(1.0),
            section: Some(Section::Today),
            unloaded: false,
            last_active_at: Some(now_ms()),
            folder_id: None,
            private: false,
        }
    }

    pub fn section(&self) -> Section {
        self.section.unwrap_or(if self.is_pinned { Section::Pinned } else { Section::Today })
    }

    pub fn set_section(&mut self, section: Section) {
        self.section = Some(section);
        self.is_pinned = section == Section::Pinned;
        if section != Section::Pinned {
            self.folder_id = None;
        }
    }

    /// True when this tab should have a live web page (a native webview).
    pub fn wants_webview(&self) -> bool {
        !self.is_sleeping && !self.unloaded && !is_internal_url(&self.url)
    }

    /// Clears runtime-only flags. Called on load so a crash mid-load does not
    /// leave spinners or stale history buttons around after restart.
    pub fn reset_runtime(&mut self) {
        self.is_loading = false;
        self.can_go_back = false;
        self.can_go_forward = false;
        self.has_active_media = false;
        self.blocked_count = 0;
        let s = self.section();
        self.set_section(s);
        if self.title.is_empty() {
            self.title = if is_internal_url(&self.url) { "New Tab".into() } else { self.url.clone() };
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteEntry {
    pub id: String,
    pub profile_id: String,
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,
    #[serde(default)]
    pub order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderRecord {
    pub id: String,
    pub space_id: String,
    pub name: String,
    #[serde(default)]
    pub order: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedTab {
    pub id: String,
    pub profile_id: String,
    pub space_id: String,
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,
    pub archived_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ShieldStats {
    #[serde(default)]
    pub total_blocked: i64,
    #[serde(default)]
    pub ads_blocked: i64,
    #[serde(default)]
    pub trackers_blocked: i64,
    #[serde(default)]
    pub analytics_blocked: i64,
    #[serde(default)]
    pub fingerprinters_blocked: i64,
    #[serde(default)]
    pub data_saved_kb: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedTrackerItem {
    pub url: String,
    pub domain: String,
    pub category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    pub timestamp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceStats {
    pub memory_saved_mb: f64,
    pub sleeping_count: usize,
}

/// Snapshot sent to the UI on every `serica:state-update`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAppState {
    pub spaces: Vec<Space>,
    pub active_space_id: String,
    pub tabs: Vec<TabState>,
    pub active_tab_id: Option<String>,
    pub shield_stats: ShieldStats,
    pub resource_stats: ResourceStats,
    pub profiles: Vec<Profile>,
    pub active_profile_id: String,
    pub favorites: Vec<FavoriteEntry>,
    pub folders: Vec<FolderRecord>,
}
