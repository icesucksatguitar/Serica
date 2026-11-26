//! Browser state: profiles, spaces, tabs, favorites, folders, archive.
//!
//! `Store` is plain data plus pure-ish operations; it never touches webviews.
//! Callers lock it briefly, mutate, then let `AppState::changed` persist and
//! broadcast. Tab order inside `tabs` is the sidebar order within each
//! (space, section) list, exactly as the UI's `sidebarModel` expects.

use crate::model::*;
use crate::settings::BrowserSettings;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const MAX_CLOSED: usize = 30;
const MAX_ARCHIVE: usize = 300;
/// Rough per-tab renderer memory used for the "memory saved" estimate.
pub const MB_PER_SLEEPING_TAB: f64 = 48.0;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PersistedState {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    profiles: Vec<Profile>,
    #[serde(default)]
    spaces: Vec<Space>,
    #[serde(default)]
    active_space_id: String,
    #[serde(default)]
    tabs: Vec<Value>,
    #[serde(default)]
    active_tab_id: Option<String>,
    #[serde(default)]
    favorites: Vec<FavoriteEntry>,
    #[serde(default)]
    folders: Vec<FolderRecord>,
    #[serde(default)]
    archive: Vec<ArchivedTab>,
    #[serde(default)]
    closed_tabs: Vec<TabState>,
    #[serde(default)]
    shield_stats: ShieldStats,
}

/// A drag-and-drop or context-menu move, mirroring `MoveOp` in the UI.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MoveOp {
    Reorder {
        section: String,
        id: String,
        #[serde(rename = "toIndex")]
        to_index: i64,
        /// Pinned folder to land in: absent keeps the tab's folder, `null` is the root.
        #[serde(rename = "folderId", default, deserialize_with = "present_option")]
        folder_id: Option<Option<String>>,
    },
    Section {
        id: String,
        from: String,
        to: String,
        #[serde(rename = "spaceId")]
        space_id: String,
        #[serde(rename = "toIndex", default)]
        to_index: Option<i64>,
        #[serde(rename = "folderId", default, deserialize_with = "present_option")]
        folder_id: Option<Option<String>>,
    },
    Space { id: String, from: String, #[serde(rename = "targetSpaceId")] target_space_id: String },
}

/// Tells a field sent as `null` (`Some(None)`) from a missing one (`None`).
fn present_option<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Ok(Some(Option::<String>::deserialize(d)?))
}

#[derive(Debug)]
pub struct Store {
    pub profiles: Vec<Profile>,
    pub spaces: Vec<Space>,
    pub active_space_id: String,
    pub tabs: Vec<TabState>,
    pub active_tab_id: Option<String>,
    pub favorites: Vec<FavoriteEntry>,
    pub folders: Vec<FolderRecord>,
    pub archive: Vec<ArchivedTab>,
    pub closed: Vec<TabState>,
    pub settings: BrowserSettings,
    pub shield_stats: ShieldStats,
    pub data_dir: PathBuf,
}

pub fn default_spaces() -> Vec<Space> {
    [("space-study", "Study", "BookOpen", "#E4E4E7"), ("space-work", "Work", "Briefcase", "#A1A1AA"), ("space-leisure", "Leisure", "Coffee", "#71717A")]
        .iter()
        .enumerate()
        .map(|(i, (id, name, icon, color))| Space {
            id: id.to_string(),
            name: name.to_string(),
            icon: icon.to_string(),
            color: color.to_string(),
            profile_id: Some(DEFAULT_PROFILE_ID.into()),
            order: Some(i as i32),
        })
        .collect()
}

const CUSTOM_SPACE_PALETTE: &[(&str, &str)] = &[
    ("#8B5CF6", "Sparkles"),
    ("#3B82F6", "Compass"),
    ("#10B981", "Leaf"),
    ("#F59E0B", "Sun"),
    ("#EC4899", "Flame"),
    ("#06B6D4", "Globe"),
];

/// Writes `contents` to `path` atomically (temp file + rename) so a crash or a
/// concurrent save can never leave a truncated JSON file behind.
pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, contents)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Writes JSON files, with `backup` keeping the previous copy as `.json.bak`
/// for `read_json`'s fallback. Skips a file whose bytes match what this process
/// last wrote there (`written` remembers a hash per path).
pub fn write_files(written: &parking_lot::Mutex<HashMap<PathBuf, u64>>, files: Vec<(PathBuf, Vec<u8>)>, backup: bool) {
    use std::hash::{Hash, Hasher};
    for (path, bytes) in files {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut h);
        let hash = h.finish();
        if written.lock().get(&path) == Some(&hash) {
            continue;
        }
        if backup && path.exists() {
            let _ = std::fs::copy(&path, path.with_extension("json.bak"));
        }
        match write_atomic(&path, &bytes) {
            Ok(()) => {
                written.lock().insert(path, hash);
            }
            Err(e) => tracing::error!("failed to save {}: {e}", path.display()),
        }
    }
}

/// Reads a JSON file, falling back to its `.bak` copy if the main file is corrupt.
pub fn read_json(path: &Path) -> Option<Value> {
    let parse = |p: &Path| std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok());
    parse(path).or_else(|| parse(&path.with_extension("json.bak")))
}

fn section_of(name: &str) -> Option<Section> {
    match name {
        "pinned" => Some(Section::Pinned),
        "today" => Some(Section::Today),
        _ => None,
    }
}

impl Store {
    pub fn load(data_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&data_dir);
        let settings_path = data_dir.join("settings.json");
        let state_path = data_dir.join("state.json");

        let mut settings = read_json(&settings_path).map(BrowserSettings::from_value);
        let mut persisted: Option<PersistedState> =
            read_json(&state_path).and_then(|v| serde_json::from_value(v).ok());

        if settings.is_none() && persisted.is_none() {
            if let Some(imported) = crate::importer::try_import_electron_data() {
                tracing::info!("imported Electron profile data");
                settings = imported.settings.map(BrowserSettings::from_value);
                persisted = imported.state.and_then(|v| serde_json::from_value(v).ok());
            }
        }

        let p = persisted.unwrap_or_default();
        let tabs: Vec<TabState> = p
            .tabs
            .into_iter()
            .filter_map(|v| serde_json::from_value::<TabState>(v).ok())
            .collect();

        let mut store = Store {
            profiles: p.profiles,
            spaces: p.spaces,
            active_space_id: p.active_space_id,
            tabs,
            active_tab_id: p.active_tab_id,
            favorites: p.favorites,
            folders: p.folders,
            archive: p.archive,
            closed: p.closed_tabs,
            settings: settings.unwrap_or_default(),
            shield_stats: p.shield_stats,
            data_dir,
        };
        store.repair();
        store
    }

    /// Restores every invariant the rest of the code relies on.
    pub fn repair(&mut self) {
        if self.profiles.is_empty() {
            self.profiles.push(Profile { id: DEFAULT_PROFILE_ID.into(), name: "Default".into(), created_at: 0 });
        }
        if self.spaces.is_empty() {
            self.spaces = default_spaces();
        }
        let default_profile = self.profiles[0].id.clone();
        let profile_ids: Vec<String> = self.profiles.iter().map(|p| p.id.clone()).collect();
        for s in &mut self.spaces {
            // Legacy ids from the first Tauri builds.
            if s.profile_id.as_deref() == Some("profile-default") {
                s.profile_id = Some(DEFAULT_PROFILE_ID.into());
            }
            if !s.profile_id.as_ref().map(|p| profile_ids.contains(p)).unwrap_or(false) {
                s.profile_id = Some(default_profile.clone());
            }
            if s.name.trim().is_empty() {
                s.name = "Space".into();
            }
        }
        self.renumber_spaces();
        let space_ids: Vec<String> = self.spaces.iter().map(|s| s.id.clone()).collect();
        let mut seen = std::collections::HashSet::new();
        self.tabs.retain(|t| space_ids.contains(&t.space_id) && seen.insert(t.id.clone()));
        for t in &mut self.tabs {
            t.reset_runtime();
        }
        self.folders.retain(|f| space_ids.contains(&f.space_id));
        let folder_ids: Vec<String> = self.folders.iter().map(|f| f.id.clone()).collect();
        for t in &mut self.tabs {
            if t.folder_id.as_ref().map(|f| !folder_ids.contains(f)).unwrap_or(false) {
                t.folder_id = None;
            }
        }
        self.favorites.retain(|f| profile_ids.contains(&f.profile_id));
        if !space_ids.contains(&self.active_space_id) {
            self.active_space_id = self.spaces[0].id.clone();
        }
        let active_ok = self
            .active_tab_id
            .as_ref()
            .and_then(|id| self.tab(id))
            .map(|t| t.space_id == self.active_space_id && !t.unloaded)
            .unwrap_or(false);
        if !active_ok {
            let sid = self.active_space_id.clone();
            self.activate_space(&sid);
        }
    }

    #[cfg(test)]
    pub fn save(&self) {
        write_files(&Default::default(), self.serialized(), true);
    }

    /// state.json and settings.json as bytes, ready for `write_files`.
    pub fn serialized(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let state = serde_json::json!({
            "schemaVersion": 2,
            "profiles": self.profiles,
            "spaces": self.spaces,
            "activeSpaceId": self.active_space_id,
            // Private tabs are never written down.
            "tabs": self.tabs.iter().filter(|t| !t.private).collect::<Vec<_>>(),
            "activeTabId": self.active_tab_id.as_ref().filter(|id| self.tab(id).is_some_and(|t| !t.private)),
            "favorites": self.favorites,
            "folders": self.folders,
            "archive": self.archive,
            "closedTabs": self.closed,
            "shieldStats": self.shield_stats,
        });
        [("state.json", state), ("settings.json", serde_json::to_value(&self.settings).unwrap_or_default())]
            .into_iter()
            .filter_map(|(name, value)| serde_json::to_vec_pretty(&value).ok().map(|t| (self.data_dir.join(name), t)))
            .collect()
    }

    pub fn snapshot(&self) -> BrowserAppState {
        let sleeping_count = self.tabs.iter().filter(|t| t.is_sleeping).count();
        BrowserAppState {
            spaces: self.spaces.clone(),
            active_space_id: self.active_space_id.clone(),
            tabs: self.tabs.clone(),
            active_tab_id: self.active_tab_id.clone(),
            shield_stats: self.shield_stats.clone(),
            resource_stats: ResourceStats {
                memory_saved_mb: sleeping_count as f64 * MB_PER_SLEEPING_TAB,
                sleeping_count,
            },
            profiles: self.profiles.clone(),
            active_profile_id: self.active_profile_id(),
            favorites: self.favorites.clone(),
            folders: self.folders.clone(),
        }
    }

    // ------------------------------------------------------------------ lookups

    pub fn tab(&self, id: &str) -> Option<&TabState> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn tab_mut(&mut self, id: &str) -> Option<&mut TabState> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn active_tab(&self) -> Option<&TabState> {
        self.active_tab_id.as_deref().and_then(|id| self.tab(id))
    }

    pub fn space(&self, id: &str) -> Option<&Space> {
        self.spaces.iter().find(|s| s.id == id)
    }

    pub fn space_profile(&self, space_id: &str) -> String {
        self.space(space_id)
            .and_then(|s| s.profile_id.clone())
            .unwrap_or_else(|| self.profiles[0].id.clone())
    }

    pub fn active_profile_id(&self) -> String {
        self.space_profile(&self.active_space_id)
    }

    /// Spaces in Space_Switcher order: profiles by creation, then space order.
    pub fn switcher_order(&self) -> Vec<String> {
        let mut profiles = self.profiles.clone();
        profiles.sort_by_key(|p| p.created_at);
        let mut out = Vec::new();
        for p in profiles {
            let mut spaces: Vec<&Space> = self.spaces.iter().filter(|s| s.profile_id.as_deref() == Some(&p.id)).collect();
            spaces.sort_by_key(|s| s.order.unwrap_or(0));
            out.extend(spaces.into_iter().map(|s| s.id.clone()));
        }
        out
    }

    /// Indices into `tabs` for one sidebar list, in display order.
    fn list(&self, space_id: &str, section: Section) -> Vec<usize> {
        self.tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| t.space_id == space_id && t.section() == section)
            .map(|(i, _)| i)
            .collect()
    }

    /// Visible tab order of a space: pinned list, then Today.
    pub fn visible_order(&self, space_id: &str) -> Vec<String> {
        let mut ids: Vec<String> = self.list(space_id, Section::Pinned).into_iter().map(|i| self.tabs[i].id.clone()).collect();
        ids.extend(self.list(space_id, Section::Today).into_iter().map(|i| self.tabs[i].id.clone()));
        ids
    }

    /// Removes the tab with `id` and inserts `tab` at `index` within the
    /// (space, section) list it now belongs to (clamped; `None` = end).
    fn place(&mut self, mut tab: TabState, space_id: &str, section: Section, index: Option<i64>) {
        self.tabs.retain(|t| t.id != tab.id);
        tab.space_id = space_id.to_string();
        tab.set_section(section);
        let list = self.list(space_id, section);
        let idx = index.map(|i| i.clamp(0, list.len() as i64) as usize).unwrap_or(list.len());
        let pos = if idx < list.len() {
            list[idx]
        } else if let Some(&last) = list.last() {
            last + 1
        } else {
            self.tabs.len()
        };
        self.tabs.insert(pos, tab);
    }

    fn profile_favorites(&self, profile_id: &str) -> Vec<FavoriteEntry> {
        let mut favs: Vec<FavoriteEntry> = self.favorites.iter().filter(|f| f.profile_id == profile_id).cloned().collect();
        favs.sort_by_key(|f| f.order);
        favs
    }

    fn write_favorites(&mut self, profile_id: &str, list: Vec<FavoriteEntry>) {
        self.favorites.retain(|f| f.profile_id != profile_id);
        for (i, mut f) in list.into_iter().enumerate() {
            f.order = i as i32;
            f.profile_id = profile_id.to_string();
            self.favorites.push(f);
        }
    }

    fn renumber_spaces(&mut self) {
        let mut by_profile: HashMap<String, i32> = HashMap::new();
        let mut order: Vec<usize> = (0..self.spaces.len()).collect();
        order.sort_by_key(|&i| self.spaces[i].order.unwrap_or(i as i32));
        let mut spaces = Vec::with_capacity(self.spaces.len());
        for i in order {
            let mut s = self.spaces[i].clone();
            let n = by_profile.entry(s.profile_id.clone().unwrap_or_default()).or_insert(0);
            s.order = Some(*n);
            *n += 1;
            spaces.push(s);
        }
        self.spaces = spaces;
    }

    // ---------------------------------------------------------------- activation

    /// Makes `id` the active tab (switching space if needed). Wakes it and
    /// reloads a closed pinned tab.
    pub fn activate_tab(&mut self, id: &str) -> bool {
        if self.tab(id).is_none() {
            return false;
        }
        // The tab being left was in use until now (it may have been restored
        // as the active tab at startup and never re-activated): without this
        // it could look idle for hours and be archived right after the switch.
        let now = now_ms();
        if let Some(prev) = self.active_tab_id.clone().filter(|p| p != id) {
            if let Some(t) = self.tab_mut(&prev) {
                t.last_active_at = Some(now);
            }
        }
        let Some(tab) = self.tab_mut(id) else { return false };
        tab.is_sleeping = false;
        tab.unloaded = false;
        tab.last_active_at = Some(now_ms());
        let space = tab.space_id.clone();
        self.active_space_id = space;
        self.active_tab_id = Some(id.to_string());
        true
    }

    /// Activates a space and picks its most recently used live tab, creating a
    /// new tab if it has none (R13.6).
    pub fn activate_space(&mut self, space_id: &str) -> bool {
        if self.space(space_id).is_none() {
            return false;
        }
        // Like `activate_tab`: the tab being left was in use until now, or a
        // tab used only through space switches would look idle and be put to
        // sleep (then rebuilt, blank, on the way back) right after leaving it.
        let now = now_ms();
        if let Some(prev) = self.active_tab_id.clone() {
            if let Some(t) = self.tab_mut(&prev) {
                t.last_active_at = Some(now);
            }
        }
        self.active_space_id = space_id.to_string();
        let order = self.visible_order(space_id);
        let best = order
            .iter()
            .filter_map(|id| self.tab(id))
            .filter(|t| !t.unloaded)
            .max_by_key(|t| t.last_active_at.unwrap_or(0))
            .map(|t| t.id.clone());
        match best {
            Some(id) => {
                self.active_tab_id = Some(id.clone());
                if let Some(t) = self.tab_mut(&id) {
                    t.is_sleeping = false;
                    t.last_active_at = Some(now);
                }
            }
            None => {
                let tab = TabState::new(space_id, None);
                let id = tab.id.clone();
                self.place(tab, space_id, Section::Today, None);
                self.active_tab_id = Some(id);
            }
        }
        true
    }

    // ---------------------------------------------------------------- tab CRUD

    /// Creates a Today tab. `after` inserts it right after an opener tab.
    pub fn create_tab(&mut self, url: Option<String>, space_id: Option<String>, activate: bool, after: Option<&str>) -> TabState {
        let space_id = space_id.filter(|s| self.space(s).is_some()).unwrap_or_else(|| self.active_space_id.clone());
        let mut tab = TabState::new(&space_id, url);
        // The site's remembered zoom, else the default (text size).
        tab.zoom_factor = Some(self.settings.zoom_for(&tab.url));
        let index = after.and_then(|a| {
            let list = self.list(&space_id, Section::Today);
            list.iter().position(|&i| self.tabs[i].id == a).map(|p| p as i64 + 1)
        });
        self.place(tab.clone(), &space_id, Section::Today, index);
        if activate {
            self.activate_tab(&tab.id);
        }
        self.tab(&tab.id).cloned().unwrap_or(tab)
    }

    /// A private tab in the active space, made active.
    pub fn create_private_tab(&mut self, url: Option<String>) -> TabState {
        let space_id = self.active_space_id.clone();
        let mut tab = TabState::new(&space_id, url);
        tab.private = true;
        tab.zoom_factor = Some(self.settings.zoom_for(&tab.url));
        self.place(tab.clone(), &space_id, Section::Today, None);
        self.activate_tab(&tab.id);
        self.tab(&tab.id).cloned().unwrap_or(tab)
    }

    /// The tab to show after `closing` (in the active space) goes away:
    /// next in visible order, else previous, skipping unloaded pinned tabs.
    fn neighbor_of(&self, closing: &str) -> Option<String> {
        let order: Vec<String> = self
            .visible_order(&self.active_space_id)
            .into_iter()
            .filter(|id| id == closing || self.tab(id).map(|t| !t.unloaded).unwrap_or(false))
            .collect();
        let pos = order.iter().position(|id| id == closing)?;
        order.get(pos + 1).or_else(|| pos.checked_sub(1).and_then(|p| order.get(p))).cloned()
    }

    /// Closes a tab. A pinned tab stays in the sidebar as `unloaded`; a Today
    /// tab is removed and remembered for reopen (R12.9, R15.3).
    pub fn close_tab(&mut self, id: &str) -> bool {
        let Some(tab) = self.tab(id).cloned() else { return false };
        let was_active = self.active_tab_id.as_deref() == Some(id);
        let next = if was_active { self.neighbor_of(id) } else { None };
        if tab.section() == Section::Pinned {
            if let Some(t) = self.tab_mut(id) {
                t.unloaded = true;
                t.is_sleeping = false;
                t.is_loading = false;
                t.has_active_media = false;
            }
        } else {
            self.tabs.retain(|t| t.id != id);
            if !is_internal_url(&tab.url) && !tab.private {
                self.closed.push(tab.clone());
                if self.closed.len() > MAX_CLOSED {
                    self.closed.remove(0);
                }
            }
        }
        if was_active {
            match next {
                Some(n) => {
                    self.activate_tab(&n);
                }
                None => {
                    let space = self.active_space_id.clone();
                    let t = self.create_tab(None, Some(space), true, None);
                    self.active_tab_id = Some(t.id);
                }
            }
        }
        true
    }

    pub fn reopen_closed_tab(&mut self) -> Option<TabState> {
        let mut tab = self.closed.pop()?;
        if self.space(&tab.space_id).is_none() {
            tab.space_id = self.active_space_id.clone();
        }
        if self.tab(&tab.id).is_some() {
            tab.id = new_id("tab");
        }
        tab.reset_runtime();
        tab.is_sleeping = false;
        tab.unloaded = false;
        tab.folder_id = None;
        let space = tab.space_id.clone();
        self.place(tab.clone(), &space, Section::Today, None);
        self.activate_tab(&tab.id);
        self.tab(&tab.id).cloned()
    }

    /// Next/previous live tab in the active space, wrapping around.
    pub fn cycle_tab(&mut self, direction: i32) -> Option<TabState> {
        let order: Vec<String> = self
            .visible_order(&self.active_space_id)
            .into_iter()
            .filter(|id| self.tab(id).map(|t| !t.unloaded).unwrap_or(false))
            .collect();
        if order.is_empty() {
            return None;
        }
        let cur = self.active_tab_id.as_ref().and_then(|a| order.iter().position(|id| id == a)).unwrap_or(0) as i64;
        let n = order.len() as i64;
        let next = ((cur + direction.signum() as i64) % n + n) % n;
        let id = order[next as usize].clone();
        self.activate_tab(&id);
        self.tab(&id).cloned()
    }

    pub fn duplicate_tab(&mut self, id: &str, space_id: Option<String>) -> Option<TabState> {
        if let Some(src) = self.tab(id).cloned() {
            let mut copy = TabState::new(&src.space_id, Some(src.url.clone()));
            copy.title = src.title.clone();
            copy.favicon = src.favicon.clone();
            let after = if src.section() == Section::Today { Some(src.id.as_str()) } else { None };
            let index = after.and_then(|a| {
                let list = self.list(&src.space_id, Section::Today);
                list.iter().position(|&i| self.tabs[i].id == a).map(|p| p as i64 + 1)
            });
            let space = src.space_id.clone();
            self.place(copy.clone(), &space, Section::Today, index);
            return self.tab(&copy.id).cloned();
        }
        let fav = self.favorites.iter().find(|f| f.id == id).cloned()?;
        let space = space_id.filter(|s| self.space(s).is_some()).unwrap_or_else(|| self.active_space_id.clone());
        let mut copy = TabState::new(&space, Some(fav.url.clone()));
        copy.title = fav.title.clone();
        copy.favicon = fav.favicon.clone();
        self.place(copy.clone(), &space, Section::Today, None);
        self.tab(&copy.id).cloned()
    }

    // ---------------------------------------------------------------- layout moves

    /// Applies a sidebar move. Returns `Some(needs_reload)` when something moved.
    pub fn apply_move(&mut self, op: &MoveOp) -> Option<bool> {
        match op {
            MoveOp::Reorder { section, id, to_index, folder_id } => {
                if section == "favorite" {
                    let fav = self.favorites.iter().find(|f| f.id == *id)?.clone();
                    let mut list: Vec<FavoriteEntry> = self.profile_favorites(&fav.profile_id).into_iter().filter(|f| f.id != *id).collect();
                    let idx = (*to_index).clamp(0, list.len() as i64) as usize;
                    list.insert(idx, fav.clone());
                    self.write_favorites(&fav.profile_id, list);
                    return Some(false);
                }
                let sec = section_of(section)?;
                let tab = self.tab(id).filter(|t| t.section() == sec)?.clone();
                let space = tab.space_id.clone();
                self.place(tab, &space, sec, Some(*to_index));
                if let (Section::Pinned, Some(folder)) = (sec, folder_id) {
                    self.set_tab_folder(id, folder.as_deref());
                }
                Some(false)
            }
            MoveOp::Section { id, from, to, space_id, to_index, folder_id } => {
                let moved = self.relocate(id, from, to, space_id, *to_index)?;
                if let ("pinned", Some(folder)) = (to.as_str(), folder_id) {
                    // A favorite dropped here became a tab with the favorite's id.
                    self.set_tab_folder(id, folder.as_deref());
                }
                Some(moved)
            }
            MoveOp::Space { id, from, target_space_id } => {
                let to = if from == "pinned" { "pinned" } else { "today" };
                self.relocate(id, from, to, target_space_id, None)
            }
        }
    }

    fn relocate(&mut self, id: &str, from: &str, to: &str, space_id: &str, index: Option<i64>) -> Option<bool> {
        self.space(space_id)?;
        // A private tab can't be pinned or kept as a favorite (both are saved).
        if from != "favorite" && (to == "pinned" || to == "favorite") && self.tab(id).is_some_and(|t| t.private) {
            return None;
        }
        let dest_profile = self.space_profile(space_id);
        if from == "favorite" {
            let fav = self.favorites.iter().find(|f| f.id == id)?.clone();
            let needs_reload = fav.profile_id != dest_profile;
            if to == "favorite" {
                let mut list: Vec<FavoriteEntry> = self.profile_favorites(&dest_profile).into_iter().filter(|f| f.id != id).collect();
                self.favorites.retain(|f| f.id != id);
                let idx = index.unwrap_or(list.len() as i64).clamp(0, list.len() as i64) as usize;
                list.insert(idx, fav);
                self.write_favorites(&dest_profile, list);
                return Some(needs_reload);
            }
            let sec = section_of(to)?;
            // A favorite dropped into a tab list becomes a tab (not loaded until selected).
            self.favorites.retain(|f| f.id != id);
            let remaining = self.profile_favorites(&fav.profile_id);
            self.write_favorites(&fav.profile_id, remaining);
            let mut tab = TabState::new(space_id, Some(fav.url.clone()));
            tab.id = fav.id.clone();
            tab.title = fav.title.clone();
            tab.favicon = fav.favicon.clone();
            self.place(tab, space_id, sec, index);
            return Some(needs_reload);
        }
        let src_sec = section_of(from)?;
        let tab = self.tab(id).filter(|t| t.section() == src_sec)?.clone();
        let needs_reload = self.space_profile(&tab.space_id) != dest_profile;
        if to == "favorite" {
            // Favorites are launchers: the open tab keeps running where it is.
            if self.favorites.iter().any(|f| f.profile_id == dest_profile && same_url(&f.url, &tab.url)) {
                return Some(false);
            }
            let mut list = self.profile_favorites(&dest_profile);
            let idx = index.unwrap_or(list.len() as i64).clamp(0, list.len() as i64) as usize;
            list.insert(
                idx,
                FavoriteEntry {
                    id: new_id("fav"),
                    profile_id: dest_profile.clone(),
                    url: tab.url.clone(),
                    title: tab.title.clone(),
                    favicon: tab.favicon.clone(),
                    order: 0,
                },
            );
            self.write_favorites(&dest_profile, list);
            return Some(false);
        }
        let dest_sec = section_of(to)?;
        let moving_space = tab.space_id != space_id;
        let was_active = self.active_tab_id.as_deref() == Some(id);
        let next = if was_active && moving_space { self.neighbor_of(id) } else { None };
        let mut moved = tab;
        if moving_space || dest_sec != Section::Pinned {
            moved.folder_id = None;
        }
        self.place(moved, space_id, dest_sec, index);
        if was_active && moving_space {
            // The tab left the visible space; keep the user where they are.
            match next {
                Some(n) => {
                    self.activate_tab(&n);
                }
                None => {
                    let sid = self.active_space_id.clone();
                    self.activate_space(&sid);
                }
            }
        }
        Some(needs_reload)
    }

    /// Adds a site as a pinned tab that loads when first opened (onboarding).
    pub fn add_pinned_site(&mut self, url: String, title: String, favicon: Option<String>, space_id: Option<String>) -> Option<TabState> {
        let tab = self.create_tab(Some(url), space_id, false, None);
        if let Some(t) = self.tab_mut(&tab.id) {
            t.title = title;
            t.favicon = favicon;
            t.unloaded = true;
        }
        if !self.set_pinned(&tab.id, true) {
            return None;
        }
        self.tab(&tab.id).cloned()
    }

    pub fn set_pinned(&mut self, id: &str, pinned: bool) -> bool {
        let Some(tab) = self.tab(id).cloned() else { return false };
        let (from, to) = if pinned { ("today", "pinned") } else { ("pinned", "today") };
        if (tab.section() == Section::Pinned) == pinned {
            return true;
        }
        if !pinned {
            if let Some(t) = self.tab_mut(id) {
                // An unpinned closed tab has no sidebar entry to come back to.
                t.unloaded = false;
            }
        }
        let space = tab.space_id.clone();
        self.relocate(id, from, to, &space, None).is_some()
    }

    pub fn add_favorite(&mut self, tab_id: &str) -> bool {
        let Some(tab) = self.tab(tab_id).cloned() else { return false };
        let from = if tab.section() == Section::Pinned { "pinned" } else { "today" };
        self.relocate(tab_id, from, "favorite", &tab.space_id, None).is_some()
    }

    pub fn remove_favorite(&mut self, fav_id: &str) -> bool {
        let Some(fav) = self.favorites.iter().find(|f| f.id == fav_id).cloned() else { return false };
        let list: Vec<FavoriteEntry> = self.profile_favorites(&fav.profile_id).into_iter().filter(|f| f.id != fav_id).collect();
        self.favorites.retain(|f| f.id != fav_id);
        self.write_favorites(&fav.profile_id, list);
        true
    }

    // ---------------------------------------------------------------- folders

    pub fn create_folder(&mut self, space_id: &str, name: &str) -> Option<FolderRecord> {
        self.space(space_id)?;
        let order = self.folders.iter().filter(|f| f.space_id == space_id).count() as i32;
        let name = name.trim();
        let folder = FolderRecord {
            id: new_id("folder"),
            space_id: space_id.to_string(),
            name: if name.is_empty() { "New Folder".into() } else { name.chars().take(40).collect() },
            order,
            collapsed: Some(false),
            icon: None,
            color: None,
        };
        self.folders.push(folder.clone());
        Some(folder)
    }

    pub fn update_folder(&mut self, id: &str, patch: &Value) -> Option<FolderRecord> {
        let f = self.folders.iter_mut().find(|f| f.id == id)?;
        if let Some(name) = patch.get("name").and_then(Value::as_str) {
            let name = name.trim();
            if !name.is_empty() {
                f.name = name.chars().take(40).collect();
            }
        }
        if let Some(c) = patch.get("collapsed").and_then(Value::as_bool) {
            f.collapsed = Some(c);
        }
        if let Some(icon) = patch.get("icon") {
            f.icon = icon.as_str().map(str::to_string);
        }
        if let Some(color) = patch.get("color") {
            f.color = color.as_str().map(str::to_string);
        }
        if let Some(order) = patch.get("order").and_then(Value::as_i64) {
            f.order = order as i32;
        }
        Some(f.clone())
    }

    /// Deletes a folder; its tabs stay pinned at the folder's place.
    pub fn delete_folder(&mut self, id: &str) -> bool {
        let before = self.folders.len();
        self.folders.retain(|f| f.id != id);
        for t in &mut self.tabs {
            if t.folder_id.as_deref() == Some(id) {
                t.folder_id = None;
            }
        }
        before != self.folders.len()
    }

    /// Puts a pinned tab into one of its own space's folders (`None`: the
    /// pinned root). A folder of another space is ignored.
    fn set_tab_folder(&mut self, tab_id: &str, folder_id: Option<&str>) {
        let Some(space) = self.tab(tab_id).map(|t| t.space_id.clone()) else { return };
        if let Some(fid) = folder_id {
            if !self.folders.iter().any(|f| f.id == fid && f.space_id == space) {
                return;
            }
        }
        if let Some(t) = self.tab_mut(tab_id) {
            if t.section() == Section::Pinned {
                t.folder_id = folder_id.map(str::to_string);
            }
        }
    }

    pub fn move_tab_to_folder(&mut self, tab_id: &str, folder_id: Option<&str>) -> bool {
        let Some(tab) = self.tab(tab_id).cloned() else { return false };
        if let Some(fid) = folder_id {
            let Some(folder) = self.folders.iter().find(|f| f.id == fid).cloned() else { return false };
            if tab.space_id != folder.space_id || tab.section() != Section::Pinned {
                let from = if tab.section() == Section::Pinned { "pinned" } else { "today" };
                if self.relocate(tab_id, from, "pinned", &folder.space_id, None).is_none() {
                    return false;
                }
            }
            if let Some(t) = self.tab_mut(tab_id) {
                t.folder_id = Some(fid.to_string());
            }
        } else if let Some(t) = self.tab_mut(tab_id) {
            t.folder_id = None;
        }
        true
    }

    // ---------------------------------------------------------------- spaces

    pub fn create_space(&mut self, name: &str, color: Option<String>, profile_id: Option<String>) -> Option<Space> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let profile = match profile_id {
            Some(p) if self.profiles.iter().any(|x| x.id == p) => p,
            Some(_) => return None,
            None => self.active_profile_id(),
        };
        let (pal_color, icon) = CUSTOM_SPACE_PALETTE[self.spaces.len() % CUSTOM_SPACE_PALETTE.len()];
        let color = color.filter(|c| is_hex_color(c)).unwrap_or_else(|| pal_color.to_string());
        let order = self.spaces.iter().filter(|s| s.profile_id.as_deref() == Some(&profile)).count() as i32;
        let space = Space {
            id: new_id("space"),
            name: name.chars().take(32).collect(),
            icon: icon.to_string(),
            color,
            profile_id: Some(profile),
            order: Some(order),
        };
        self.spaces.push(space.clone());
        self.activate_space(&space.id);
        Some(space)
    }

    pub fn delete_space(&mut self, space_id: &str) -> Result<(), &'static str> {
        let space = self.space(space_id).cloned().ok_or("not-found")?;
        let profile = space.profile_id.clone();
        if self.spaces.iter().filter(|s| s.profile_id == profile).count() <= 1 {
            return Err("last-space");
        }
        self.spaces.retain(|s| s.id != space_id);
        self.tabs.retain(|t| t.space_id != space_id);
        self.folders.retain(|f| f.space_id != space_id);
        self.closed.retain(|t| t.space_id != space_id);
        self.settings.space_themes.remove(space_id);
        self.renumber_spaces();
        if self.active_space_id == space_id {
            let next = self
                .spaces
                .iter()
                .find(|s| s.profile_id == profile)
                .or_else(|| self.spaces.first())
                .map(|s| s.id.clone())
                .unwrap_or_default();
            self.activate_space(&next);
        }
        Ok(())
    }

    pub fn update_space(&mut self, space_id: &str, patch: &Value) -> Option<Space> {
        let s = self.spaces.iter_mut().find(|s| s.id == space_id)?;
        if let Some(name) = patch.get("name").and_then(Value::as_str) {
            let name: String = name.trim().chars().take(32).collect();
            if !name.is_empty() {
                s.name = name;
            }
        }
        if let Some(color) = patch.get("color").and_then(Value::as_str) {
            if is_hex_color(color) {
                s.color = color.to_string();
            }
        }
        if let Some(icon) = patch.get("icon").and_then(Value::as_str) {
            if !icon.trim().is_empty() {
                s.icon = icon.to_string();
            }
        }
        Some(s.clone())
    }

    /// Moves a space within its profile's switcher list.
    pub fn reorder_space(&mut self, space_id: &str, to_index: usize) -> bool {
        let Some(space) = self.space(space_id).cloned() else { return false };
        let mut group: Vec<Space> = self.spaces.iter().filter(|s| s.profile_id == space.profile_id).cloned().collect();
        group.sort_by_key(|s| s.order.unwrap_or(0));
        group.retain(|s| s.id != space_id);
        let idx = to_index.min(group.len());
        group.insert(idx, space);
        for (i, g) in group.iter().enumerate() {
            if let Some(s) = self.spaces.iter_mut().find(|s| s.id == g.id) {
                s.order = Some(i as i32);
            }
        }
        true
    }

    // ---------------------------------------------------------------- profiles

    pub fn create_profile(&mut self, name: Option<&str>) -> (Profile, Space) {
        let n = self.profiles.len() + 1;
        let name = name.map(str::trim).filter(|s| !s.is_empty()).map(|s| s.chars().take(32).collect()).unwrap_or_else(|| format!("Profile {n}"));
        let profile = Profile { id: new_id("profile"), name, created_at: now_ms() };
        self.profiles.push(profile.clone());
        let space = self.create_space("Home", None, Some(profile.id.clone())).expect("space for new profile");
        (profile, space)
    }

    pub fn rename_profile(&mut self, id: &str, name: &str) -> Option<Profile> {
        let name: String = name.trim().chars().take(32).collect();
        if name.is_empty() {
            return None;
        }
        let p = self.profiles.iter_mut().find(|p| p.id == id)?;
        p.name = name;
        Some(p.clone())
    }

    pub fn delete_profile(&mut self, id: &str) -> Result<(), &'static str> {
        if !self.profiles.iter().any(|p| p.id == id) {
            return Err("unknown-profile");
        }
        if self.profiles.len() <= 1 {
            return Err("last-profile");
        }
        let spaces: Vec<String> = self.spaces.iter().filter(|s| s.profile_id.as_deref() == Some(id)).map(|s| s.id.clone()).collect();
        self.profiles.retain(|p| p.id != id);
        self.spaces.retain(|s| !spaces.contains(&s.id));
        self.tabs.retain(|t| !spaces.contains(&t.space_id));
        self.folders.retain(|f| !spaces.contains(&f.space_id));
        self.closed.retain(|t| !spaces.contains(&t.space_id));
        self.favorites.retain(|f| f.profile_id != id);
        self.archive.retain(|a| a.profile_id != id);
        for s in &spaces {
            self.settings.space_themes.remove(s);
        }
        if spaces.contains(&self.active_space_id) {
            let first = self.switcher_order().first().cloned().unwrap_or_default();
            self.activate_space(&first);
        }
        Ok(())
    }

    // ---------------------------------------------------------------- sleep / archive

    /// Puts inactive tabs to sleep. `min_idle_ms = 0` sleeps every eligible tab
    /// (manual Zen); otherwise only tabs idle at least that long. Pinned tabs,
    /// tabs playing media and the active tab are never slept.
    pub fn sleep_inactive(&mut self, min_idle_ms: i64) -> usize {
        self.sleep_least_recent(min_idle_ms, usize::MAX)
    }

    /// Sleeps up to `limit` eligible tabs idle at least `min_idle_ms`, least
    /// recently used first (used when the system runs low on memory).
    pub fn sleep_least_recent(&mut self, min_idle_ms: i64, limit: usize) -> usize {
        let now = now_ms();
        let active = self.active_tab_id.clone();
        let mut eligible: Vec<(i64, usize)> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                Some(&t.id) != active.as_ref()
                    && !t.is_sleeping
                    && !t.unloaded
                    && !t.has_active_media
                    // Sleeping a private tab would end its session.
                    && !t.private
                    && t.section() != Section::Pinned
                    && !is_internal_url(&t.url)
            })
            .map(|(i, t)| (t.last_active_at.unwrap_or(0), i))
            .filter(|(last, _)| now - last >= min_idle_ms)
            .collect();
        eligible.sort_unstable();
        let n = eligible.len().min(limit);
        for &(_, i) in &eligible[..n] {
            let t = &mut self.tabs[i];
            t.is_sleeping = true;
            t.is_loading = false;
        }
        n
    }

    /// Archives Today tabs unused for longer than the archive interval (R16).
    pub fn archive_sweep(&mut self) -> usize {
        let Some(hours) = self.settings.archive_after_hours() else { return 0 };
        let cutoff = now_ms() - hours * 3_600_000;
        let active = self.active_tab_id.clone();
        let mut archived = Vec::new();
        for t in &self.tabs {
            if t.section() == Section::Today
                && Some(&t.id) != active.as_ref()
                && !t.has_active_media
                && !t.private
                && t.last_active_at.unwrap_or(i64::MAX) < cutoff
            {
                archived.push(t.clone());
            }
        }
        for t in &archived {
            self.tabs.retain(|x| x.id != t.id);
            if !is_internal_url(&t.url) {
                self.archive.push(ArchivedTab {
                    id: new_id("archived"),
                    profile_id: self.space_profile(&t.space_id),
                    space_id: t.space_id.clone(),
                    url: t.url.clone(),
                    title: t.title.clone(),
                    favicon: t.favicon.clone(),
                    archived_at: now_ms(),
                });
            }
        }
        if self.archive.len() > MAX_ARCHIVE {
            let extra = self.archive.len() - MAX_ARCHIVE;
            self.archive.drain(0..extra);
        }
        archived.len()
    }

    pub fn archive_list(&self) -> Vec<Value> {
        let profile = self.active_profile_id();
        let mut list: Vec<&ArchivedTab> = self.archive.iter().filter(|a| a.profile_id == profile).collect();
        list.sort_by(|a, b| b.archived_at.cmp(&a.archived_at));
        list.into_iter()
            .map(|a| {
                let mut v = serde_json::to_value(a).unwrap_or_default();
                v["spaceName"] = self.space(&a.space_id).map(|s| Value::from(s.name.clone())).unwrap_or(Value::Null);
                v
            })
            .collect()
    }

    pub fn restore_archived(&mut self, id: &str) -> Result<TabState, &'static str> {
        let profile = self.active_profile_id();
        let item = self.archive.iter().find(|a| a.id == id).cloned().ok_or("unknown-archived-tab")?;
        if item.profile_id != profile {
            return Err("forbidden");
        }
        self.archive.retain(|a| a.id != id);
        let space = if self.space(&item.space_id).is_some() { Some(item.space_id.clone()) } else { None };
        let mut tab = self.create_tab(Some(item.url.clone()), space, true, None);
        if let Some(t) = self.tab_mut(&tab.id) {
            t.title = item.title.clone();
            t.favicon = item.favicon.clone();
            tab = t.clone();
        }
        Ok(tab)
    }
}

pub fn is_hex_color(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit())
}

pub fn same_url(a: &str, b: &str) -> bool {
    let norm = |u: &str| u.trim().trim_end_matches('/').to_lowercase();
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let dir = std::env::temp_dir().join(format!("serica-test-{}", uuid::Uuid::new_v4()));
        Store::load(dir)
    }

    fn today_ids(s: &Store, space: &str) -> Vec<String> {
        s.list(space, Section::Today).into_iter().map(|i| s.tabs[i].id.clone()).collect()
    }

    #[test]
    fn pinned_site_is_added_unloaded_without_activation() {
        let mut s = store();
        let active = s.active_tab_id.clone();
        let space = s.active_space_id.clone();
        let tab = s
            .add_pinned_site("https://mail.google.com/".into(), "Gmail".into(), None, Some(space.clone()))
            .expect("pinned");
        assert_eq!(tab.section(), Section::Pinned);
        assert!(tab.unloaded);
        assert_eq!(tab.title, "Gmail");
        assert_eq!(s.active_tab_id, active);
        assert!(s.list(&space, Section::Pinned).iter().any(|&i| s.tabs[i].id == tab.id));
    }

    #[test]
    fn leaving_a_tab_marks_it_recently_used() {
        let mut s = store();
        let a = s.create_tab(Some("https://a.com".into()), None, true, None).id;
        s.tab_mut(&a).unwrap().last_active_at = Some(now_ms() - 48 * 3_600_000);
        let b = s.create_tab(Some("https://b.com".into()), None, true, None).id;
        assert_eq!(s.active_tab_id.as_deref(), Some(b.as_str()));
        // The tab just left must not be archived as if idle for two days.
        assert_eq!(s.archive_sweep(), 0);
        assert!(s.tab(&a).is_some());
    }

    #[test]
    fn new_tabs_open_at_the_text_size() {
        let mut s = store();
        s.settings.text_scale = 1.25;
        let tab = s.create_tab(Some("https://a.com".into()), None, false, None);
        assert_eq!(tab.zoom_factor, Some(1.25));
    }

    #[test]
    fn fresh_store_has_one_active_tab() {
        let s = store();
        assert_eq!(s.spaces.len(), 3);
        let active = s.active_tab().expect("active tab");
        assert_eq!(active.space_id, s.active_space_id);
    }

    #[test]
    fn closing_active_tab_selects_neighbor_in_same_space() {
        let mut s = store();
        let space = s.active_space_id.clone();
        let a = s.create_tab(Some("https://a.com".into()), None, true, None).id;
        let b = s.create_tab(Some("https://b.com".into()), None, true, None).id;
        let c = s.create_tab(Some("https://c.com".into()), None, true, None).id;
        // A tab in another space sits between them in the Vec.
        s.create_tab(Some("https://other.com".into()), Some("space-work".into()), false, None);
        s.activate_tab(&b);
        s.close_tab(&b);
        assert_eq!(s.active_tab_id.as_deref(), Some(c.as_str()));
        s.close_tab(&c);
        assert_eq!(s.active_tab_id.as_deref(), Some(a.as_str()));
        assert_eq!(s.active_space_id, space);
        assert_eq!(s.reopen_closed_tab().map(|t| t.url), Some("https://c.com".into()));
    }

    #[test]
    fn closing_last_tab_creates_new_tab() {
        let mut s = store();
        let only = s.active_tab_id.clone().unwrap();
        s.close_tab(&only);
        let active = s.active_tab().unwrap();
        assert_ne!(active.id, only);
        assert_eq!(active.url, NEW_TAB_URL);
        // New-tab pages are not remembered for reopen.
        assert!(s.closed.is_empty());
    }

    #[test]
    fn pinned_close_keeps_entry_unloaded() {
        let mut s = store();
        let t = s.create_tab(Some("https://p.com".into()), None, true, None).id;
        assert!(s.set_pinned(&t, true));
        s.close_tab(&t);
        let tab = s.tab(&t).unwrap();
        assert!(tab.unloaded && tab.is_pinned);
        assert_ne!(s.active_tab_id.as_deref(), Some(t.as_str()));
        s.activate_tab(&t);
        assert!(!s.tab(&t).unwrap().unloaded);
    }

    #[test]
    fn reorder_within_today() {
        let mut s = store();
        let space = s.active_space_id.clone();
        let first = today_ids(&s, &space)[0].clone();
        let a = s.create_tab(Some("https://a.com".into()), None, false, None).id;
        let b = s.create_tab(Some("https://b.com".into()), None, false, None).id;
        s.apply_move(&MoveOp::Reorder { section: "today".into(), id: b.clone(), to_index: 0, folder_id: None });
        assert_eq!(today_ids(&s, &space), vec![b, first, a]);
    }

    #[test]
    fn drag_into_and_out_of_folders() {
        let mut s = store();
        let space = s.active_space_id.clone();
        let f = s.create_folder(&space, "Docs").unwrap().id;
        let g = s.create_folder(&space, "Other").unwrap().id;
        // A Today tab dropped on a folder becomes a pinned tab inside it.
        let t = s.create_tab(Some("https://a.com".into()), None, false, None).id;
        let op: MoveOp = serde_json::from_value(serde_json::json!({
            "type": "section", "id": t, "from": "today", "to": "pinned", "spaceId": space, "toIndex": 0, "folderId": f
        }))
        .unwrap();
        assert!(s.apply_move(&op).is_some());
        assert!(s.tab(&t).unwrap().is_pinned);
        assert_eq!(s.tab(&t).unwrap().folder_id.as_deref(), Some(f.as_str()));
        // Pinned reorder into another folder, then to the root (null).
        let op: MoveOp = serde_json::from_value(serde_json::json!({
            "type": "reorder", "section": "pinned", "id": t, "toIndex": 0, "folderId": g
        }))
        .unwrap();
        s.apply_move(&op);
        assert_eq!(s.tab(&t).unwrap().folder_id.as_deref(), Some(g.as_str()));
        let op: MoveOp = serde_json::from_value(serde_json::json!({
            "type": "reorder", "section": "pinned", "id": t, "toIndex": 0, "folderId": null
        }))
        .unwrap();
        s.apply_move(&op);
        assert_eq!(s.tab(&t).unwrap().folder_id, None);
        // Without folderId a reorder keeps the folder.
        s.move_tab_to_folder(&t, Some(&f));
        let op: MoveOp = serde_json::from_value(serde_json::json!({
            "type": "reorder", "section": "pinned", "id": t, "toIndex": 0
        }))
        .unwrap();
        s.apply_move(&op);
        assert_eq!(s.tab(&t).unwrap().folder_id.as_deref(), Some(f.as_str()));
        // A folder of another space is ignored.
        let other = s.create_folder("space-work", "Elsewhere").unwrap().id;
        let op: MoveOp = serde_json::from_value(serde_json::json!({
            "type": "reorder", "section": "pinned", "id": t, "toIndex": 0, "folderId": other
        }))
        .unwrap();
        s.apply_move(&op);
        assert_eq!(s.tab(&t).unwrap().folder_id.as_deref(), Some(f.as_str()));
    }

    #[test]
    fn move_to_other_space_and_favorites() {
        let mut s = store();
        let t = s.create_tab(Some("https://x.com".into()), None, true, None).id;
        let r = s.apply_move(&MoveOp::Space { id: t.clone(), from: "today".into(), target_space_id: "space-work".into() });
        assert_eq!(r, Some(false));
        assert_eq!(s.tab(&t).unwrap().space_id, "space-work");
        // The moved tab was active; the user stays in the original space.
        assert_ne!(s.active_tab_id.as_deref(), Some(t.as_str()));
        assert!(s.add_favorite(&t));
        assert!(!s.add_favorite(&t) || s.favorites.len() == 1);
        assert_eq!(s.favorites.len(), 1);
        let fid = s.favorites[0].id.clone();
        assert!(s.remove_favorite(&fid));
        assert!(s.favorites.is_empty());
    }

    #[test]
    fn delete_last_space_of_profile_is_rejected() {
        let mut s = store();
        assert!(s.delete_space("space-work").is_ok());
        assert!(s.delete_space("space-leisure").is_ok());
        assert_eq!(s.delete_space("space-study"), Err("last-space"));
    }

    #[test]
    fn private_tabs_leave_nothing_behind() {
        let mut s = store();
        let normal = s.create_tab(Some("https://normal.com".into()), None, true, None).id;
        let p = s.create_private_tab(Some("https://secret.example".into()));
        assert!(p.private);
        assert_eq!(s.active_tab_id.as_deref(), Some(p.id.as_str()));
        // Never written to disk, not even as the active tab.
        let files = s.serialized();
        let state = String::from_utf8(files.iter().find(|(f, _)| f.ends_with("state.json")).unwrap().1.clone()).unwrap();
        assert!(!state.contains("secret.example"));
        assert!(!state.contains(&p.id));
        assert!(state.contains("normal.com"));
        // Can't be pinned or kept as a favorite.
        assert!(!s.set_pinned(&p.id, true));
        assert!(!s.add_favorite(&p.id));
        // Never slept or archived, even when idle for days.
        s.activate_tab(&normal);
        s.tab_mut(&p.id).unwrap().last_active_at = Some(now_ms() - 7 * 24 * 3_600_000);
        assert_eq!(s.sleep_inactive(60_000), 0);
        assert!(!s.tab(&p.id).unwrap().is_sleeping);
        // Not offered again after closing.
        s.close_tab(&p.id);
        assert!(s.tab(&p.id).is_none());
        let reopened = s.reopen_closed_tab();
        assert!(reopened.map_or(true, |t| !t.url.contains("secret.example")));
    }

    #[test]
    fn zen_sleep_respects_idle_threshold() {
        let mut s = store();
        let a = s.create_tab(Some("https://a.com".into()), None, false, None).id;
        let b = s.create_tab(Some("https://b.com".into()), None, false, None).id;
        s.tab_mut(&a).unwrap().last_active_at = Some(now_ms() - 3_600_000);
        assert_eq!(s.sleep_inactive(600_000), 1);
        assert!(s.tab(&a).unwrap().is_sleeping);
        assert!(!s.tab(&b).unwrap().is_sleeping);
    }

    #[test]
    fn switching_spaces_marks_both_tabs_in_use() {
        let mut s = store();
        s.activate_space("space-study");
        let study = s.create_tab(Some("https://study.com".into()), Some("space-study".into()), true, None).id;
        let work = s.create_tab(Some("https://work.com".into()), Some("space-work".into()), false, None).id;
        // Both last picked directly long ago, then used only through space switches.
        s.tab_mut(&study).unwrap().last_active_at = Some(now_ms() - 3 * 3_600_000);
        s.tab_mut(&work).unwrap().last_active_at = Some(now_ms() - 3 * 3_600_000);
        s.activate_space("space-work");
        assert_eq!(s.active_tab_id.as_deref(), Some(work.as_str()));
        // The tab just left was in use until now: it must not sleep right away.
        assert_eq!(s.sleep_inactive(60 * 60_000), 0);
        assert!(!s.tab(&study).unwrap().is_sleeping);
        s.activate_space("space-study");
        assert_eq!(s.sleep_least_recent(60_000, 5), 0);
        assert!(!s.tab(&work).unwrap().is_sleeping);
    }

    #[test]
    fn memory_pressure_sleeps_least_recently_used_first() {
        let mut s = store();
        let old = s.create_tab(Some("https://old.com".into()), None, false, None).id;
        let older = s.create_tab(Some("https://older.com".into()), None, false, None).id;
        let fresh = s.create_tab(Some("https://fresh.com".into()), None, false, None).id;
        let active = s.create_tab(Some("https://active.com".into()), None, true, None).id;
        s.tab_mut(&old).unwrap().last_active_at = Some(now_ms() - 20 * 60_000);
        s.tab_mut(&older).unwrap().last_active_at = Some(now_ms() - 40 * 60_000);
        s.tab_mut(&fresh).unwrap().last_active_at = Some(now_ms() - 60_000);
        s.tab_mut(&active).unwrap().last_active_at = Some(now_ms() - 90 * 60_000);
        // One at a time, oldest first; the active tab and recent tabs are spared.
        assert_eq!(s.sleep_least_recent(5 * 60_000, 1), 1);
        assert!(s.tab(&older).unwrap().is_sleeping);
        assert!(!s.tab(&old).unwrap().is_sleeping);
        assert_eq!(s.sleep_least_recent(5 * 60_000, 5), 1);
        assert!(s.tab(&old).unwrap().is_sleeping);
        assert!(!s.tab(&fresh).unwrap().is_sleeping);
        assert!(!s.tab(&active).unwrap().is_sleeping);
        assert_eq!(s.sleep_least_recent(5 * 60_000, 5), 0);
    }

    #[test]
    fn archive_sweep_moves_old_today_tabs() {
        let mut s = store();
        let a = s.create_tab(Some("https://old.com".into()), None, false, None).id;
        s.tab_mut(&a).unwrap().last_active_at = Some(now_ms() - 13 * 3_600_000);
        assert_eq!(s.archive_sweep(), 1);
        assert!(s.tab(&a).is_none());
        let list = s.archive_list();
        assert_eq!(list.len(), 1);
        let id = list[0]["id"].as_str().unwrap().to_string();
        let restored = s.restore_archived(&id).unwrap();
        assert_eq!(restored.url, "https://old.com");
    }

    #[test]
    fn state_round_trips_through_disk() {
        let mut s = store();
        let t = s.create_tab(Some("https://persist.me".into()), None, true, None).id;
        s.settings.apply_patch(&serde_json::json!({"sidebarWidth": 333}));
        s.save();
        let again = Store::load(s.data_dir.clone());
        assert_eq!(again.active_tab_id.as_deref(), Some(t.as_str()));
        assert_eq!(again.settings.sidebar_width, 333.0);
    }

    #[test]
    fn cycle_wraps() {
        let mut s = store();
        let first = s.active_tab_id.clone().unwrap();
        let b = s.create_tab(Some("https://b.com".into()), None, true, None).id;
        assert_eq!(s.cycle_tab(1).unwrap().id, first);
        assert_eq!(s.cycle_tab(-1).unwrap().id, b);
    }
}
