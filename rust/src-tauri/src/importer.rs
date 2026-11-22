//! One-time, read-only import of the Electron build's profile on first launch.
//!
//! Electron keeps settings in `neutrino-data/browser-state.json` and the
//! sidebar model (profiles, spaces, tabs, favorites, folders, archive) in
//! `neutrino-data/profiles.json`. Both shapes are field-compatible with the
//! Rust model, so they are handed to `Store::load` as raw JSON.

use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct ImportedData {
    pub settings: Option<Value>,
    pub state: Option<Value>,
}

pub fn try_import_electron_data() -> Option<ImportedData> {
    candidate_dirs().into_iter().find_map(|d| import_from(&d))
}

fn read(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn import_from(dir: &Path) -> Option<ImportedData> {
    let data = dir.join("neutrino-data");
    let browser_state = read(&data.join("browser-state.json"));
    let profiles = read(&data.join("profiles.json"));
    if browser_state.is_none() && profiles.is_none() {
        return None;
    }
    tracing::info!("importing Electron data from {:?}", data);

    let settings = browser_state.as_ref().and_then(|v| v.get("settings").cloned());
    let mut state = serde_json::Map::new();
    if let Some(Value::Object(p)) = profiles {
        for key in ["profiles", "spaces", "tabs", "favorites", "folders", "archive"] {
            if let Some(v) = p.get(key) {
                state.insert(key.to_string(), v.clone());
            }
        }
    } else if let Some(Value::Object(b)) = &browser_state {
        for key in ["spaces", "tabs"] {
            if let Some(v) = b.get(key) {
                state.insert(key.to_string(), v.clone());
            }
        }
    }
    let active = settings
        .as_ref()
        .and_then(|s| s.get("activeSpaceId"))
        .or_else(|| browser_state.as_ref().and_then(|b| b.get("activeSpaceId")))
        .cloned();
    if let Some(a) = active {
        state.insert("activeSpaceId".into(), a);
    }
    Some(ImportedData { settings, state: if state.is_empty() { None } else { Some(Value::Object(state)) } })
}

fn candidate_dirs() -> Vec<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.map(|b| ["serica-browser", "serica", "Serica", "neutrino-browser", "neutrino", "Neutrino", "com.serica.browser", "com.neutrino.browser"].iter().map(|n| b.join(n)).collect())
        .unwrap_or_default()
}
