//! Automatic updates from GitHub releases.
//!
//! A release build checks the `latest.json` published with the newest GitHub
//! release shortly after launch and then every two hours. A newer version is
//! downloaded in the background and verified against the public key in
//! tauri.conf.json (the plugin rejects packages not signed with our key); the
//! UI then offers "Restart to update", which saves the session and runs the
//! installer in passive mode (a progress bar, no questions). Serica reopens
//! after the installer finishes. If the user just closes Serica instead, a
//! downloaded update is installed as it quits (see `install_on_exit`), so the
//! next launch is already the new version.

use crate::app::{ui_emit, Shared};
use parking_lot::Mutex;
use serde::Serialize;
use std::sync::OnceLock;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

const FIRST_CHECK_DELAY: Duration = Duration::from_secs(20);
const CHECK_INTERVAL: Duration = Duration::from_secs(2 * 60 * 60);
const EVENT: &str = "serica:update-status";

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// idle | checking | up-to-date | downloading | ready | installing | error
    pub state: String,
    pub current_version: String,
    pub new_version: Option<String>,
    pub notes: Option<String>,
    /// 0..1 while downloading, when the size is known.
    pub progress: Option<f64>,
    pub error: Option<String>,
    /// Unix ms of the last completed check.
    pub last_checked: Option<i64>,
}

struct Inner {
    status: UpdateStatus,
    ready: Option<(Update, Vec<u8>)>,
    busy: bool,
}

fn inner() -> &'static Mutex<Inner> {
    static I: OnceLock<Mutex<Inner>> = OnceLock::new();
    I.get_or_init(|| {
        Mutex::new(Inner {
            status: UpdateStatus { state: "idle".into(), ..Default::default() },
            ready: None,
            busy: false,
        })
    })
}

fn publish(handle: &AppHandle, f: impl FnOnce(&mut UpdateStatus)) {
    let snapshot = {
        let mut g = inner().lock();
        f(&mut g.status);
        g.status.clone()
    };
    ui_emit(handle, EVENT, snapshot);
}

pub fn status(handle: &AppHandle) -> UpdateStatus {
    let mut s = inner().lock().status.clone();
    s.current_version = handle.package_info().version.to_string();
    s
}

/// Starts the background check loop (release builds only).
pub fn start(handle: AppHandle) {
    {
        let v = handle.package_info().version.to_string();
        inner().lock().status.current_version = v;
    }
    if cfg!(debug_assertions) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_DELAY).await;
        loop {
            let _ = check(&handle).await;
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

/// Checks for an update and, if there is one, downloads it. Returns the
/// resulting status. Concurrent calls return the current status at once.
pub async fn check(handle: &AppHandle) -> UpdateStatus {
    {
        let mut g = inner().lock();
        if g.busy || g.ready.is_some() {
            return g.status.clone();
        }
        g.busy = true;
    }
    let result = check_inner(handle).await;
    inner().lock().busy = false;
    if let Err(e) = result {
        tracing::warn!("update check failed: {e}");
        publish(handle, |s| {
            s.state = "error".into();
            s.error = Some(e);
            s.progress = None;
        });
    }
    status(handle)
}

async fn check_inner(handle: &AppHandle) -> Result<(), String> {
    publish(handle, |s| {
        s.state = "checking".into();
        s.error = None;
    });
    let updater = handle
        .updater_builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let found = updater.check().await.map_err(|e| format!("Couldn't check for updates: {e}"))?;
    let now = chrono::Utc::now().timestamp_millis();
    let Some(update) = found else {
        publish(handle, |s| {
            s.state = "up-to-date".into();
            s.new_version = None;
            s.notes = None;
            s.last_checked = Some(now);
        });
        return Ok(());
    };

    let version = update.version.clone();
    tracing::info!("downloading Serica {version}");
    publish(handle, |s| {
        s.state = "downloading".into();
        s.new_version = Some(version.clone());
        s.notes = update.body.clone();
        s.progress = Some(0.0);
        s.last_checked = Some(now);
    });

    let h = handle.clone();
    let mut got: u64 = 0;
    let mut last_pct = -1i64;
    let bytes = update
        .download(
            move |chunk, total| {
                got += chunk as u64;
                if let Some(total) = total.filter(|t| *t > 0) {
                    let p = (got as f64 / total as f64).min(1.0);
                    let pct = (p * 100.0) as i64;
                    if pct != last_pct {
                        last_pct = pct;
                        publish(&h, |s| s.progress = Some(p));
                    }
                }
            },
            || {},
        )
        .await
        .map_err(|e| format!("Couldn't download Serica {version}: {e}"))?;

    inner().lock().ready = Some((update, bytes));
    publish(handle, |s| {
        s.state = "ready".into();
        s.progress = None;
    });
    Ok(())
}

/// Called as Serica quits (after the session is saved): if an update has
/// been downloaded, the installer runs now and does not reopen Serica, so
/// the next launch is already the new version. Nothing happens otherwise.
/// On Windows this does not return when it starts the installer.
pub fn install_on_exit() {
    let Some((update, bytes)) = inner().lock().ready.take() else {
        return;
    };
    tracing::info!("installing Serica {} on exit", update.version);
    if let Err(e) = update.restart_after_install(false).install(bytes) {
        tracing::warn!("couldn't install the update on exit: {e}");
    }
}

/// Saves the session and runs the downloaded installer; the app exits.
pub fn install(handle: &AppHandle) -> Result<(), String> {
    let Some((update, bytes)) = inner().lock().ready.take() else {
        return Err("No update has been downloaded yet".into());
    };
    publish(handle, |s| s.state = "installing".into());
    if let Some(shared) = handle.try_state::<Shared>() {
        shared.save_now();
    }
    // On Windows this launches the installer and exits the process.
    if let Err(e) = update.install(bytes) {
        let msg = format!("Couldn't start the installer: {e}");
        publish(handle, |s| {
            s.state = "error".into();
            s.error = Some(msg.clone());
        });
        return Err(msg);
    }
    handle.restart();
}
