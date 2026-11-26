//! Focus statistics, Cold Turkey Pro style: how long you focused, the
//! sessions you finished or ended early, what you tried to open while it was
//! blocked (and when), and where your browsing time went, site by site.
//!
//! Everything is kept only on this PC (`focus-stats.json`), by local day,
//! for 400 days. Site time counts only while Serica is the window in front
//! and you are using it (or the page plays sound or video), never for the
//! block page, and can be switched off.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Instant;

/// Days kept.
const KEEP_DAYS: i64 = 400;
/// A day counts toward the streak with at least this much focus.
pub const STREAK_MS: i64 = 25 * 60_000;
/// Sites remembered per day; the rest are summed as "Other sites".
const MAX_SITES: usize = 600;
pub const OTHER: &str = "(other)";
/// The same site blocked again within this time is one attempt (reloads,
/// redirects and the page's own retries are not new attempts).
const ATTEMPT_GAP_MS: u128 = 4000;

fn yes() -> bool {
    true
}

fn hours<T: Default + Clone>(v: &mut Vec<T>) -> &mut Vec<T> {
    if v.len() != 24 {
        v.resize(24, T::default());
    }
    v
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Day {
    pub focus_ms: i64,
    pub focus_hours: Vec<i64>,
    /// Focus time per list (by id).
    pub lists: BTreeMap<String, i64>,
    pub started: u32,
    pub completed: u32,
    pub stopped: u32,
    /// Longest unbroken stretch of focus reached on this day.
    pub longest_ms: i64,
    pub blocked: u32,
    pub blocked_hours: Vec<u32>,
    pub blocked_sites: BTreeMap<String, u32>,
    pub browse_ms: i64,
    pub browse_hours: Vec<i64>,
    /// Time on sites that are in one of your lists (blocked or not).
    pub distract_ms: i64,
    pub sites: BTreeMap<String, i64>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Saved {
    #[serde(default = "yes")]
    track_sites: bool,
    days: BTreeMap<String, Day>,
    /// Names of lists, so deleted ones still read well.
    list_names: BTreeMap<String, String>,
}

impl Default for Saved {
    fn default() -> Self {
        Saved { track_sites: true, days: BTreeMap::new(), list_names: BTreeMap::new() }
    }
}

struct Inner {
    saved: Saved,
    dirty: bool,
    last_blocked: HashMap<String, Instant>,
    /// Current unbroken focus.
    stretch_ms: i64,
}

pub struct Stats {
    path: PathBuf,
    inner: Mutex<Inner>,
}

/// What the user is looking at this second.
pub struct Browsing {
    pub host: String,
    /// The site is in one of the user's lists.
    pub listed: bool,
}

fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

fn key(d: chrono::NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn hour_now() -> usize {
    use chrono::Timelike;
    chrono::Local::now().hour() as usize % 24
}

/// `example.com` from a URL (no `www.`), for http(s) pages only.
pub fn host_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    let h = u.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    Some(h.strip_prefix("www.").map(str::to_string).unwrap_or(h))
}

// ------------------------------------------------------------------ view

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    /// Date (`2026-11-26`), month (`2026-11`) or hour (`14`).
    pub key: String,
    pub focus_ms: i64,
    pub browse_ms: i64,
    pub distract_ms: i64,
    pub blocked: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteRow {
    pub host: String,
    pub ms: i64,
    pub listed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountRow {
    pub host: String,
    pub count: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListRow {
    pub id: String,
    pub name: String,
    pub ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub focus_ms: i64,
    pub browse_ms: i64,
    pub distract_ms: i64,
    pub blocked: u32,
    pub started: u32,
    pub completed: u32,
    pub stopped: u32,
    pub longest_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsView {
    /// "day" | "week" | "month" | "year"
    pub range: String,
    pub tracking: bool,
    /// The chart: hours of the day, days, or months.
    pub buckets: Vec<Bucket>,
    pub totals: Totals,
    /// The period before, for comparison.
    pub previous: Totals,
    /// Days in the range (for daily averages).
    pub days: u32,
    pub streak: u32,
    pub best_streak: u32,
    pub streak_goal_ms: i64,
    pub sites: Vec<SiteRow>,
    pub blocked_sites: Vec<CountRow>,
    /// Blocked attempts and focus by hour of the day, over the range.
    pub blocked_hours: Vec<u32>,
    pub focus_hours: Vec<i64>,
    pub lists: Vec<ListRow>,
    /// Any data at all, ever.
    pub has_data: bool,
}

fn totals<'a>(days: impl Iterator<Item = &'a Day>) -> Totals {
    let mut t = Totals { focus_ms: 0, browse_ms: 0, distract_ms: 0, blocked: 0, started: 0, completed: 0, stopped: 0, longest_ms: 0 };
    for d in days {
        t.focus_ms += d.focus_ms;
        t.browse_ms += d.browse_ms;
        t.distract_ms += d.distract_ms;
        t.blocked += d.blocked;
        t.started += d.started;
        t.completed += d.completed;
        t.stopped += d.stopped;
        t.longest_ms = t.longest_ms.max(d.longest_ms);
    }
    t
}

impl Stats {
    pub fn load(path: PathBuf) -> Self {
        let saved: Saved = crate::store::read_json(&path).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        Stats { path, inner: Mutex::new(Inner { saved, dirty: false, last_blocked: HashMap::new(), stretch_ms: 0 }) }
    }

    fn day<'a>(g: &'a mut Inner) -> &'a mut Day {
        g.saved.days.entry(key(today())).or_default()
    }

    /// One step of the clock: `dt_ms` of focus for the lists in effect, and
    /// of browsing for the site in front (if any).
    pub fn tick(&self, dt_ms: i64, active: &[(String, String)], browsing: Option<Browsing>) {
        if dt_ms <= 0 {
            return;
        }
        let h = hour_now();
        let mut g = self.inner.lock();
        if active.is_empty() {
            g.stretch_ms = 0;
        } else {
            g.stretch_ms += dt_ms;
            let stretch = g.stretch_ms;
            for (id, name) in active {
                if g.saved.list_names.get(id) != Some(name) {
                    g.saved.list_names.insert(id.clone(), name.clone());
                }
            }
            let d = Self::day(&mut g);
            d.focus_ms += dt_ms;
            hours(&mut d.focus_hours)[h] += dt_ms;
            for (id, _) in active {
                *d.lists.entry(id.clone()).or_default() += dt_ms;
            }
            d.longest_ms = d.longest_ms.max(stretch);
            g.dirty = true;
        }
        if let Some(b) = browsing.filter(|_| g.saved.track_sites) {
            let d = Self::day(&mut g);
            d.browse_ms += dt_ms;
            hours(&mut d.browse_hours)[h] += dt_ms;
            if b.listed {
                d.distract_ms += dt_ms;
            }
            let site = if d.sites.len() >= MAX_SITES && !d.sites.contains_key(&b.host) { OTHER.to_string() } else { b.host };
            *d.sites.entry(site).or_default() += dt_ms;
            g.dirty = true;
        }
    }

    /// A blocked page was asked for (top-level or frame document).
    pub fn blocked(&self, url: &str) {
        let Some(host) = host_of(url) else { return };
        let mut g = self.inner.lock();
        let now = Instant::now();
        if g.last_blocked.get(&host).is_some_and(|t| now.duration_since(*t).as_millis() < ATTEMPT_GAP_MS) {
            g.last_blocked.insert(host, now);
            return;
        }
        g.last_blocked.insert(host.clone(), now);
        if g.last_blocked.len() > 256 {
            g.last_blocked.retain(|_, t| now.duration_since(*t).as_millis() < ATTEMPT_GAP_MS);
        }
        let h = hour_now();
        let d = Self::day(&mut g);
        d.blocked += 1;
        hours(&mut d.blocked_hours)[h] += 1;
        let site = if d.blocked_sites.len() >= MAX_SITES && !d.blocked_sites.contains_key(&host) { OTHER.to_string() } else { host };
        *d.blocked_sites.entry(site).or_default() += 1;
        g.dirty = true;
    }

    /// Serica itself is about to show the block page for an open tab (a list
    /// just started): that is not an attempt, so the next request is skipped.
    pub fn expect_block(&self, url: &str) {
        if let Some(host) = host_of(url) {
            self.inner.lock().last_blocked.insert(host, Instant::now());
        }
    }

    pub fn session_started(&self) {
        let mut g = self.inner.lock();
        Self::day(&mut g).started += 1;
        g.dirty = true;
    }

    /// A session ran to its end (`true`) or was stopped early.
    pub fn session_ended(&self, completed: bool) {
        let mut g = self.inner.lock();
        let d = Self::day(&mut g);
        if completed {
            d.completed += 1;
        } else {
            d.stopped += 1;
        }
        g.dirty = true;
    }

    pub fn set_tracking(&self, on: bool) {
        let mut g = self.inner.lock();
        g.saved.track_sites = on;
        g.dirty = true;
    }

    pub fn clear(&self) {
        let mut g = self.inner.lock();
        let track = g.saved.track_sites;
        g.saved = Saved { track_sites: track, ..Saved::default() };
        g.stretch_ms = 0;
        g.dirty = true;
    }

    /// Writes `focus-stats.json` atomically when something changed; drops
    /// days older than 400.
    pub fn save(&self, force: bool) {
        let mut g = self.inner.lock();
        if !force && !g.dirty {
            return;
        }
        g.dirty = false;
        let oldest = key(today() - chrono::Duration::days(KEEP_DAYS));
        g.saved.days.retain(|k, _| *k >= oldest);
        let bytes = serde_json::to_vec(&g.saved);
        drop(g);
        if let Ok(bytes) = bytes {
            if let Err(e) = crate::store::write_atomic(&self.path, &bytes) {
                tracing::error!("focus stats: could not save: {e}");
            }
        }
    }

    /// Consecutive days, ending today (or yesterday, while today is still
    /// short of the goal), with at least 25 minutes of focus; and the best run.
    fn streaks(days: &BTreeMap<String, Day>, today: chrono::NaiveDate) -> (u32, u32) {
        let good = |d: chrono::NaiveDate| days.get(&key(d)).is_some_and(|x| x.focus_ms >= STREAK_MS);
        let mut cur = 0;
        let mut d = if good(today) { today } else { today - chrono::Duration::days(1) };
        while good(d) {
            cur += 1;
            d -= chrono::Duration::days(1);
        }
        let mut best = 0;
        let mut run = 0;
        let mut prev: Option<chrono::NaiveDate> = None;
        for (k, v) in days {
            let Ok(date) = chrono::NaiveDate::parse_from_str(k, "%Y-%m-%d") else { continue };
            if v.focus_ms >= STREAK_MS {
                run = if prev.is_some_and(|p| date - p == chrono::Duration::days(1)) { run + 1 } else { 1 };
                prev = Some(date);
                best = best.max(run);
            } else {
                prev = None;
                run = 0;
            }
        }
        (cur, best.max(cur))
    }

    pub fn view(&self, range: &str, is_listed: impl Fn(&str) -> bool) -> StatsView {
        self.view_at(range, today(), is_listed)
    }

    fn view_at(&self, range: &str, today: chrono::NaiveDate, is_listed: impl Fn(&str) -> bool) -> StatsView {
        use chrono::{Datelike, Duration};
        let g = self.inner.lock();
        let days_map = &g.saved.days;
        let span: i64 = match range {
            "day" => 1,
            "month" => 30,
            "year" => 365,
            _ => 7,
        };
        let range = match span {
            1 => "day",
            30 => "month",
            365 => "year",
            _ => "week",
        };
        let first = today - Duration::days(span - 1);
        let in_range: Vec<(chrono::NaiveDate, &Day)> = (0..span)
            .filter_map(|i| {
                let d = first + Duration::days(i);
                days_map.get(&key(d)).map(|x| (d, x))
            })
            .collect();
        let prev_first = first - Duration::days(span);
        let previous = totals((0..span).filter_map(|i| days_map.get(&key(prev_first + Duration::days(i)))));
        let totals_now = totals(in_range.iter().map(|(_, d)| *d));

        let buckets: Vec<Bucket> = match range {
            "day" => {
                let d = days_map.get(&key(today));
                (0..24)
                    .map(|h| Bucket {
                        key: h.to_string(),
                        focus_ms: d.and_then(|d| d.focus_hours.get(h).copied()).unwrap_or(0),
                        browse_ms: d.and_then(|d| d.browse_hours.get(h).copied()).unwrap_or(0),
                        distract_ms: 0,
                        blocked: d.and_then(|d| d.blocked_hours.get(h).copied()).unwrap_or(0),
                    })
                    .collect()
            }
            "year" => {
                // Twelve calendar months, ending with this one.
                let mut out = Vec::new();
                for back in (0..12).rev() {
                    let total = today.year() * 12 + today.month0() as i32 - back;
                    let (y, m) = (total.div_euclid(12), total.rem_euclid(12) as u32 + 1);
                    let prefix = format!("{y:04}-{m:02}");
                    let mut b = Bucket { key: prefix.clone(), ..Default::default() };
                    for (_, d) in days_map.range(format!("{prefix}-00")..format!("{prefix}-99")) {
                        b.focus_ms += d.focus_ms;
                        b.browse_ms += d.browse_ms;
                        b.distract_ms += d.distract_ms;
                        b.blocked += d.blocked;
                    }
                    out.push(b);
                }
                out
            }
            _ => (0..span)
                .map(|i| {
                    let date = first + Duration::days(i);
                    let d = days_map.get(&key(date));
                    Bucket {
                        key: key(date),
                        focus_ms: d.map_or(0, |d| d.focus_ms),
                        browse_ms: d.map_or(0, |d| d.browse_ms),
                        distract_ms: d.map_or(0, |d| d.distract_ms),
                        blocked: d.map_or(0, |d| d.blocked),
                    }
                })
                .collect(),
        };

        let mut sites: BTreeMap<&str, i64> = BTreeMap::new();
        let mut blocked_sites: BTreeMap<&str, u32> = BTreeMap::new();
        let mut lists: BTreeMap<&str, i64> = BTreeMap::new();
        let mut blocked_hours = vec![0u32; 24];
        let mut focus_hours = vec![0i64; 24];
        for (_, d) in &in_range {
            for (s, ms) in &d.sites {
                *sites.entry(s).or_default() += ms;
            }
            for (s, n) in &d.blocked_sites {
                *blocked_sites.entry(s).or_default() += n;
            }
            for (id, ms) in &d.lists {
                *lists.entry(id).or_default() += ms;
            }
            for (h, n) in d.blocked_hours.iter().enumerate().take(24) {
                blocked_hours[h] += n;
            }
            for (h, ms) in d.focus_hours.iter().enumerate().take(24) {
                focus_hours[h] += ms;
            }
        }
        let mut sites: Vec<SiteRow> = sites.into_iter().map(|(h, ms)| SiteRow { listed: h != OTHER && is_listed(h), host: h.to_string(), ms }).collect();
        sites.sort_by(|a, b| b.ms.cmp(&a.ms).then_with(|| a.host.cmp(&b.host)));
        sites.truncate(15);
        let mut blocked_sites: Vec<CountRow> = blocked_sites.into_iter().map(|(h, count)| CountRow { host: h.to_string(), count }).collect();
        blocked_sites.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.host.cmp(&b.host)));
        blocked_sites.truncate(10);
        let mut lists: Vec<ListRow> = lists
            .into_iter()
            .map(|(id, ms)| ListRow { id: id.to_string(), name: g.saved.list_names.get(id).cloned().unwrap_or_else(|| "A deleted list".into()), ms })
            .collect();
        lists.sort_by(|a, b| b.ms.cmp(&a.ms));

        let (streak, best_streak) = Self::streaks(days_map, today);
        StatsView {
            range: range.to_string(),
            tracking: g.saved.track_sites,
            buckets,
            totals: totals_now,
            previous,
            days: span as u32,
            streak,
            best_streak,
            streak_goal_ms: STREAK_MS,
            sites,
            blocked_sites,
            blocked_hours,
            focus_hours,
            lists,
            has_data: !days_map.is_empty(),
        }
    }

    /// Every day as CSV, for a spreadsheet.
    pub fn csv(&self) -> String {
        let g = self.inner.lock();
        let mut out = String::from("date,focus_minutes,sessions_started,sessions_completed,sessions_stopped,longest_focus_minutes,blocked_attempts,browsing_minutes,minutes_on_listed_sites,top_site,top_site_minutes,most_blocked_site,most_blocked_count\n");
        let mins = |ms: i64| format!("{:.1}", ms as f64 / 60_000.0);
        let quote = |s: &str| if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() };
        for (k, d) in &g.saved.days {
            let top = d.sites.iter().max_by_key(|(_, ms)| **ms);
            let most = d.blocked_sites.iter().max_by_key(|(_, n)| **n);
            out.push_str(&format!(
                "{k},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                mins(d.focus_ms),
                d.started,
                d.completed,
                d.stopped,
                mins(d.longest_ms),
                d.blocked,
                mins(d.browse_ms),
                mins(d.distract_ms),
                top.map(|(h, _)| quote(h)).unwrap_or_default(),
                top.map(|(_, ms)| mins(*ms)).unwrap_or_default(),
                most.map(|(h, _)| quote(h)).unwrap_or_default(),
                most.map(|(_, n)| n.to_string()).unwrap_or_default(),
            ));
        }
        out
    }
}

/// The site in front this second, if the user is actually on it: Serica is
/// the foreground window, they used the keyboard or mouse in the last two
/// minutes (or the page plays media), and it is a web page, not the block
/// page or a new tab.
pub fn sample_browsing(app: &crate::app::Shared) -> Option<Browsing> {
    if !app.host.lock().visible {
        return None;
    }
    let (url, media) = {
        let s = app.store.lock();
        let t = s.active_tab()?;
        (t.url.clone(), t.has_active_media)
    };
    let host = host_of(&url)?;
    if app.focus.check(&url).is_some() {
        return None;
    }
    #[cfg(windows)]
    unsafe {
        use tauri::Manager;
        use windows::Win32::System::SystemInformation::GetTickCount;
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
        use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GetForegroundWindow, GA_ROOT};
        let main = app.handle.get_webview_window("main")?.hwnd().ok()?;
        let fg = GetForegroundWindow();
        if fg.is_invalid() || GetAncestor(fg, GA_ROOT) != main {
            return None;
        }
        let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        let idle_ms = if GetLastInputInfo(&mut info).as_bool() { GetTickCount().wrapping_sub(info.dwTime) } else { 0 };
        if idle_ms > 120_000 && !media {
            return None;
        }
    }
    let listed = app.focus.listed(&url);
    Some(Browsing { host, listed })
}

type R<T> = Result<T, String>;

#[tauri::command]
pub fn focus_stats(range: String, app: tauri::State<'_, crate::app::Shared>) -> StatsView {
    app.focus_stats.view(&range, |host| app.focus.listed(&format!("https://{host}/")))
}

#[tauri::command]
pub fn focus_stats_tracking(on: bool, app: tauri::State<'_, crate::app::Shared>) -> R<()> {
    app.focus_stats.set_tracking(on);
    app.focus_stats.save(true);
    Ok(())
}

#[tauri::command]
pub fn focus_stats_clear(app: tauri::State<'_, crate::app::Shared>) -> R<()> {
    app.focus_stats.clear();
    app.focus_stats.save(true);
    Ok(())
}

/// Saves every day as a CSV file where the user picks; returns its path.
#[tauri::command]
pub async fn focus_stats_export(app: tauri::State<'_, crate::app::Shared>) -> R<Option<String>> {
    let csv = app.focus_stats.csv();
    let name = format!("Serica Focus {}.csv", today().format("%Y-%m-%d"));
    let Some(path) = crate::files::pick_save(&app, &name, "CSV", &["csv"]).await else { return Ok(None) };
    std::fs::write(&path, csv).map_err(|e| e.to_string())?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats() -> Stats {
        Stats { path: std::env::temp_dir().join("serica-focus-stats-test.json"), inner: Mutex::new(Inner { saved: Saved::default(), dirty: false, last_blocked: HashMap::new(), stretch_ms: 0 }) }
    }

    #[test]
    fn counts_focus_lists_and_stretches() {
        let s = stats();
        let a = vec![("l1".to_string(), "Deep work".to_string())];
        for _ in 0..90 {
            s.tick(1000, &a, None);
        }
        s.tick(1000, &[], None);
        for _ in 0..30 {
            s.tick(1000, &a, None);
        }
        let v = s.view("day", |_| false);
        assert_eq!(v.totals.focus_ms, 120_000);
        assert_eq!(v.totals.longest_ms, 90_000);
        assert_eq!(v.lists[0].name, "Deep work");
        assert_eq!(v.lists[0].ms, 120_000);
        assert_eq!(v.buckets.len(), 24);
        assert_eq!(v.buckets.iter().map(|b| b.focus_ms).sum::<i64>(), 120_000);
    }

    #[test]
    fn blocked_attempts_are_debounced_per_site() {
        let s = stats();
        s.blocked("https://www.reddit.com/r/all");
        s.blocked("https://reddit.com/r/rust");
        s.blocked("https://x.com/home");
        s.blocked("about:blank");
        let v = s.view("week", |_| true);
        assert_eq!(v.totals.blocked, 2);
        assert_eq!(v.blocked_sites.len(), 2);
        assert!(v.blocked_sites.iter().any(|r| r.host == "reddit.com" && r.count == 1));
        assert_eq!(v.blocked_hours.iter().sum::<u32>(), 2);
    }

    #[test]
    fn reloads_serica_makes_are_not_attempts() {
        let s = stats();
        s.expect_block("https://youtube.com/watch?v=1");
        s.blocked("https://www.youtube.com/watch?v=1");
        assert_eq!(s.view("day", |_| true).totals.blocked, 0);
    }

    #[test]
    fn site_time_respects_tracking_switch() {
        let s = stats();
        s.tick(1000, &[], Some(Browsing { host: "github.com".into(), listed: false }));
        s.tick(1000, &[], Some(Browsing { host: "youtube.com".into(), listed: true }));
        s.set_tracking(false);
        s.tick(1000, &[], Some(Browsing { host: "github.com".into(), listed: false }));
        let v = s.view("week", |h| h == "youtube.com");
        assert_eq!(v.totals.browse_ms, 2000);
        assert_eq!(v.totals.distract_ms, 1000);
        assert!(!v.tracking);
        assert!(v.sites.iter().any(|r| r.host == "youtube.com" && r.listed));
    }

    #[test]
    fn streaks_count_days_with_enough_focus() {
        let s = stats();
        let t = chrono::NaiveDate::from_ymd_opt(2026, 11, 26).unwrap();
        {
            let mut g = s.inner.lock();
            for back in [1, 2, 3, 6, 7] {
                g.saved.days.insert(key(t - chrono::Duration::days(back)), Day { focus_ms: STREAK_MS, ..Default::default() });
            }
            // Today is still short: the streak carries from yesterday.
            g.saved.days.insert(key(t), Day { focus_ms: 60_000, ..Default::default() });
        }
        let v = s.view_at("month", t, |_| false);
        assert_eq!(v.streak, 3);
        assert_eq!(v.best_streak, 3);
        let y = s.view_at("year", t, |_| false);
        assert_eq!(y.buckets.len(), 12);
        assert_eq!(y.buckets.last().unwrap().key, "2026-11");
        assert_eq!(y.buckets.last().unwrap().focus_ms, STREAK_MS * 5 + 60_000);
    }

    #[test]
    fn sessions_and_previous_period() {
        let s = stats();
        let t = chrono::NaiveDate::from_ymd_opt(2026, 11, 26).unwrap();
        {
            let mut g = s.inner.lock();
            g.saved.days.insert(key(t - chrono::Duration::days(8)), Day { focus_ms: 10, completed: 1, ..Default::default() });
            g.saved.days.insert(key(t), Day { focus_ms: 20, started: 2, completed: 1, stopped: 1, ..Default::default() });
        }
        let v = s.view_at("week", t, |_| false);
        assert_eq!((v.totals.started, v.totals.completed, v.totals.stopped), (2, 1, 1));
        assert_eq!(v.previous.focus_ms, 10);
        assert_eq!(v.buckets.len(), 7);
        assert_eq!(v.buckets.last().unwrap().key, "2026-11-26");
    }

    #[test]
    fn csv_has_a_row_per_day() {
        let s = stats();
        s.blocked("https://reddit.com/");
        s.tick(60_000, &[("a".into(), "A".into())], Some(Browsing { host: "news, \"ok\".com".into(), listed: false }));
        let csv = s.csv();
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains(",1.0,"));
        assert!(lines[1].contains("\"news, \"\"ok\"\".com\""));
    }
}
