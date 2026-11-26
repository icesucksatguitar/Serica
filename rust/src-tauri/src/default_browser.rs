//! Making Serica the Windows default browser.
//!
//! Windows 10 and 11 don't let an app make itself the default: it registers
//! its capabilities, and the user confirms in Settings › Default apps. Serica
//! registers for the current user (HKCU, no administrator prompt) as a
//! "StartMenuInternet" client that handles http, https and web files, then
//! opens Settings right at Serica's page, where one click makes it default.

#[cfg(windows)]
mod imp {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegGetValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
    };
    use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

    pub const PROG_ID: &str = "SericaHTML";
    const CLIENT: &str = r"Software\Clients\StartMenuInternet\Serica";

    fn set(path: &str, name: Option<&str>, value: &str) -> Result<(), String> {
        unsafe {
            let mut key = HKEY::default();
            let r = RegCreateKeyExW(HKEY_CURRENT_USER, &HSTRING::from(path), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None);
            if r != ERROR_SUCCESS {
                return Err(format!("registry: {path}: {r:?}"));
            }
            let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
            let name_h = name.map(HSTRING::from);
            let r = RegSetValueExW(key, name_h.as_ref().map(|h| PCWSTR(h.as_ptr())).unwrap_or(PCWSTR::null()), None, REG_SZ, Some(bytes));
            let _ = RegCloseKey(key);
            if r != ERROR_SUCCESS {
                return Err(format!("registry value: {path}: {r:?}"));
            }
        }
        Ok(())
    }

    fn get(path: &str, name: &str) -> Option<String> {
        unsafe {
            let mut buf = vec![0u16; 512];
            let mut size = (buf.len() * 2) as u32;
            let r = RegGetValueW(HKEY_CURRENT_USER, &HSTRING::from(path), &HSTRING::from(name), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size));
            if r != ERROR_SUCCESS {
                return None;
            }
            let len = (size as usize / 2).saturating_sub(1);
            Some(String::from_utf16_lossy(&buf[..len.min(buf.len())]))
        }
    }

    /// Registers Serica (current user) as a browser that can be the default.
    pub fn register() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe = exe.to_string_lossy().to_string();
        let icon = format!("{exe},0");
        let open = format!("\"{exe}\" \"%1\"");
        let classes = format!(r"Software\Classes\{PROG_ID}");
        set(&classes, None, "Serica HTML Document")?;
        set(&format!(r"{classes}\DefaultIcon"), None, &icon)?;
        set(&format!(r"{classes}\shell\open\command"), None, &open)?;
        set(&format!(r"{classes}\Application"), Some("ApplicationName"), "Serica")?;
        set(&format!(r"{classes}\Application"), Some("ApplicationIcon"), &icon)?;
        set(&format!(r"{classes}\Application"), Some("ApplicationDescription"), "A calm, fast browser for Windows")?;

        set(CLIENT, None, "Serica")?;
        set(&format!(r"{CLIENT}\DefaultIcon"), None, &icon)?;
        set(&format!(r"{CLIENT}\shell\open\command"), None, &format!("\"{exe}\""))?;
        let caps = format!(r"{CLIENT}\Capabilities");
        set(&caps, Some("ApplicationName"), "Serica")?;
        set(&caps, Some("ApplicationIcon"), &icon)?;
        set(&caps, Some("ApplicationDescription"), "A calm, fast browser for Windows")?;
        set(&format!(r"{caps}\StartMenu"), Some("StartMenuInternet"), "Serica")?;
        for scheme in ["http", "https"] {
            set(&format!(r"{caps}\URLAssociations"), Some(scheme), PROG_ID)?;
        }
        for ext in [".htm", ".html", ".shtml", ".xht", ".xhtml", ".svg", ".webp", ".pdf"] {
            set(&format!(r"{caps}\FileAssociations"), Some(ext), PROG_ID)?;
        }
        set(r"Software\RegisteredApplications", Some("Serica"), &format!(r"{CLIENT}\Capabilities"))?;
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
        Ok(())
    }

    /// Whether Windows sends https links to Serica.
    pub fn is_default() -> bool {
        let base = r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https";
        // Recent Windows 11 builds keep the choice under UserChoiceLatest.
        [format!(r"{base}\UserChoiceLatest"), format!(r"{base}\UserChoice")]
            .iter()
            .filter_map(|k| get(k, "ProgId"))
            .next()
            .is_some_and(|p| p.eq_ignore_ascii_case(PROG_ID))
    }

    /// Opens Settings at Serica's default-apps page.
    pub fn open_settings() -> Result<(), String> {
        // Windows 11 jumps straight to the app's page; older builds show the list.
        let deep = "ms-settings:defaultapps?registeredAppUser=Serica";
        open::that_detached(deep).or_else(|_| open::that_detached("ms-settings:defaultapps")).map_err(|e| e.to_string())
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultBrowserStatus {
    pub is_default: bool,
    pub supported: bool,
}

#[tauri::command]
pub fn default_browser_status() -> DefaultBrowserStatus {
    #[cfg(windows)]
    return DefaultBrowserStatus { is_default: imp::is_default(), supported: true };
    #[cfg(not(windows))]
    DefaultBrowserStatus { is_default: false, supported: false }
}

/// Registers Serica and opens the system's default-apps settings; the UI
/// polls `default_browser_status` to see the user's choice land.
#[tauri::command]
pub async fn make_default_browser() -> Result<DefaultBrowserStatus, String> {
    #[cfg(windows)]
    {
        imp::register()?;
        if !imp::is_default() {
            imp::open_settings()?;
        }
        return Ok(DefaultBrowserStatus { is_default: imp::is_default(), supported: true });
    }
    #[cfg(not(windows))]
    Err("unsupported".into())
}
