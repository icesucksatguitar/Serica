//! Content blocking powered by Brave's `adblock` engine (adblock-rust).
//!
//! * Network: every subresource request of a tab webview is checked with
//!   `Engine::check_network_request` (see `webview_win.rs`), including
//!   `$redirect` neutered resources and `$important` rules.
//! * Cosmetic: per-URL hide selectors and uBO/Brave scriptlets come from
//!   `url_cosmetic_resources`; generic class/id hiding uses
//!   `hidden_class_id_selectors` with the classes/ids the page reports.
//!
//! Lists: a compiled-in fallback list is active immediately at startup; the
//! full lists (EasyList, EasyPrivacy, uBlock filters/privacy/unbreak/quick-fixes)
//! plus Brave's scriptlet resources are downloaded in the background, cached on
//! disk and refreshed every few days. A serialized engine makes later startups
//! fast.

use crate::model::{now_ms, ShieldStats};
use adblock::lists::{FilterSet, ParseOptions};
use adblock::resources::{PermissionMask, Resource};
use adblock::Engine;
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const REFRESH_AFTER_MS: i64 = 4 * 24 * 3_600_000;

struct ListSource {
    name: &'static str,
    url: &'static str,
    /// uBO lists may use trusted scriptlets.
    trusted: bool,
}

/// Brave's default desktop filter set, from brave/adblock-resources
/// `filter_lists/list_catalog.json` (default-enabled lists): "Brave Default
/// Adblock Filters", "Brave Default Privacy Filters", "Brave First Party
/// Adblock Filters" and "Cookie notice blocker". Android/iOS-only lists are left
/// out. uBlock Origin and Brave lists may use trusted scriptlets.
const LISTS: &[ListSource] = &[
    ListSource { name: "ublock-filters", url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/filters.txt", trusted: true },
    ListSource { name: "ublock-filters-2020", url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/filters-2020.txt", trusted: true },
    ListSource { name: "ublock-filters-2021", url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/filters-2021.txt", trusted: true },
    ListSource { name: "ublock-filters-2022", url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/filters-2022.txt", trusted: true },
    ListSource { name: "ublock-filters-2023", url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/filters-2023.txt", trusted: true },
    ListSource { name: "ublock-filters-2024", url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/filters-2024.txt", trusted: true },
    ListSource { name: "ublock-filters-2025", url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/filters-2025.txt", trusted: true },
    ListSource { name: "ublock-filters-2026", url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/filters-2026.txt", trusted: true },
    ListSource { name: "ublock-filters-general", url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/filters-general.txt", trusted: true },
    ListSource { name: "ublock-badware", url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/badware.txt", trusted: true },
    ListSource { name: "ublock-resource-abuse", url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/resource-abuse.txt", trusted: true },
    ListSource { name: "ublock-unbreak", url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/unbreak.txt", trusted: true },
    ListSource { name: "ublock-quick-fixes", url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/quick-fixes.txt", trusted: true },
    ListSource { name: "ublock-link-shorteners", url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/ubo-link-shorteners.txt", trusted: true },
    ListSource { name: "easylist", url: "https://easylist.to/easylist/easylist.txt", trusted: false },
    ListSource { name: "urlhaus", url: "https://malware-filter.gitlab.io/malware-filter/urlhaus-filter-agh-online.txt", trusted: false },
    ListSource { name: "brave-unbreak", url: "https://raw.githubusercontent.com/brave/adblock-lists/master/brave-unbreak.txt", trusted: true },
    ListSource { name: "brave-specific", url: "https://raw.githubusercontent.com/brave/adblock-lists/master/brave-lists/brave-specific.txt", trusted: true },
    ListSource { name: "brave-social", url: "https://raw.githubusercontent.com/brave/adblock-lists/master/brave-lists/brave-social.txt", trusted: true },
    ListSource { name: "brave-lists-unbreak", url: "https://raw.githubusercontent.com/brave/adblock-lists/master/brave-lists/brave-unbreak.txt", trusted: true },
    ListSource { name: "brave-sugarcoat", url: "https://raw.githubusercontent.com/brave/adblock-lists/master/brave-lists/brave-sugarcoat.txt", trusted: true },
    ListSource { name: "easyprivacy", url: "https://easylist.to/easylist/easyprivacy.txt", trusted: false },
    ListSource { name: "ublock-privacy", url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/privacy.txt", trusted: true },
    ListSource { name: "brave-firstparty", url: "https://raw.githubusercontent.com/brave/adblock-lists/master/brave-lists/brave-firstparty.txt", trusted: true },
    ListSource { name: "brave-firstparty-regional", url: "https://raw.githubusercontent.com/brave/adblock-lists/master/brave-lists/brave-firstparty-regional.txt", trusted: true },
    ListSource { name: "fanboy-cookiemonster", url: "https://secure.fanboy.co.nz/fanboy-cookiemonster_ubo.txt", trusted: false },
    ListSource { name: "ublock-annoyances-cookies", url: "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/annoyances-cookies.txt", trusted: true },
    ListSource { name: "brave-cookie-specific", url: "https://raw.githubusercontent.com/brave/adblock-lists/master/brave-lists/brave-cookie-specific.txt", trusted: true },
];

const RESOURCES_URL: &str = "https://raw.githubusercontent.com/brave/adblock-resources/master/dist/resources.json";
/// uBlock Origin's scriptlets (set-constant, json-prune, trusted-*, …) converted
/// by `rust/tools/build-ubo-scriptlets.mjs`. Brave's file above only holds
/// Brave's extras, and most uBO rules (e.g. YouTube's) need these.
const UBO_RESOURCES_URL: &str = "https://raw.githubusercontent.com/icesucksatguitar/neutrino/main/rust/resources/ubo-scriptlets.json";

/// Always-available rules used until the downloaded lists are ready (and if
/// the machine is offline on first start).
const BUILTIN_RULES: &str = r#"
||doubleclick.net^
||googleadservices.com^
||googlesyndication.com^
||adservice.google.com^
||pagead2.googlesyndication.com^
||adnxs.com^
||criteo.com^
||criteo.net^
||rubiconproject.com^
||pubmatic.com^
||openx.net^
||amazon-adsystem.com^
||adroll.com^
||advertising.com^
||taboola.com^
||outbrain.com^
||popads.net^
||adcolony.com^
||moatads.com^
||casalemedia.com^
||smartadserver.com^
||google-analytics.com^
||analytics.google.com^
||googletagmanager.com^
||googletagservices.com^
||connect.facebook.net^$third-party
||pixel.facebook.com^
||tr.snapchat.com^
||analytics.tiktok.com^
||ads.tiktok.com^
||scorecardresearch.com^
||quantserve.com^
||hotjar.com^
||mouseflow.com^
||segment.io^
||cdn.segment.com^
||mixpanel.com^
||amplitude.com^
||fullstory.com^
||clarity.ms^
||bat.bing.com^
||fingerprintjs.com^
||fpjs.io^
##.adsbygoogle
##.ad-banner
##.advertisement
##[id^="google_ads_iframe"]
##.trc_related_container
youtube.com##ytd-ad-slot-renderer
youtube.com##ytd-in-feed-ad-layout-renderer
youtube.com##ytd-banner-promo-renderer
youtube.com###player-ads
youtube.com##.ytp-ad-module
youtube.com##ytd-promoted-sparkles-web-renderer
youtube.com##ytd-display-ad-renderer
"#;

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ShieldsInfo {
    /// "builtin" until the downloaded lists are compiled, then "full".
    pub engine: String,
    pub lists: Vec<String>,
    pub last_updated: Option<i64>,
    pub updating: bool,
    pub last_error: Option<String>,
}

pub struct Decision {
    pub redirect: Option<String>,
    pub rule: Option<String>,
    pub category: &'static str,
}

pub struct Shields {
    engine: RwLock<Arc<Engine>>,
    enabled: AtomicBool,
    whitelist: RwLock<Vec<String>>,
    stats: Mutex<ShieldStats>,
    tab_counts: Mutex<HashMap<String, i64>>,
    pub info: Mutex<ShieldsInfo>,
    dir: PathBuf,
}

fn parse_opts(trusted: bool) -> ParseOptions {
    ParseOptions {
        permissions: if trusted { PermissionMask::from_bits(0b1) } else { PermissionMask::default() },
        ..Default::default()
    }
}

fn builtin_engine() -> Engine {
    let mut set = FilterSet::new(false);
    set.add_filter_list(BUILTIN_RULES.to_string(), parse_opts(false));
    Engine::new_with_filter_set(set)
}

impl Shields {
    pub fn new(dir: PathBuf, stats: ShieldStats) -> Arc<Self> {
        let _ = std::fs::create_dir_all(&dir);
        let s = Arc::new(Self {
            engine: RwLock::new(Arc::new(builtin_engine())),
            enabled: AtomicBool::new(true),
            whitelist: RwLock::new(Vec::new()),
            stats: Mutex::new(stats),
            tab_counts: Mutex::new(HashMap::new()),
            info: Mutex::new(ShieldsInfo { engine: "builtin".into(), ..Default::default() }),
            dir,
        });
        s.load_cached();
        s
    }

    pub fn configure(&self, enabled: bool, whitelist: &[String]) {
        self.enabled.store(enabled, Ordering::Relaxed);
        *self.whitelist.write() = whitelist.to_vec();
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// True when blocking applies to pages on `page_url`'s host.
    pub fn active_for(&self, page_url: &str) -> bool {
        if !self.enabled() {
            return false;
        }
        let host = url::Url::parse(page_url).ok().and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_lowercase()));
        match host {
            Some(h) => !self.whitelist.read().iter().any(|w| h == *w || h.ends_with(&format!(".{w}"))),
            None => true,
        }
    }

    /// Checks one network request. `request_type` uses adblock-rust names
    /// ("script", "image", "sub_frame", "xhr", ...).
    pub fn check(&self, url: &str, source_url: &str, request_type: &str) -> Option<Decision> {
        if !(url.starts_with("http") || url.starts_with("ws")) || !self.active_for(source_url) {
            return None;
        }
        let req = adblock::request::Request::new(url, source_url, request_type, "GET").ok()?;
        let engine = self.engine.read().clone();
        let res = engine.check_network_request(&req);
        if !res.should_block() {
            return None;
        }
        Some(Decision {
            redirect: res.redirect,
            rule: res.filter.and_then(|f| f.raw_line),
            category: categorize(url),
        })
    }

    pub fn cosmetic(&self, url: &str) -> Option<adblock::cosmetic_filter_cache::UrlSpecificResources> {
        if !self.active_for(url) {
            return None;
        }
        Some(self.engine.read().clone().url_cosmetic_resources(url))
    }

    pub fn class_id_selectors(&self, classes: &[String], ids: &[String], exceptions: &HashSet<String>) -> Vec<String> {
        self.engine.read().clone().hidden_class_id_selectors(classes, ids, exceptions)
    }

    /// Records a block for stats and returns the event payload for the UI.
    pub fn record_block(&self, tab_id: &str, url: &str, d: &Decision, request_type: &str) -> crate::model::BlockedTrackerItem {
        {
            let mut s = self.stats.lock();
            s.total_blocked += 1;
            match d.category {
                "Advertising" => s.ads_blocked += 1,
                "Analytics" => s.analytics_blocked += 1,
                "Fingerprinting" => s.fingerprinters_blocked += 1,
                _ => s.trackers_blocked += 1,
            }
            s.data_saved_kb += estimated_kb(request_type);
        }
        *self.tab_counts.lock().entry(tab_id.to_string()).or_insert(0) += 1;
        crate::model::BlockedTrackerItem {
            url: url.chars().take(512).collect(),
            domain: url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default(),
            category: d.category.to_string(),
            rule: d.rule.clone(),
            timestamp: now_ms(),
            tab_id: Some(tab_id.to_string()),
        }
    }

    pub fn stats(&self) -> ShieldStats {
        self.stats.lock().clone()
    }

    pub fn reset_stats(&self) -> ShieldStats {
        *self.stats.lock() = ShieldStats::default();
        self.tab_counts.lock().clear();
        ShieldStats::default()
    }

    pub fn tab_count(&self, tab_id: &str) -> i64 {
        self.tab_counts.lock().get(tab_id).copied().unwrap_or(0)
    }

    /// Per-tab counters restart on each top-level navigation.
    pub fn reset_tab(&self, tab_id: &str) {
        self.tab_counts.lock().remove(tab_id);
    }

    // ------------------------------------------------------------------ lists

    /// Brave's resources plus uBlock Origin's scriptlets, de-duplicated by name.
    fn resources(&self) -> Option<Vec<Resource>> {
        let read = |file: &str| -> Vec<Resource> {
            std::fs::read_to_string(self.dir.join(file)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
        };
        let mut seen = std::collections::HashSet::new();
        let mut out: Vec<Resource> = Vec::new();
        for r in read("ubo-resources.json").into_iter().chain(read("resources.json")) {
            if seen.insert(r.name.clone()) {
                out.push(r);
            }
        }
        (!out.is_empty()).then_some(out)
    }

    fn load_cached(&self) {
        let Ok(bytes) = std::fs::read(self.dir.join("engine.dat")) else { return };
        let mut engine = Engine::default();
        if engine.deserialize(&bytes).is_err() {
            tracing::warn!("cached adblock engine is stale; will rebuild");
            return;
        }
        if let Some(r) = self.resources() {
            engine.use_resources(r);
        }
        *self.engine.write() = Arc::new(engine);
        let meta = std::fs::read_to_string(self.dir.join("meta.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
        let mut info = self.info.lock();
        info.engine = "full".into();
        info.last_updated = meta.as_ref().and_then(|m| m["updatedAt"].as_i64());
        info.lists = LISTS.iter().map(|l| l.name.to_string()).collect();
        tracing::info!("loaded cached adblock engine");
    }

    fn needs_refresh(&self) -> bool {
        // A list added to the catalog, or scriptlets never fetched, can't wait for the next refresh.
        let lists_dir = self.dir.join("lists");
        let incomplete = LISTS.iter().any(|l| !lists_dir.join(format!("{}.txt", l.name)).exists())
            || !self.dir.join("ubo-resources.json").exists();
        let info = self.info.lock();
        incomplete || info.engine != "full" || info.last_updated.map(|t| now_ms() - t > REFRESH_AFTER_MS).unwrap_or(true)
    }

    /// Downloads lists and rebuilds the engine on a background thread.
    pub fn spawn_updater(self: &Arc<Self>, force: bool) {
        let this = self.clone();
        if !force && !this.needs_refresh() {
            return;
        }
        {
            let mut info = this.info.lock();
            if info.updating {
                return;
            }
            info.updating = true;
        }
        tauri::async_runtime::spawn(async move {
            let result = this.update_lists().await;
            let mut info = this.info.lock();
            info.updating = false;
            match result {
                Ok(()) => info.last_error = None,
                Err(e) => {
                    tracing::warn!("adblock list update failed: {e}");
                    info.last_error = Some(e);
                }
            }
        });
    }

    async fn update_lists(self: &Arc<Self>) -> Result<(), String> {
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Serica/0.3")
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| e.to_string())?;
        let lists_dir = self.dir.join("lists");
        let _ = std::fs::create_dir_all(&lists_dir);
        let mut texts = Vec::new();
        for l in LISTS {
            let path = lists_dir.join(format!("{}.txt", l.name));
            let fetched = match client.get(l.url).send().await.and_then(|r| r.error_for_status()) {
                Ok(r) => r.text().await.ok(),
                Err(e) => {
                    tracing::warn!("fetching {} failed: {e}", l.name);
                    None
                }
            };
            let text = match fetched.filter(|t| !t.trim().is_empty() && !t.trim_start().starts_with('<')) {
                Some(t) => {
                    let _ = crate::store::write_atomic(&path, t.as_bytes());
                    t
                }
                // Offline: fall back to the copy from the last successful update.
                None => match std::fs::read_to_string(&path) {
                    Ok(t) => t,
                    Err(_) => continue,
                },
            };
            texts.push((text, l.trusted));
        }
        if texts.is_empty() {
            return Err("no filter lists could be downloaded".into());
        }
        for (url, file) in [(RESOURCES_URL, "resources.json"), (UBO_RESOURCES_URL, "ubo-resources.json")] {
            if let Ok(r) = client.get(url).send().await.and_then(|r| r.error_for_status()) {
                if let Ok(body) = r.text().await {
                    if serde_json::from_str::<Vec<Resource>>(&body).map(|v| !v.is_empty()).unwrap_or(false) {
                        let _ = crate::store::write_atomic(&self.dir.join(file), body.as_bytes());
                    }
                }
            }
        }
        let resources = self.resources();
        let dir = self.dir.clone();
        let engine = tauri::async_runtime::spawn_blocking(move || {
            let mut set = FilterSet::new(false);
            set.add_filter_list(BUILTIN_RULES.to_string(), parse_opts(false));
            for (text, trusted) in texts {
                set.add_filter_list(text, parse_opts(trusted));
            }
            let mut engine = Engine::new_with_filter_set(set);
            let _ = crate::store::write_atomic(&dir.join("engine.dat"), &engine.serialize());
            if let Some(r) = resources {
                engine.use_resources(r);
            }
            engine
        })
        .await
        .map_err(|e| e.to_string())?;
        *self.engine.write() = Arc::new(engine);
        let now = now_ms();
        let _ = crate::store::write_atomic(&self.dir.join("meta.json"), serde_json::json!({ "updatedAt": now }).to_string().as_bytes());
        let mut info = self.info.lock();
        info.engine = "full".into();
        info.last_updated = Some(now);
        info.lists = LISTS.iter().map(|l| l.name.to_string()).collect();
        tracing::info!("adblock engine rebuilt from downloaded lists");
        Ok(())
    }
}

/// Coarse category for the Shields panel, from the blocked URL.
pub fn categorize(url: &str) -> &'static str {
    let u = url.to_lowercase();
    const FINGERPRINT: &[&str] = &["fingerprint", "fpjs", "fpcdn", "deviceid", "clientjs", "/fp.js", "iovation", "threatmetrix"];
    const ANALYTICS: &[&str] = &[
        "analytics", "google-analytics", "googletagmanager", "metrics", "telemetry", "segment.", "mixpanel", "amplitude",
        "hotjar", "mouseflow", "fullstory", "clarity.ms", "scorecardresearch", "quantserve", "/collect", "stats.", "newrelic", "nr-data",
    ];
    const TRACKING: &[&str] = &["pixel", "track", "beacon", "facebook.net", "facebook.com/tr", "snapchat", "tiktok", "bat.bing", "/event", "linkedin.com/px"];
    if FINGERPRINT.iter().any(|k| u.contains(k)) {
        "Fingerprinting"
    } else if ANALYTICS.iter().any(|k| u.contains(k)) {
        "Analytics"
    } else if TRACKING.iter().any(|k| u.contains(k)) {
        "Tracking"
    } else {
        "Advertising"
    }
}

fn estimated_kb(request_type: &str) -> i64 {
    match request_type {
        "script" => 40,
        "sub_frame" => 80,
        "image" => 18,
        "media" => 200,
        "stylesheet" => 12,
        "font" => 30,
        _ => 4,
    }
}

/// Builds a CSS block hiding `selectors` (chunked so one invalid selector only
/// disables its own chunk).
pub fn hide_css(selectors: impl IntoIterator<Item = String>) -> String {
    let list: Vec<String> = selectors.into_iter().filter(|s| !s.contains('{') && !s.contains('}')).collect();
    list.chunks(64)
        .map(|c| format!("{}{{display:none !important;}}", c.join(",")))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shields() -> Arc<Shields> {
        Shields::new(std::env::temp_dir().join(format!("shields-{}", uuid::Uuid::new_v4())), ShieldStats::default())
    }

    #[test]
    fn blocks_known_ad_domain_with_builtin_rules() {
        let s = shields();
        let d = s.check("https://securepubads.g.doubleclick.net/tag/js/gpt.js", "https://news.example.com/", "script");
        assert!(d.is_some());
        assert!(s.check("https://news.example.com/app.js", "https://news.example.com/", "script").is_none());
    }

    #[test]
    fn whitelist_and_toggle_disable_blocking() {
        let s = shields();
        let ad = "https://www.google-analytics.com/analytics.js";
        s.configure(true, &["example.com".into()]);
        assert!(s.check(ad, "https://shop.example.com/", "script").is_none());
        assert!(s.check(ad, "https://other.org/", "script").is_some());
        s.configure(false, &[]);
        assert!(s.check(ad, "https://other.org/", "script").is_none());
    }

    #[test]
    fn stats_categories() {
        let s = shields();
        let d = s.check("https://www.google-analytics.com/analytics.js", "https://a.org/", "script").unwrap();
        assert_eq!(d.category, "Analytics");
        s.record_block("t1", "https://www.google-analytics.com/analytics.js", &d, "script");
        assert_eq!(s.stats().analytics_blocked, 1);
        assert_eq!(s.tab_count("t1"), 1);
        s.reset_stats();
        assert_eq!(s.stats().total_blocked, 0);
    }

    #[test]
    fn cosmetic_selectors_for_youtube() {
        let s = shields();
        let r = s.cosmetic("https://www.youtube.com/watch?v=x").unwrap();
        assert!(r.hide_selectors.contains("ytd-ad-slot-renderer"));
        let generic = s.class_id_selectors(&["adsbygoogle".into()], &[], &HashSet::new());
        assert!(generic.iter().any(|g| g.contains("adsbygoogle")));
    }
}

#[cfg(test)]
mod youtube_diag {
    /// `SERICA_ADBLOCK_DIR=<data>/adblock cargo test youtube_scriptlets -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn youtube_scriptlets() {
        let dir = std::path::PathBuf::from(std::env::var("SERICA_ADBLOCK_DIR").unwrap());
        let read = |f: &str| -> Vec<serde_json::Value> {
            std::fs::read_to_string(dir.join(f)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
        };
        let mut names = std::collections::HashSet::new();
        for r in read("resources.json").iter().chain(read("ubo-resources.json").iter()) {
            names.insert(r["name"].as_str().unwrap_or("").trim_end_matches(".js").to_string());
            for a in r["aliases"].as_array().cloned().unwrap_or_default() {
                names.insert(a.as_str().unwrap_or("").trim_end_matches(".js").to_string());
            }
        }
        let mut missing = std::collections::BTreeMap::<String, usize>::new();
        let mut total = 0;
        for f in std::fs::read_dir(dir.join("lists")).unwrap() {
            let text = std::fs::read_to_string(f.unwrap().path()).unwrap();
            for line in text.lines().filter(|l| l.contains("youtube.com") && l.contains("##+js(")) {
                total += 1;
                let name = line.split("##+js(").nth(1).unwrap().split([',', ')']).next().unwrap().trim().trim_end_matches(".js").to_string();
                if !names.contains(&name) {
                    *missing.entry(name).or_default() += 1;
                }
            }
        }
        println!("youtube +js rules: {total}; scriptlet names missing from resources: {missing:?}");

        // Build the real engine the way Shields does and inspect what it injects.
        let mut set = adblock::lists::FilterSet::new(false);
        for l in super::LISTS {
            if let Ok(text) = std::fs::read_to_string(dir.join("lists").join(format!("{}.txt", l.name))) {
                set.add_filter_list(text, super::parse_opts(l.trusted));
            }
        }
        let mut engine = adblock::Engine::new_with_filter_set(set);
        let shields = super::Shields::new(dir.clone(), Default::default());
        engine.use_resources(shields.resources().unwrap_or_default());
        let script = engine.url_cosmetic_resources("https://www.youtube.com/watch?v=x").injected_script;
        let invoked = script.matches("try {").count();
        println!("youtube injected script: {} bytes, {invoked} scriptlet invocations", script.len());
        for f in ["function jsonPrune", "function setConstant", "function trustedReplaceXhrResponse", "class JSONPath"] {
            println!("  contains {f:?}: {}", script.contains(f));
        }
    }
}
