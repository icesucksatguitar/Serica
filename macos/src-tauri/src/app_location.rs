//! Where Serica.app runs from, and moving it to Applications.
//!
//! An app opened straight from its disk image, or from Downloads without
//! being moved by Finder first, is run by macOS from a hidden read-only copy
//! ("App Translocation": `/private/var/folders/…/AppTranslocation/…`). The
//! updater can't replace an app there ("Read-only file system"). Serica
//! notices this, offers to move itself into Applications (as most Mac apps
//! do), and relaunches from there, where updates install normally.

use std::ffi::{c_char, c_void, CString};
use std::path::{Path, PathBuf};
use std::process::Command;

/// `Serica.app`, from the running executable (`…/Serica.app/Contents/MacOS/serica`).
pub fn bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let app = exe.parent()?.parent()?.parent()?.to_path_buf();
    (app.extension().is_some_and(|e| e == "app")).then_some(app)
}

fn is_translocated(p: &Path) -> bool {
    p.to_string_lossy().contains("/AppTranslocation/")
}

fn on_disk_image(p: &Path) -> bool {
    p.starts_with("/Volumes")
}

/// Running where updates can't be installed: a translocated copy or a disk image.
pub fn needs_move() -> bool {
    bundle().is_some_and(|b| is_translocated(&b) || on_disk_image(&b))
}

/// The app's real location behind a translocated copy (Security framework's
/// `SecTranslocateCreateOriginalPathForURL`, which Safari and others use).
fn original_of_translocated(p: &Path) -> Option<PathBuf> {
    use objc2::rc::Retained;
    use objc2_foundation::{NSString, NSURL};
    type Original = unsafe extern "C" fn(*const c_void, *mut *mut c_void) -> *mut c_void;
    extern "C" {
        fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    let lib = CString::new("/System/Library/Frameworks/Security.framework/Security").ok()?;
    let sym = CString::new("SecTranslocateCreateOriginalPathForURL").ok()?;
    unsafe {
        let h = dlopen(lib.as_ptr(), 1 /* RTLD_LAZY */);
        if h.is_null() {
            return None;
        }
        let f = dlsym(h, sym.as_ptr());
        if f.is_null() {
            return None;
        }
        let f: Original = std::mem::transmute(f);
        let url = NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()));
        // NSURL and CFURL are toll-free bridged; the result is owned (+1).
        let out = f(Retained::as_ptr(&url) as *const c_void, std::ptr::null_mut());
        if out.is_null() {
            return None;
        }
        let out: Retained<NSURL> = Retained::from_raw(out as *mut NSURL)?;
        out.path().map(|s| PathBuf::from(s.to_string()))
    }
}

/// Where the app should go: /Applications, or ~/Applications when the user
/// can't write to /Applications.
fn applications_dir() -> PathBuf {
    let system = PathBuf::from("/Applications");
    let probe = system.join(format!(".serica-write-test-{}", std::process::id()));
    if std::fs::write(&probe, b"").is_ok() {
        let _ = std::fs::remove_file(&probe);
        return system;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    let user = home.join("Applications");
    let _ = std::fs::create_dir_all(&user);
    user
}

/// Copies `src` (an .app) into `dir`, replacing an older copy there, and
/// clears the download quarantine so macOS doesn't translocate it again.
/// Returns the new path.
fn copy_bundle(src: &Path, dir: &Path) -> Result<PathBuf, String> {
    let name = src.file_name().ok_or("no app name")?;
    let dest = dir.join(name);
    let tmp = dir.join(format!(".{}.moving-{}", name.to_string_lossy(), std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let ok = Command::new("/usr/bin/ditto").arg(src).arg(&tmp).status().map_err(|e| e.to_string())?;
    if !ok.success() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("couldn't copy Serica to {}", dir.display()));
    }
    let _ = Command::new("/usr/bin/xattr").args(["-dr", "com.apple.quarantine"]).arg(&tmp).status();
    if dest.exists() {
        std::fs::remove_dir_all(&dest).map_err(|e| format!("couldn't replace the old Serica in {}: {e}", dir.display()))?;
    }
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    Ok(dest)
}

/// Moves the running app into Applications and relaunches it from there.
/// The download in Downloads goes to the Trash (a disk image is left alone).
pub fn move_to_applications(app: &tauri::AppHandle) -> Result<(), String> {
    let running = bundle().ok_or("Serica isn't running from an app bundle")?;
    let src = if is_translocated(&running) { original_of_translocated(&running).ok_or("couldn't find where Serica was downloaded to")? } else { running.clone() };
    if !src.exists() {
        return Err("couldn't find where Serica was downloaded to".into());
    }
    let dest = copy_bundle(&src, &applications_dir())?;
    tracing::info!("moved Serica to {}", dest.display());
    if !on_disk_image(&src) && src != dest {
        let trashed = Command::new("/usr/bin/osascript")
            .args(["-e", &format!("tell application \"Finder\" to delete POSIX file {:?}", src.to_string_lossy())])
            .status()
            .is_ok_and(|s| s.success());
        if !trashed {
            let _ = std::fs::remove_dir_all(&src);
        }
    }
    // Reopen from Applications once this process has quit.
    let script = format!("while kill -0 {} 2>/dev/null; do sleep 0.2; done; /usr/bin/open {:?}", std::process::id(), dest.to_string_lossy());
    Command::new("/bin/sh").args(["-c", &script]).spawn().map_err(|e| e.to_string())?;
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_translocated_and_disk_image_paths() {
        assert!(is_translocated(Path::new("/private/var/folders/v5/x/T/AppTranslocation/C088FF15/d/Serica.app")));
        assert!(!is_translocated(Path::new("/Applications/Serica.app")));
        assert!(on_disk_image(Path::new("/Volumes/Serica/Serica.app")));
        assert!(!on_disk_image(Path::new("/Applications/Serica.app")));
    }

    #[test]
    fn asks_macos_for_the_original_path_without_crashing() {
        // A path that isn't translocated: macOS answers with it (or nothing).
        let p = std::env::temp_dir();
        if let Some(o) = original_of_translocated(&p) {
            assert!(!o.as_os_str().is_empty());
        }
    }

    #[test]
    fn copies_and_replaces_a_bundle() {
        let root = std::env::temp_dir().join(format!("serica-move-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let src = root.join("dl/Serica.app/Contents/MacOS");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("serica"), b"new").unwrap();
        let apps = root.join("Applications");
        std::fs::create_dir_all(apps.join("Serica.app/Contents")).unwrap();
        std::fs::write(apps.join("Serica.app/Contents/old"), b"old").unwrap();
        let dest = copy_bundle(&root.join("dl/Serica.app"), &apps).unwrap();
        assert_eq!(dest, apps.join("Serica.app"));
        assert_eq!(std::fs::read(dest.join("Contents/MacOS/serica")).unwrap(), b"new");
        assert!(!dest.join("Contents/old").exists(), "the old copy is replaced");
        let _ = std::fs::remove_dir_all(&root);
    }
}
