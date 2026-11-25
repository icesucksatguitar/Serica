//! Browser settings. Persisted as `settings.json` in the app data directory.
//!
//! Only the keys the backend acts on are typed. Theme data (`spaceThemes`,
//! `customThemePresets`) is owned by the UI's theme studio and stored opaquely,
//! so a shape change on the UI side can never make the whole file fail to load.
//! Unknown keys (`theme`, `userProfile`, `pinnedSites`, ...) are kept verbatim.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

pub const SEARCH_ENGINES: &[(&str, &str)] = &[
    ("google", "https://www.google.com/search?q="),
    ("duckduckgo", "https://duckduckgo.com/?q="),
    ("bing", "https://www.bing.com/search?q="),
    ("brave", "https://search.brave.com/search?q="),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceStatus {
    pub hardware_acceleration: bool,
    pub gpu_disabled_flag: bool,
    pub software_rasterized: bool,
    pub reduced_effects_mode: String,
    pub reduced_effects_active: bool,
    pub gpu_feature_status: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSettings {
    #[serde(default = "yes")]
    pub adblock_enabled: bool,
    #[serde(default = "yes")]
    pub zen_sleep_enabled: bool,
    /// Minutes a background tab stays unused before Zen sleeps it (see `TAB_SLEEP_MINUTES`).
    #[serde(default = "default_tab_sleep_minutes")]
    pub tab_sleep_minutes: u32,
    #[serde(default = "default_search_engine")]
    pub search_engine: String,
    #[serde(default = "default_reduced_effects")]
    pub reduced_effects_mode: String,
    #[serde(default)]
    pub space_themes: Map<String, Value>,
    #[serde(default)]
    pub custom_theme_presets: Vec<Value>,
    #[serde(default)]
    pub shield_whitelist: Vec<String>,
    /// 12 | 24 | 168 | 720 | "never"
    #[serde(default = "default_archive_interval")]
    pub archive_interval_hours: Value,
    #[serde(default = "yes")]
    pub right_pins_dock_visible: bool,
    #[serde(default = "yes")]
    pub sidebar_expanded: bool,
    #[serde(default)]
    pub sidebar_auto_hide: bool,
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f64,
    /// Text size: scales the browser interface and is the default zoom for new pages.
    #[serde(default = "default_text_scale")]
    pub text_scale: f64,
    /// Pop a playing video into picture-in-picture when switching away from its tab.
    #[serde(default = "yes")]
    pub auto_picture_in_picture: bool,
    /// Show PDFs with black pages and white text.
    #[serde(default = "yes")]
    pub pdf_dark_mode: bool,
    /// Block cookies of sites other than the one in the address bar (sites
    /// embedded in pages, trackers). Applies to pages after they reload.
    #[serde(default)]
    pub block_third_party_cookies: bool,
    /// Delete cookies when Serica closes, except for `cookie_keep_sites`.
    #[serde(default)]
    pub clear_cookies_on_exit: bool,
    /// Sites (and their subdomains) whose cookies are always kept.
    #[serde(default)]
    pub cookie_keep_sites: Vec<String>,
    /// Page zoom per site (host without `www.`), as other browsers keep it.
    /// Sites at the default zoom (the text size) have no entry.
    #[serde(default)]
    pub site_zoom: std::collections::BTreeMap<String, f64>,
    /// Local MCP endpoint for external agents (off by default).
    #[serde(default)]
    pub mcp_server_enabled: bool,
    #[serde(default = "default_mcp_port")]
    pub mcp_server_port: u16,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn yes() -> bool {
    true
}
fn default_sidebar_width() -> f64 {
    240.0
}
pub const TEXT_SCALES: [f64; 4] = [0.9, 1.0, 1.1, 1.25];
fn default_text_scale() -> f64 {
    1.0
}
pub const TAB_SLEEP_MINUTES: [u32; 6] = [15, 30, 60, 120, 240, 720];
fn default_tab_sleep_minutes() -> u32 {
    60
}
fn default_mcp_port() -> u16 {
    crate::mcp_http::DEFAULT_PORT
}
fn default_search_engine() -> String {
    "google".into()
}
fn default_reduced_effects() -> String {
    "auto".into()
}
fn default_archive_interval() -> Value {
    Value::from(12)
}

impl Default for BrowserSettings {
    fn default() -> Self {
        Self::from_value(Value::Object(Map::new()))
    }
}

/// Normalizes a whitelist entry to a bare lowercase hostname.
pub fn normalize_host(input: &str) -> String {
    let mut s = input.trim().to_lowercase();
    if s.contains("://") {
        if let Ok(u) = url::Url::parse(&s) {
            s = u.host_str().unwrap_or("").to_string();
        }
    }
    let s = s.split('/').next().unwrap_or("");
    let s = s.rsplit('@').next().unwrap_or("");
    let s = s.split(':').next().unwrap_or("");
    s.trim_start_matches("www.").trim_matches('.').to_string()
}

impl BrowserSettings {
    /// Builds settings from any JSON value, never failing: invalid typed fields
    /// are dropped and fall back to their defaults.
    pub fn from_value(value: Value) -> Self {
        let mut obj = match value {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        // Drop typed keys whose JSON type is wrong so one bad key can't reset the file.
        let typed: &[(&str, fn(&Value) -> bool)] = &[
            ("adblockEnabled", Value::is_boolean),
            ("zenSleepEnabled", Value::is_boolean),
            ("tabSleepMinutes", Value::is_u64),
            ("searchEngine", Value::is_string),
            ("reducedEffectsMode", Value::is_string),
            ("spaceThemes", Value::is_object),
            ("customThemePresets", Value::is_array),
            ("shieldWhitelist", Value::is_array),
            ("rightPinsDockVisible", Value::is_boolean),
            ("sidebarExpanded", Value::is_boolean),
            ("sidebarAutoHide", Value::is_boolean),
            ("sidebarWidth", Value::is_number),
            ("textScale", Value::is_number),
            ("mcpServerEnabled", Value::is_boolean),
            ("autoPictureInPicture", Value::is_boolean),
            ("pdfDarkMode", Value::is_boolean),
            ("siteZoom", Value::is_object),
            ("blockThirdPartyCookies", Value::is_boolean),
            ("clearCookiesOnExit", Value::is_boolean),
            ("cookieKeepSites", Value::is_array),
            ("mcpServerPort", Value::is_u64),
        ];
        for (key, ok) in typed {
            if obj.get(*key).map(|v| !ok(v)).unwrap_or(false) {
                obj.remove(*key);
            }
        }
        for key in ["shieldWhitelist", "cookieKeepSites"] {
            if let Some(Value::Array(list)) = obj.get(key) {
                if list.iter().any(|v| !v.is_string()) {
                    obj.remove(key);
                }
            }
        }
        let mut s: BrowserSettings = serde_json::from_value(Value::Object(obj)).unwrap_or_else(|e| {
            tracing::warn!("settings could not be parsed, using defaults: {e}");
            serde_json::from_value(Value::Object(Map::new())).expect("default settings")
        });
        s.normalize();
        s
    }

    pub fn normalize(&mut self) {
        if self.mcp_server_port < 1024 {
            self.mcp_server_port = default_mcp_port();
        }
        if !SEARCH_ENGINES.iter().any(|(id, _)| *id == self.search_engine) {
            self.search_engine = default_search_engine();
        }
        if !matches!(self.reduced_effects_mode.as_str(), "auto" | "on" | "off") {
            self.reduced_effects_mode = default_reduced_effects();
        }
        let valid_interval = match &self.archive_interval_hours {
            Value::Number(n) => matches!(n.as_i64(), Some(12 | 24 | 168 | 720)),
            Value::String(s) => s == "never",
            _ => false,
        };
        if !valid_interval {
            self.archive_interval_hours = default_archive_interval();
        }
        if !TAB_SLEEP_MINUTES.contains(&self.tab_sleep_minutes) {
            self.tab_sleep_minutes = default_tab_sleep_minutes();
        }
        if !self.sidebar_width.is_finite() {
            self.sidebar_width = default_sidebar_width();
        }
        self.sidebar_width = self.sidebar_width.clamp(160.0, 520.0);
        // Snap to the offered sizes (also replaces NaN/inf with the default).
        let requested = if self.text_scale.is_finite() { self.text_scale } else { default_text_scale() };
        self.text_scale = TEXT_SCALES
            .iter()
            .copied()
            .min_by(|a, b| (a - requested).abs().total_cmp(&(b - requested).abs()))
            .unwrap_or(1.0);
        let mut seen = std::collections::HashSet::new();
        self.shield_whitelist = self
            .shield_whitelist
            .iter()
            .map(|h| normalize_host(h))
            .filter(|h| !h.is_empty() && seen.insert(h.clone()))
            .collect();
        // Typed keys must never also live in `extra`, or they'd be serialized twice.
        for k in [
            "adblockEnabled", "zenSleepEnabled", "tabSleepMinutes", "searchEngine", "reducedEffectsMode", "spaceThemes",
            "customThemePresets", "shieldWhitelist", "archiveIntervalHours", "rightPinsDockVisible",
            "sidebarExpanded", "sidebarAutoHide", "sidebarWidth", "textScale",
        ] {
            self.extra.remove(k);
        }
    }

    /// Applies a partial update: top-level keys in `patch` replace existing ones;
    /// a `null` value removes an untyped key.
    pub fn apply_patch(&mut self, patch: &Value) {
        let Value::Object(patch) = patch else { return };
        let mut current = serde_json::to_value(&*self).unwrap_or(Value::Object(Map::new()));
        if let Value::Object(map) = &mut current {
            for (k, v) in patch {
                if v.is_null() {
                    map.remove(k);
                } else {
                    map.insert(k.clone(), v.clone());
                }
            }
        }
        *self = Self::from_value(current);
    }

    pub fn search_prefix(&self) -> &'static str {
        SEARCH_ENGINES
            .iter()
            .find(|(id, _)| *id == self.search_engine)
            .map(|(_, p)| *p)
            .unwrap_or(SEARCH_ENGINES[0].1)
    }

    /// Zoom for a page: its site's saved zoom, else the default (text size).
    pub fn zoom_for(&self, url: &str) -> f64 {
        zoom_site(url).and_then(|h| self.site_zoom.get(&h).copied()).filter(|z| z.is_finite() && *z > 0.0).unwrap_or(self.text_scale)
    }

    /// Remembers `zoom` for `url`'s site. Returns whether anything changed.
    pub fn remember_zoom(&mut self, url: &str, zoom: f64) -> bool {
        let Some(site) = zoom_site(url) else { return false };
        if (zoom - self.text_scale).abs() < 0.005 {
            self.site_zoom.remove(&site).is_some()
        } else {
            self.site_zoom.insert(site, zoom).map(|old| (old - zoom).abs() >= 0.005).unwrap_or(true)
        }
    }

    /// Hours after which an unused Today tab is archived; `None` = never.
    pub fn archive_after_hours(&self) -> Option<i64> {
        self.archive_interval_hours.as_i64()
    }

    #[allow(dead_code)]
    pub fn is_whitelisted(&self, host: &str) -> bool {
        let host = normalize_host(host);
        !host.is_empty()
            && self
                .shield_whitelist
                .iter()
                .any(|w| host == *w || host.ends_with(&format!(".{w}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn zoom_is_remembered_per_site() {
        let mut s = BrowserSettings::default();
        assert!(s.remember_zoom("https://www.youtube.com/watch?v=1", 1.25));
        assert_eq!(s.zoom_for("https://youtube.com/feed"), 1.25);
        assert_eq!(s.zoom_for("https://example.com/"), s.text_scale);
        // Back to the default forgets the site; non-web pages are never kept.
        assert!(s.remember_zoom("https://youtube.com/", s.text_scale));
        assert!(s.site_zoom.is_empty());
        assert!(!s.remember_zoom("file:///C:/a.pdf", 1.5));
        assert_eq!(zoom_site("https://WWW.Example.com:8080/x").as_deref(), Some("example.com"));
    }

    #[test]
    fn tab_sleep_minutes_defaults_and_validates() {
        assert_eq!(BrowserSettings::from_value(json!({})).tab_sleep_minutes, 60);
        assert_eq!(BrowserSettings::from_value(json!({ "tabSleepMinutes": 240 })).tab_sleep_minutes, 240);
        assert_eq!(BrowserSettings::from_value(json!({ "tabSleepMinutes": 7 })).tab_sleep_minutes, 60);
        assert_eq!(BrowserSettings::from_value(json!({ "tabSleepMinutes": "soon" })).tab_sleep_minutes, 60);
        let mut s = BrowserSettings::default();
        s.apply_patch(&json!({ "tabSleepMinutes": 15 }));
        assert_eq!(s.tab_sleep_minutes, 15);
        assert_eq!(serde_json::to_string(&s).unwrap().matches("tabSleepMinutes").count(), 1);
    }

    #[test]
    fn text_scale_snaps_to_offered_sizes() {
        assert_eq!(BrowserSettings::from_value(json!({})).text_scale, 1.0);
        assert_eq!(BrowserSettings::from_value(json!({ "textScale": 1.12 })).text_scale, 1.1);
        assert_eq!(BrowserSettings::from_value(json!({ "textScale": 9 })).text_scale, 1.25);
        assert_eq!(BrowserSettings::from_value(json!({ "textScale": "big" })).text_scale, 1.0);
        let mut s = BrowserSettings::default();
        s.apply_patch(&json!({ "textScale": 0.9 }));
        assert_eq!(s.text_scale, 0.9);
        assert!(s.extra.get("textScale").is_none());
    }

    #[test]
    fn bad_typed_key_does_not_reset_everything() {
        let s = BrowserSettings::from_value(json!({
            "adblockEnabled": "yes", "sidebarExpanded": false, "theme": "light"
        }));
        assert!(s.adblock_enabled);
        assert!(!s.sidebar_expanded);
        assert_eq!(s.extra.get("theme"), Some(&json!("light")));
    }

    #[test]
    fn sidebar_width_patch_is_typed_and_not_duplicated() {
        let mut s = BrowserSettings::default();
        s.apply_patch(&json!({ "sidebarWidth": 300 }));
        assert_eq!(s.sidebar_width, 300.0);
        let text = serde_json::to_string(&s).unwrap();
        assert_eq!(text.matches("sidebarWidth").count(), 1);
        s.apply_patch(&json!({ "sidebarWidth": 9000 }));
        assert_eq!(s.sidebar_width, 520.0);
    }

    #[test]
    fn whitelist_is_normalized() {
        let mut s = BrowserSettings::default();
        s.apply_patch(&json!({ "shieldWhitelist": ["https://www.Example.com/path", "example.com", ""] }));
        assert_eq!(s.shield_whitelist, vec!["example.com".to_string()]);
        assert!(s.is_whitelisted("sub.example.com"));
        assert!(!s.is_whitelisted("notexample.com"));
    }

    #[test]
    fn invalid_enum_values_fall_back() {
        let s = BrowserSettings::from_value(json!({ "searchEngine": "yahoo", "archiveIntervalHours": 5 }));
        assert_eq!(s.search_engine, "google");
        assert_eq!(s.archive_interval_hours, json!(12));
    }
}

/// The site a page's zoom is remembered for: its host without `www.`, for
/// web pages only.
pub fn zoom_site(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    let host = u.host_str()?.to_ascii_lowercase();
    Some(host.strip_prefix("www.").unwrap_or(&host).to_string())
}
