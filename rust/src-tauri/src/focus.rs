//! Focus Mode: blocks websites for a while, Cold Turkey style.
//!
//! - Block lists hold rules: exact domains (`reddit.com`, which also covers
//!   `www.`), whole domains with their subdomains (`*.reddit.com`), paths
//!   (`youtube.com/shorts`), globs (`*casino*`), category presets, and
//!   exceptions that are always allowed (`reddit.com/r/rust`).
//! - A list is in effect during a session (a countdown, or until stopped) or
//!   inside its weekly schedule.
//! - Locked lists cannot be stopped, loosened, disabled or deleted until the
//!   time is up (they can always be made stricter). A list may instead allow
//!   an early end by typing a long random text.
//!
//! Enforcement happens in every page webview's request pipeline (see
//! `webview_win`): a blocked page is answered with Serica's block page before
//! anything is sent to the site, in every profile, after restarts too.
//!
//! Time can't be cheated by moving the clock: session time is counted with
//! the monotonic clock while Serica runs, and time while Serica was closed
//! only counts when a network time source vouches for it. Schedules use a
//! clock that is set once (from the network when possible) and then advanced
//! monotonically, so changing the system time while Serica runs does nothing.

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

// ------------------------------------------------------------------ data

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Unlock {
    /// Nothing ends a locked list early.
    #[default]
    Strict,
    /// Typing a long random text ends it early.
    RandomText,
}

/// A weekly window: `days` is a bit mask (bit 0 = Sunday … bit 6 = Saturday),
/// `start`/`end` are minutes after midnight; `end <= start` runs past midnight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScheduleSlot {
    pub days: u8,
    pub start: u16,
    pub end: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FocusBlock {
    pub id: String,
    pub name: String,
    pub sites: Vec<String>,
    pub allow: Vec<String>,
    pub categories: Vec<String>,
    pub schedule: Vec<ScheduleSlot>,
    pub schedule_on: bool,
    /// The list is locked whenever its schedule is in effect.
    pub schedule_locked: bool,
    pub unlock: Unlock,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub block_id: String,
    /// Time left; `None` runs until stopped.
    pub remaining_ms: Option<i64>,
    pub total_ms: Option<i64>,
    pub locked: bool,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Saved {
    blocks: Vec<FocusBlock>,
    sessions: Vec<Session>,
    /// Trusted time (ms since the epoch) when this was written.
    saved_at_ms: i64,
    /// `saved_at_ms` came from a network-verified clock.
    saved_at_verified: bool,
    /// Lists whose schedule was ended early (random text), until this time.
    suspended_until: BTreeMap<String, i64>,
}

pub struct Category {
    pub id: &'static str,
    pub label: &'static str,
    pub sites: &'static [&'static str],
}

pub const CATEGORIES: &[Category] = &[
    Category {
        id: "social",
        label: "Social media",
        sites: &[
            "*.facebook.com", "*.instagram.com", "*.x.com", "*.twitter.com", "*.tiktok.com", "*.reddit.com",
            "*.snapchat.com", "*.pinterest.com", "*.tumblr.com", "*.threads.net", "*.threads.com", "*.bsky.app",
            "*.linkedin.com", "*.mastodon.social", "*.discord.com", "*.quora.com",
        ],
    },
    Category {
        id: "video",
        label: "Video and streaming",
        sites: &[
            "*.youtube.com", "*.twitch.tv", "*.netflix.com", "*.primevideo.com", "*.disneyplus.com", "*.hulu.com",
            "*.max.com", "*.vimeo.com", "*.dailymotion.com", "*.crunchyroll.com", "*.hotstar.com", "*.kick.com",
        ],
    },
    Category {
        id: "news",
        label: "News",
        sites: &[
            "*.cnn.com", "*.foxnews.com", "*.nytimes.com", "*.bbc.com", "*.bbc.co.uk", "*.theguardian.com",
            "*.washingtonpost.com", "*.reuters.com", "*.apnews.com", "news.google.com", "news.ycombinator.com",
            "*.buzzfeed.com", "*.huffpost.com", "*.nbcnews.com", "*.ndtv.com", "*.indiatimes.com", "*.aljazeera.com",
        ],
    },
    Category {
        id: "shopping",
        label: "Shopping",
        sites: &[
            "*.amazon.com", "*.amazon.in", "*.amazon.co.uk", "*.amazon.de", "*.ebay.com", "*.etsy.com",
            "*.aliexpress.com", "*.temu.com", "*.walmart.com", "*.target.com", "*.flipkart.com", "*.shein.com",
            "*.bestbuy.com", "*.myntra.com",
        ],
    },
    Category {
        id: "games",
        label: "Games",
        sites: &[
            "store.steampowered.com", "*.steamcommunity.com", "*.epicgames.com", "*.roblox.com", "*.miniclip.com",
            "*.chess.com", "*.lichess.org", "*.poki.com", "*.crazygames.com", "*.itch.io", "*.coolmathgames.com",
        ],
    },
];

// ------------------------------------------------------------------ rules

#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    /// A host, optionally with its subdomains, optionally only under a path.
    Host { host: String, subdomains: bool, path: Option<String> },
    /// `*` wildcards over "host/path".
    Glob(String),
}

/// Parses a rule as a person types it: scheme and `www.` are ignored, case
/// doesn't matter. Returns `None` for anything that can't be a site.
pub fn parse_rule(input: &str) -> Option<Rule> {
    let mut s = input.trim().to_ascii_lowercase();
    for scheme in ["https://", "http://"] {
        if let Some(rest) = s.strip_prefix(scheme) {
            s = rest.to_string();
        }
    }
    let s = s.trim_end_matches('/');
    if s.is_empty() || s.len() > 300 || s.contains(char::is_whitespace) {
        return None;
    }
    let (host_part, path) = match s.find('/') {
        Some(i) => (&s[..i], Some(s[i..].to_string())),
        None => (s, None),
    };
    let (host_part, subdomains) = match host_part.strip_prefix("*.") {
        Some(h) => (h, true),
        None => (host_part, false),
    };
    let host_ok = !host_part.is_empty()
        && host_part.contains('.')
        && host_part.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && !host_part.starts_with('.')
        && !host_part.ends_with('.');
    if host_ok && !path.as_deref().unwrap_or("").contains('*') {
        let host = host_part.strip_prefix("www.").unwrap_or(host_part).to_string();
        return Some(Rule::Host { host, subdomains, path });
    }
    // Anything else with a wildcard is matched as a pattern.
    if s.contains('*') && s.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Some(Rule::Glob(s.to_string()));
    }
    None
}

/// `*` matches any run of characters (including none).
fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut pos = 0usize;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !text.starts_with(part) {
                return false;
            }
            pos = part.len();
        } else if i == parts.len() - 1 {
            return text.len() >= pos + part.len() && text[pos..].ends_with(part);
        } else {
            match text[pos..].find(part) {
                Some(j) => pos += j + part.len(),
                None => return false,
            }
        }
    }
    true
}

impl Rule {
    /// `host` lowercase without `www.`; `path` starts with `/`.
    fn matches(&self, host: &str, path: &str) -> bool {
        match self {
            Rule::Host { host: h, subdomains, path: p } => {
                let host_hit = host == h || (*subdomains && host.ends_with(&format!(".{h}")));
                if !host_hit {
                    return false;
                }
                match p {
                    None => true,
                    // A path rule covers the path and what is under it.
                    Some(p) => path == p || path.starts_with(&format!("{p}/")) || path.starts_with(&format!("{p}?")) || path.starts_with(&format!("{p}#")),
                }
            }
            Rule::Glob(g) => glob_match(g, &format!("{host}{path}")) || glob_match(g, host),
        }
    }

    /// Whether the rule covers a whole site (not just some pages): such rules
    /// also stop the site's requests made from other pages.
    fn whole_site(&self) -> bool {
        matches!(self, Rule::Host { path: None, .. })
    }
}

/// Host (lowercase, no `www.`) and path of a web URL; `None` for anything
/// that isn't http(s), so Serica's own pages are never blocked.
fn split_url(url: &str) -> Option<(String, String)> {
    let u = url::Url::parse(url).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    let host = u.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let mut path = u.path().to_ascii_lowercase();
    if let Some(q) = u.query() {
        path.push('?');
        path.push_str(&q.to_ascii_lowercase());
    }
    Some((host, path))
}

struct ActiveRules {
    block_id: String,
    block_name: String,
    /// Trusted time this list stops, if known.
    until_ms: Option<i64>,
    locked: bool,
    rules: Vec<Rule>,
    allow: Vec<Rule>,
}

/// What a blocked page is told.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub block_id: String,
    pub block_name: String,
    /// Time left in ms, if the list has an end.
    pub remaining_ms: Option<i64>,
    pub locked: bool,
    /// The rule covers the whole site (other pages' requests to it are stopped too).
    pub whole_site: bool,
}

pub struct Compiled {
    lists: Vec<ActiveRules>,
    /// Trusted time when compiled, with the monotonic instant (to age `until`).
    at_ms: i64,
    at: Instant,
}

impl Compiled {
    pub fn check(&self, url: &str) -> Option<Hit> {
        let (host, path) = split_url(url)?;
        let now = self.at_ms + self.at.elapsed().as_millis() as i64;
        let mut best: Option<Hit> = None;
        for l in &self.lists {
            if l.allow.iter().any(|r| r.matches(&host, &path)) {
                continue;
            }
            let Some(rule) = l.rules.iter().find(|r| r.matches(&host, &path)) else { continue };
            let remaining = l.until_ms.map(|u| (u - now).max(0));
            let hit = Hit { block_id: l.block_id.clone(), block_name: l.block_name.clone(), remaining_ms: remaining, locked: l.locked, whole_site: rule.whole_site() };
            // The longest-lasting block wins (no end beats any end).
            let better = match &best {
                None => true,
                Some(b) => match (b.remaining_ms, hit.remaining_ms) {
                    (Some(_), None) => true,
                    (Some(a), Some(c)) => c > a,
                    _ => false,
                },
            };
            if better {
                best = Some(hit);
            }
        }
        best
    }
}

// ------------------------------------------------------------------ clock

/// Wall time that can't be moved by changing the system clock while Serica
/// runs: set once, then advanced with the monotonic clock.
#[derive(Clone, Copy)]
struct TrustedClock {
    base_ms: i64,
    base_at: Instant,
    verified: bool,
}

impl TrustedClock {
    fn now_ms(&self) -> i64 {
        self.base_ms + self.base_at.elapsed().as_millis() as i64
    }
}

fn system_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Minutes after local midnight and weekday (0 = Sunday) at trusted time `ms`.
fn local_parts(ms: i64) -> (u16, u8, i64) {
    use chrono::{Datelike, TimeZone, Timelike};
    let t = chrono::Local.timestamp_millis_opt(ms).single().unwrap_or_else(chrono::Local::now);
    let minutes = (t.hour() * 60 + t.minute()) as u16;
    let ms_into_minute = (t.second() as i64) * 1000 + (t.timestamp_subsec_millis() as i64);
    (minutes, t.weekday().num_days_from_sunday() as u8, ms_into_minute)
}

/// If `slot` is in effect at local (weekday, minute), the ms until it ends.
fn slot_remaining(slot: &ScheduleSlot, weekday: u8, minute: u16, ms_into_minute: i64) -> Option<i64> {
    let day_bit = |d: u8| slot.days & (1 << (d % 7)) != 0;
    let to_ms = |mins: i64| mins * 60_000 - ms_into_minute;
    if slot.start == slot.end {
        return None;
    }
    if slot.start < slot.end {
        (day_bit(weekday) && minute >= slot.start && minute < slot.end).then(|| to_ms((slot.end - minute) as i64))
    } else {
        // Past midnight: the evening part belongs to today, the morning part to yesterday's slot.
        if day_bit(weekday) && minute >= slot.start {
            return Some(to_ms((1440 - minute as i64) + slot.end as i64));
        }
        let yesterday = (weekday + 6) % 7;
        (day_bit(yesterday) && minute < slot.end).then(|| to_ms((slot.end - minute) as i64))
    }
}

/// Time from a network source (HTTP `Date` headers), corrected for half the
/// round trip. `None` when offline.
pub async fn network_time_ms() -> Option<i64> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(6))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .build()
        .ok()?;
    for url in ["https://www.google.com/generate_204", "https://www.cloudflare.com/cdn-cgi/trace", "https://www.microsoft.com/"] {
        let sent = Instant::now();
        let Ok(resp) = client.head(url).send().await else { continue };
        let rtt = sent.elapsed().as_millis() as i64;
        let Some(date) = resp.headers().get(reqwest::header::DATE).and_then(|v| v.to_str().ok()) else { continue };
        let Ok(t) = chrono::DateTime::parse_from_rfc2822(date) else { continue };
        // Date has whole seconds: aim at the middle of that second.
        let server = t.timestamp_millis() + 500;
        return Some(server + rtt / 2);
    }
    None
}

// ------------------------------------------------------------------ state

struct Inner {
    blocks: Vec<FocusBlock>,
    sessions: Vec<Session>,
    suspended_until: BTreeMap<String, i64>,
    clock: TrustedClock,
    last_tick: Instant,
    /// Random texts handed out for early unlocks, by block id.
    challenges: BTreeMap<String, String>,
    /// Active (block id, locked, source) last compiled, to see changes.
    signature: Vec<(String, bool, &'static str)>,
    dirty: bool,
    /// Time since Serica was closed hasn't been settled yet (no network answer).
    downtime_pending: Option<(i64, bool)>,
}

pub struct Focus {
    path: PathBuf,
    inner: Mutex<Inner>,
    active: RwLock<Option<Arc<Compiled>>>,
    any: AtomicBool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveView {
    pub block_id: String,
    /// "session" | "schedule"
    pub source: &'static str,
    pub remaining_ms: Option<i64>,
    pub total_ms: Option<i64>,
    pub locked: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryView {
    pub id: &'static str,
    pub label: &'static str,
    pub sites: Vec<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusView {
    pub blocks: Vec<FocusBlock>,
    pub active: Vec<ActiveView>,
    pub categories: Vec<CategoryView>,
    /// The clock was checked against the network.
    pub clock_verified: bool,
}

fn new_id() -> String {
    format!("focus-{}", &uuid::Uuid::new_v4().simple().to_string()[..12])
}

fn clean_rules(list: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    list.iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| parse_rule(s).is_some() && seen.insert(s.clone()))
        .take(2000)
        .collect()
}

fn normalize_block(mut b: FocusBlock) -> FocusBlock {
    b.name = b.name.trim().chars().take(60).collect();
    if b.name.is_empty() {
        b.name = "Focus".into();
    }
    b.sites = clean_rules(&b.sites);
    b.allow = clean_rules(&b.allow);
    let known: Vec<&str> = CATEGORIES.iter().map(|c| c.id).collect();
    b.categories.retain(|c| known.contains(&c.as_str()));
    b.categories.dedup();
    b.schedule.retain(|s| s.days & 0x7f != 0 && s.start < 1440 && s.end < 1440 && s.start != s.end);
    b.schedule.truncate(28);
    b
}

/// Whether `new` blocks at least everything `old` blocks (a locked list may
/// only change that way).
fn is_stricter_or_same(old: &FocusBlock, new: &FocusBlock) -> bool {
    old.sites.iter().all(|s| new.sites.contains(s))
        && old.categories.iter().all(|c| new.categories.contains(c))
        && new.allow.iter().all(|a| old.allow.contains(a))
        && old.schedule == new.schedule
        && (new.schedule_on || !old.schedule_on)
        && (new.schedule_locked || !old.schedule_locked)
        && !(old.unlock == Unlock::Strict && new.unlock == Unlock::RandomText)
}

impl Focus {
    pub fn load(path: PathBuf) -> Self {
        let saved: Saved = crate::store::read_json(&path).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        // Time never runs backwards across restarts.
        let base = system_ms().max(saved.saved_at_ms);
        let blocks: Vec<FocusBlock> = saved.blocks.into_iter().map(normalize_block).collect();
        let sessions: Vec<Session> = saved.sessions.into_iter().filter(|s| blocks.iter().any(|b| b.id == s.block_id)).collect();
        let had_sessions = !sessions.is_empty();
        let focus = Focus {
            path,
            inner: Mutex::new(Inner {
                blocks,
                sessions,
                suspended_until: saved.suspended_until,
                clock: TrustedClock { base_ms: base, base_at: Instant::now(), verified: false },
                last_tick: Instant::now(),
                challenges: BTreeMap::new(),
                signature: Vec::new(),
                dirty: false,
                downtime_pending: had_sessions.then_some((saved.saved_at_ms, saved.saved_at_verified)),
            }),
            active: RwLock::new(None),
            any: AtomicBool::new(false),
        };
        focus.recompile();
        focus
    }

    /// The block that applies to `url` right now, if any. Cheap when nothing
    /// is blocked (called for every request of every page).
    pub fn check(&self, url: &str) -> Option<Hit> {
        if !self.any.load(Ordering::Relaxed) {
            return None;
        }
        self.active.read().as_ref()?.check(url)
    }

    pub fn is_active(&self) -> bool {
        self.any.load(Ordering::Relaxed)
    }

    /// The rules in effect now (to compare with after a change).
    pub fn snapshot(&self) -> Option<Arc<Compiled>> {
        self.active.read().clone()
    }


    /// Sets the trusted clock from a network time; settles the time Serica
    /// was closed if the last save was network-verified too.
    pub fn apply_network_time(&self, net_ms: i64) {
        let mut g = self.inner.lock();
        g.clock = TrustedClock { base_ms: net_ms, base_at: Instant::now(), verified: true };
        if let Some((saved_at, verified)) = g.downtime_pending.take() {
            if verified && saved_at > 0 {
                let down = (net_ms - saved_at).max(0);
                for s in g.sessions.iter_mut() {
                    if let Some(r) = s.remaining_ms.as_mut() {
                        *r -= down;
                    }
                }
                g.sessions.retain(|s| s.remaining_ms.map_or(true, |r| r > 0));
                tracing::info!("focus: {down} ms passed while closed (network verified)");
            }
        }
        g.dirty = true;
        drop(g);
        self.recompile();
    }

    /// Counts time down and moves schedules along. Returns true when the set
    /// of blocked lists changed.
    pub fn tick(&self) -> bool {
        let mut g = self.inner.lock();
        let dt = g.last_tick.elapsed().as_millis() as i64;
        g.last_tick = Instant::now();
        if dt > 0 && !g.sessions.is_empty() {
            for s in g.sessions.iter_mut() {
                if let Some(r) = s.remaining_ms.as_mut() {
                    *r -= dt;
                }
            }
            let before = g.sessions.len();
            g.sessions.retain(|s| s.remaining_ms.map_or(true, |r| r > 0));
            if g.sessions.len() != before {
                g.dirty = true;
            }
        }
        let now = g.clock.now_ms();
        g.suspended_until.retain(|_, until| *until > now);
        drop(g);
        self.recompile()
    }

    /// Rebuilds the rules in effect. Returns whether the active set changed.
    fn recompile(&self) -> bool {
        let mut g = self.inner.lock();
        let now = g.clock.now_ms();
        let (minute, weekday, ms_in) = local_parts(now);
        let mut lists = Vec::new();
        let mut signature = Vec::new();
        for b in &g.blocks {
            let session = g.sessions.iter().find(|s| s.block_id == b.id);
            let sched_left = if b.schedule_on && !g.suspended_until.contains_key(&b.id) {
                b.schedule.iter().filter_map(|s| slot_remaining(s, weekday, minute, ms_in)).max()
            } else {
                None
            };
            if session.is_none() && sched_left.is_none() {
                continue;
            }
            let session_until = session.map(|s| s.remaining_ms.map(|r| now + r));
            let until = match (session_until, sched_left) {
                (Some(None), _) => None,
                (Some(Some(a)), Some(s)) => Some(a.max(now + s)),
                (Some(Some(a)), None) => Some(a),
                (None, Some(s)) => Some(now + s),
                (None, None) => None,
            };
            let locked = session.is_some_and(|s| s.locked) || (sched_left.is_some() && b.schedule_locked);
            let mut rules: Vec<Rule> = b.sites.iter().filter_map(|s| parse_rule(s)).collect();
            for c in &b.categories {
                if let Some(cat) = CATEGORIES.iter().find(|x| x.id == c) {
                    rules.extend(cat.sites.iter().filter_map(|s| parse_rule(s)));
                }
            }
            let allow = b.allow.iter().filter_map(|s| parse_rule(s)).collect();
            signature.push((b.id.clone(), locked, if session.is_some() { "session" } else { "schedule" }));
            if !rules.is_empty() {
                lists.push(ActiveRules { block_id: b.id.clone(), block_name: b.name.clone(), until_ms: until, locked, rules, allow });
            }
        }
        let changed = signature != g.signature || g.dirty;
        g.signature = signature;
        let any = !lists.is_empty();
        drop(g);
        *self.active.write() = any.then(|| Arc::new(Compiled { lists, at_ms: now, at: Instant::now() }));
        self.any.store(any, Ordering::Relaxed);
        changed
    }

    /// Whether `block_id` is locked right now.
    fn locked(&self, g: &Inner, block_id: &str) -> bool {
        let now = g.clock.now_ms();
        let (minute, weekday, ms_in) = local_parts(now);
        let session_locked = g.sessions.iter().any(|s| s.block_id == block_id && s.locked);
        let sched_locked = g.blocks.iter().find(|b| b.id == block_id).is_some_and(|b| {
            b.schedule_on
                && b.schedule_locked
                && !g.suspended_until.contains_key(&b.id)
                && b.schedule.iter().any(|s| slot_remaining(s, weekday, minute, ms_in).is_some())
        });
        session_locked || sched_locked
    }

    pub fn view(&self) -> FocusView {
        let g = self.inner.lock();
        let now = g.clock.now_ms();
        let (minute, weekday, ms_in) = local_parts(now);
        let mut active = Vec::new();
        for b in &g.blocks {
            if let Some(s) = g.sessions.iter().find(|s| s.block_id == b.id) {
                active.push(ActiveView { block_id: b.id.clone(), source: "session", remaining_ms: s.remaining_ms, total_ms: s.total_ms, locked: s.locked });
            } else if b.schedule_on && !g.suspended_until.contains_key(&b.id) {
                if let Some(left) = b.schedule.iter().filter_map(|s| slot_remaining(s, weekday, minute, ms_in)).max() {
                    active.push(ActiveView { block_id: b.id.clone(), source: "schedule", remaining_ms: Some(left), total_ms: None, locked: b.schedule_locked });
                }
            }
        }
        FocusView {
            blocks: g.blocks.clone(),
            active,
            categories: CATEGORIES.iter().map(|c| CategoryView { id: c.id, label: c.label, sites: c.sites.to_vec() }).collect(),
            clock_verified: g.clock.verified,
        }
    }

    /// Adds or updates a list. A locked list may only become stricter.
    pub fn save_block(&self, block: FocusBlock) -> Result<String, String> {
        let block = normalize_block(block);
        {
            let mut g = self.inner.lock();
            let id = if block.id.is_empty() { new_id() } else { block.id.clone() };
            let block = FocusBlock { id: id.clone(), ..block };
            match g.blocks.iter().position(|b| b.id == id) {
                Some(i) => {
                    if self.locked(&g, &id) && !is_stricter_or_same(&g.blocks[i], &block) {
                        return Err("locked".into());
                    }
                    g.blocks[i] = block;
                }
                None => {
                    if g.blocks.len() >= 40 {
                        return Err("too-many".into());
                    }
                    g.blocks.push(block);
                }
            }
            g.dirty = true;
            drop(g);
            self.recompile();
            Ok(id)
        }
    }

    pub fn delete_block(&self, id: &str) -> Result<(), String> {
        let mut g = self.inner.lock();
        if self.locked(&g, id) {
            return Err("locked".into());
        }
        g.blocks.retain(|b| b.id != id);
        g.sessions.retain(|s| s.block_id != id);
        g.dirty = true;
        drop(g);
        self.recompile();
        Ok(())
    }

    /// Starts (or lengthens) a session. A lock needs a duration; a locked
    /// session can only be made longer.
    pub fn start(&self, id: &str, minutes: Option<u32>, lock: bool) -> Result<(), String> {
        let minutes = minutes.map(|m| m.clamp(1, 24 * 60 * 7));
        if lock && minutes.is_none() {
            return Err("lock-needs-time".into());
        }
        let mut g = self.inner.lock();
        if !g.blocks.iter().any(|b| b.id == id) {
            return Err("no-such-list".into());
        }
        let ms = minutes.map(|m| m as i64 * 60_000);
        match g.sessions.iter_mut().find(|s| s.block_id == id) {
            Some(s) if s.locked => {
                // Stricter only: more time, still locked.
                let left = s.remaining_ms.unwrap_or(i64::MAX);
                if ms.is_some_and(|m| m > left) {
                    s.remaining_ms = ms;
                    s.total_ms = ms;
                }
            }
            Some(s) => {
                s.remaining_ms = ms;
                s.total_ms = ms;
                s.locked = lock;
            }
            None => g.sessions.push(Session { block_id: id.to_string(), remaining_ms: ms, total_ms: ms, locked: lock }),
        }
        g.dirty = true;
        drop(g);
        self.recompile();
        Ok(())
    }

    /// A long random text to type for an early end (lists with `RandomText`).
    pub fn challenge(&self, id: &str) -> Result<String, String> {
        let mut g = self.inner.lock();
        let b = g.blocks.iter().find(|b| b.id == id).ok_or("no-such-list")?;
        if b.unlock != Unlock::RandomText {
            return Err("strict".into());
        }
        const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
        let mut text = String::new();
        while text.len() < 64 {
            for byte in uuid::Uuid::new_v4().as_bytes() {
                if text.len() < 64 {
                    text.push(ALPHABET[(*byte as usize) % ALPHABET.len()] as char);
                }
            }
        }
        g.challenges.insert(id.to_string(), text.clone());
        Ok(text)
    }

    /// Ends a session, or this window of a schedule. A locked list needs the
    /// random text (when it allows that), otherwise it can't be stopped.
    pub fn stop(&self, id: &str, text: Option<&str>) -> Result<(), String> {
        let mut g = self.inner.lock();
        if self.locked(&g, id) {
            let allows_text = g.blocks.iter().find(|b| b.id == id).is_some_and(|b| b.unlock == Unlock::RandomText);
            let ok = allows_text && text.is_some_and(|t| g.challenges.get(id).is_some_and(|c| c == t));
            if !ok {
                return Err("locked".into());
            }
            g.challenges.remove(id);
        }
        g.sessions.retain(|s| s.block_id != id);
        // A schedule in effect is paused until its window ends.
        let now = g.clock.now_ms();
        let (minute, weekday, ms_in) = local_parts(now);
        let sched_left = g
            .blocks
            .iter()
            .find(|b| b.id == id && b.schedule_on)
            .and_then(|b| b.schedule.iter().filter_map(|s| slot_remaining(s, weekday, minute, ms_in)).max());
        if let Some(left) = sched_left {
            g.suspended_until.insert(id.to_string(), now + left);
        }
        g.dirty = true;
        drop(g);
        self.recompile();
        Ok(())
    }

    /// Whether anything needs saving; writes `focus.json` atomically.
    pub fn save(&self, force: bool) {
        let mut g = self.inner.lock();
        if !force && !g.dirty && g.sessions.is_empty() {
            return;
        }
        g.dirty = false;
        let saved = Saved {
            blocks: g.blocks.clone(),
            sessions: g.sessions.clone(),
            saved_at_ms: g.clock.now_ms(),
            saved_at_verified: g.clock.verified && g.downtime_pending.is_none(),
            suspended_until: g.suspended_until.clone(),
        };
        drop(g);
        if let Ok(bytes) = serde_json::to_vec_pretty(&saved) {
            if let Err(e) = crate::store::write_atomic(&self.path, &bytes) {
                tracing::error!("focus: could not save: {e}");
            }
        }
    }
}

// ------------------------------------------------------------------ block page

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// The page shown instead of a blocked site: calm, with the time left. It
/// reloads by itself when the time is up (and shows itself again if the site
/// is still blocked).
pub fn block_page(hit: &Hit, url: &str) -> String {
    let host = split_url(url).map(|(h, _)| h).unwrap_or_default();
    let data = serde_json::json!({
        "host": host,
        "list": hit.block_name,
        "remaining": hit.remaining_ms,
        "locked": hit.locked,
    });
    BLOCK_PAGE.replace("__TITLE__", &html_escape(&host)).replace("__DATA__", &data.to_string().replace("</", "<\\/"))
}

const BLOCK_PAGE: &str = r#"<!doctype html><html><head><meta charset="utf-8"><meta name="color-scheme" content="light dark">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>__TITLE__ · Focus</title>
<style>
:root { --bg:#f5f5f4; --fg:#1c1c1e; --muted:#6e6e73; --faint:#a1a1a6; --line:rgba(0,0,0,.08); }
@media (prefers-color-scheme: dark) { :root { --bg:#111113; --fg:#ececef; --muted:#9b9ba3; --faint:#5d5d66; --line:rgba(255,255,255,.08); } }
* { box-sizing:border-box; margin:0 }
html,body { height:100% }
body { background:var(--bg); color:var(--fg); display:grid; place-items:center; padding:32px;
  font:14px/1.55 "Inter Variable","Segoe UI Variable Text","Segoe UI",system-ui,sans-serif; -webkit-font-smoothing:antialiased; }
main { width:min(420px,100%); text-align:center; animation:in .5s cubic-bezier(.22,1,.36,1) both }
@keyframes in { from { opacity:0; transform:translateY(8px) } }
.mark { width:44px; height:44px; margin:0 auto 26px; border-radius:50%; border:1.5px solid var(--fg); position:relative; opacity:.9 }
.mark::after { content:""; position:absolute; inset:11px; border-radius:50%; background:var(--fg) }
h1 { font-size:22px; font-weight:600; letter-spacing:-.017em; margin-bottom:6px }
p { color:var(--muted) }
.time { margin:30px 0 8px; font-size:44px; font-weight:500; letter-spacing:-.024em; font-variant-numeric:tabular-nums; font-feature-settings:"tnum" 1 }
.label { color:var(--faint); font-size:12px; letter-spacing:.06em; text-transform:uppercase }
.list { margin-top:34px; padding-top:16px; border-top:1px solid var(--line); color:var(--faint); font-size:12.5px }
</style></head><body><main>
<div class="mark" aria-hidden="true"></div>
<h1 id="h"></h1><p id="p"></p>
<div class="time" id="t"></div><div class="label" id="l"></div>
<div class="list" id="n"></div>
</main><script>
(() => {
  const d = __DATA__;
  Object.defineProperty(window, '__sericaFocusBlock', { value: true });
  document.getElementById('h').textContent = (d.host || 'This site') + ' is blocked';
  document.getElementById('p').textContent = d.locked ? 'Focus Mode is locked. It ends on its own.' : 'You turned on Focus Mode for this site.';
  document.getElementById('n').textContent = 'Focus list: ' + d.list;
  const t = document.getElementById('t'), l = document.getElementById('l');
  if (d.remaining === null) { t.textContent = '∞'; l.textContent = 'until you turn it off'; return; }
  const end = performance.now() + d.remaining;
  const fmt = (ms) => { const s = Math.ceil(ms / 1000), h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), x = s % 60;
    return h ? h + ':' + String(m).padStart(2, '0') + ':' + String(x).padStart(2, '0') : m + ':' + String(x).padStart(2, '0'); };
  l.textContent = 'left';
  const tick = () => { const left = end - performance.now();
    if (left <= 0) { t.textContent = '0:00'; setTimeout(() => location.reload(), 800); return; }
    t.textContent = fmt(left); setTimeout(tick, 1000 - (left % 1000) + 5); };
  tick();
})();
</script></body></html>"#;

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(rule: &str, url: &str) -> bool {
        let r = parse_rule(rule).unwrap();
        let (h, p) = split_url(url).unwrap();
        r.matches(&h, &p)
    }

    #[test]
    fn rule_forms() {
        // Exact domain, with or without www and scheme.
        assert!(hit("reddit.com", "https://reddit.com/"));
        assert!(hit("reddit.com", "https://www.reddit.com/r/all"));
        assert!(hit("https://www.Reddit.com/", "https://reddit.com/x"));
        assert!(!hit("reddit.com", "https://old.reddit.com/"));
        assert!(!hit("reddit.com", "https://notreddit.com/"));
        // Wildcard subdomains include the domain itself.
        assert!(hit("*.reddit.com", "https://old.reddit.com/"));
        assert!(hit("*.reddit.com", "https://reddit.com/"));
        assert!(!hit("*.reddit.com", "https://reddit.co/"));
        // Paths match on segment boundaries.
        assert!(hit("youtube.com/shorts", "https://www.youtube.com/shorts/abc"));
        assert!(hit("youtube.com/shorts", "https://youtube.com/shorts"));
        assert!(!hit("youtube.com/shorts", "https://youtube.com/shortstop"));
        assert!(!hit("youtube.com/shorts", "https://youtube.com/watch?v=1"));
        // Globs.
        assert!(hit("*casino*", "https://best-casino.example/"));
        // Junk is refused.
        assert!(parse_rule("").is_none());
        assert!(parse_rule("hello world").is_none());
        assert!(parse_rule("localhost").is_none());
    }

    #[test]
    fn serica_pages_are_never_blocked() {
        assert!(split_url("serica://newtab").is_none());
        assert!(split_url("file:///C:/a.pdf").is_none());
    }

    fn temp_focus(name: &str) -> Focus {
        let p = std::env::temp_dir().join(format!("serica-focus-test-{name}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&p);
        Focus::load(p)
    }

    fn block(sites: &[&str], allow: &[&str]) -> FocusBlock {
        FocusBlock { name: "Work".into(), sites: sites.iter().map(|s| s.to_string()).collect(), allow: allow.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn sessions_block_and_exceptions_allow() {
        let f = temp_focus("session");
        let id = f.save_block(block(&["*.reddit.com"], &["reddit.com/r/rust"])).unwrap();
        assert!(f.check("https://reddit.com/").is_none(), "not active before start");
        f.start(&id, Some(25), false).unwrap();
        let h = f.check("https://www.reddit.com/r/all").unwrap();
        assert!(!h.locked);
        assert!(h.remaining_ms.unwrap() > 24 * 60_000);
        assert!(f.check("https://reddit.com/r/rust/comments/1").is_none(), "exception wins");
        assert!(f.check("https://example.com/").is_none());
        f.stop(&id, None).unwrap();
        assert!(f.check("https://reddit.com/").is_none());
    }

    #[test]
    fn categories_expand() {
        let f = temp_focus("cats");
        let mut b = block(&[], &[]);
        b.categories = vec!["video".into(), "bogus".into()];
        let id = f.save_block(b).unwrap();
        assert_eq!(f.view().blocks[0].categories, vec!["video".to_string()]);
        f.start(&id, None, false).unwrap();
        assert!(f.check("https://m.youtube.com/watch?v=x").is_some());
        assert!(f.check("https://github.com/").is_none());
    }

    #[test]
    fn locked_lists_resist() {
        let f = temp_focus("locked");
        let id = f.save_block(block(&["x.com", "reddit.com"], &[])).unwrap();
        assert_eq!(f.start(&id, None, true), Err("lock-needs-time".into()));
        f.start(&id, Some(60), true).unwrap();
        assert_eq!(f.stop(&id, None), Err("locked".into()));
        assert_eq!(f.delete_block(&id), Err("locked".into()));
        // Loosening is refused, tightening allowed.
        let mut looser = f.view().blocks[0].clone();
        looser.sites = vec!["x.com".into()];
        assert_eq!(f.save_block(looser), Err("locked".into()));
        let mut exception = f.view().blocks[0].clone();
        exception.allow = vec!["x.com/settings".into()];
        assert_eq!(f.save_block(exception), Err("locked".into()));
        let mut stricter = f.view().blocks[0].clone();
        stricter.sites.push("youtube.com".into());
        assert!(f.save_block(stricter).is_ok());
        assert!(f.check("https://youtube.com/").is_some());
        // A shorter session can't replace a locked one; a longer one can.
        f.start(&id, Some(5), true).unwrap();
        assert!(f.check("https://x.com/").unwrap().remaining_ms.unwrap() > 55 * 60_000);
        f.start(&id, Some(120), false).unwrap();
        assert!(f.check("https://x.com/").unwrap().remaining_ms.unwrap() > 115 * 60_000);
        assert!(f.check("https://x.com/").unwrap().locked, "still locked");
    }

    #[test]
    fn random_text_unlocks_only_with_the_text() {
        let f = temp_focus("text");
        let mut b = block(&["x.com"], &[]);
        b.unlock = Unlock::RandomText;
        let id = f.save_block(b).unwrap();
        f.start(&id, Some(30), true).unwrap();
        let text = f.challenge(&id).unwrap();
        assert_eq!(text.len(), 64);
        assert_eq!(f.stop(&id, Some("nope")), Err("locked".into()));
        assert!(f.stop(&id, Some(&text)).is_ok());
        assert!(f.check("https://x.com/").is_none());
    }

    #[test]
    fn locked_session_survives_restart_and_offline_time_is_not_counted() {
        let p = std::env::temp_dir().join(format!("serica-focus-test-restart-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let f = Focus::load(p.clone());
        let id = f.save_block(block(&["x.com"], &[])).unwrap();
        f.start(&id, Some(30), true).unwrap();
        f.save(true);
        // Saved with an unverified clock: a later network time can't shorten it.
        let g = Focus::load(p.clone());
        assert!(g.check("https://x.com/").unwrap().locked);
        g.apply_network_time(system_ms() + 10 * 24 * 3600 * 1000);
        assert!(g.check("https://x.com/").is_some(), "a jumped clock doesn't end the lock");
        assert_eq!(g.stop(&id, None), Err("locked".into()));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn verified_downtime_counts() {
        let p = std::env::temp_dir().join(format!("serica-focus-test-verified-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let f = Focus::load(p.clone());
        let now = system_ms();
        f.apply_network_time(now);
        let id = f.save_block(block(&["x.com"], &[])).unwrap();
        f.start(&id, Some(30), true).unwrap();
        f.save(true);
        let g = Focus::load(p.clone());
        // 10 verified minutes later: 20 left.
        g.apply_network_time(now + 10 * 60_000);
        let left = g.check("https://x.com/").unwrap().remaining_ms.unwrap();
        assert!(left <= 20 * 60_000 + 2000 && left > 19 * 60_000, "{left}");
        // 40 more verified minutes: over.
        g.save(true);
        let h = Focus::load(p.clone());
        h.apply_network_time(now + 60 * 60_000);
        assert!(h.check("https://x.com/").is_none());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn schedules() {
        let s = ScheduleSlot { days: 0b0111110, start: 9 * 60, end: 17 * 60 }; // Mon-Fri 9-17
        assert!(slot_remaining(&s, 1, 10 * 60, 0).is_some());
        assert_eq!(slot_remaining(&s, 1, 16 * 60 + 59, 0), Some(60_000));
        assert!(slot_remaining(&s, 0, 10 * 60, 0).is_none(), "Sunday");
        assert!(slot_remaining(&s, 1, 17 * 60, 0).is_none());
        let night = ScheduleSlot { days: 0b0000001, start: 22 * 60, end: 6 * 60 }; // Sunday 22:00 -> Monday 6:00
        assert!(slot_remaining(&night, 0, 23 * 60, 0).is_some());
        assert!(slot_remaining(&night, 1, 5 * 60, 0).is_some(), "Monday morning belongs to Sunday's slot");
        assert!(slot_remaining(&night, 1, 23 * 60, 0).is_none());
    }

    #[test]
    fn block_page_is_safe() {
        let h = Hit { block_id: "b".into(), block_name: "<script>x</script>".into(), remaining_ms: Some(1000), locked: true, whole_site: true };
        let page = block_page(&h, "https://evil.example/</script>");
        assert!(!page.contains("<script>x</script>"));
        assert!(page.contains("__sericaFocusBlock"));
    }
}

// ------------------------------------------------------------------ app glue

/// After the rules in effect changed: save, tell the UI, and reload open
/// pages whose blocked state changed (a page blocked now shows the block
/// page, a page shown blocked before gets its site back).
pub fn after_change(app: &crate::app::Shared, before: Option<Arc<Compiled>>) {
    app.focus.save(true);
    crate::app::ui_emit(&app.handle, "serica:focus-updated", app.focus.view());
    let after = app.focus.snapshot();
    // Views first, then URLs: never hold the store lock while taking the host lock.
    let views: Vec<(String, tauri::Webview)> = app.host.lock().views.iter().map(|(id, v)| (id.clone(), v.webview.clone())).collect();
    for (id, wv) in views {
        let Some(url) = app.store.lock().tab(&id).map(|t| t.url.clone()) else { continue };
        let was = before.as_ref().and_then(|c| c.check(&url)).is_some();
        let now = after.as_ref().and_then(|c| c.check(&url)).is_some();
        if was != now {
            if let Ok(u) = url::Url::parse(&url) {
                let _ = wv.navigate(u);
            }
        }
    }
    if let Some((wv, _)) = app.host.lock().assistant.as_ref().map(|(w, b)| (w.clone(), *b)) {
        let _ = wv.url().map(|u| {
            let s = u.to_string();
            let was = before.as_ref().and_then(|c| c.check(&s)).is_some();
            let now = after.as_ref().and_then(|c| c.check(&s)).is_some();
            if was != now {
                let _ = wv.navigate(u);
            }
        });
    }
}

type R<T> = Result<T, String>;

#[tauri::command]
pub fn focus_get(app: tauri::State<'_, crate::app::Shared>) -> FocusView {
    app.focus.view()
}

#[tauri::command]
pub async fn focus_save_block(block: FocusBlock, app: tauri::State<'_, crate::app::Shared>) -> R<FocusView> {
    let before = app.focus.snapshot();
    app.focus.save_block(block)?;
    after_change(&app, before);
    Ok(app.focus.view())
}

#[tauri::command]
pub async fn focus_delete_block(id: String, app: tauri::State<'_, crate::app::Shared>) -> R<FocusView> {
    let before = app.focus.snapshot();
    app.focus.delete_block(&id)?;
    after_change(&app, before);
    Ok(app.focus.view())
}

#[tauri::command]
pub async fn focus_start(id: String, minutes: Option<u32>, lock: bool, app: tauri::State<'_, crate::app::Shared>) -> R<FocusView> {
    let before = app.focus.snapshot();
    app.focus.start(&id, minutes, lock)?;
    after_change(&app, before);
    Ok(app.focus.view())
}

#[tauri::command]
pub async fn focus_stop(id: String, text: Option<String>, app: tauri::State<'_, crate::app::Shared>) -> R<FocusView> {
    let before = app.focus.snapshot();
    app.focus.stop(&id, text.as_deref())?;
    after_change(&app, before);
    Ok(app.focus.view())
}

#[tauri::command]
pub fn focus_challenge(id: String, app: tauri::State<'_, crate::app::Shared>) -> R<String> {
    app.focus.challenge(&id)
}

/// Whether a URL is blocked right now (the UI shows it in the address bar).
#[tauri::command]
pub fn focus_check(url: String, app: tauri::State<'_, crate::app::Shared>) -> bool {
    app.focus.check(&url).is_some()
}
