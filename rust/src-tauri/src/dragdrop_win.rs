//! Drag and drop onto Serica's own interface (sidebar, tab strip, new-tab
//! page). Pages handle drops themselves (see `TabHost::builder`).
//!
//! Tauri's handler only understands files, so a link dragged out of a page
//! onto the sidebar was refused. This target accepts files, links and text,
//! and reports the drag to the UI (`serica:drag` events, positions in
//! physical client px of the main window), which shows where it will land
//! and decides what the drop does.

#![allow(non_snake_case)]

use crate::app::{ui_emit, Shared};
use std::cell::{Cell, RefCell};
use windows::core::{implement, BOOL};
use windows::Win32::Foundation::{DRAGDROP_E_INVALIDHWND, HGLOBAL, HWND, LPARAM, POINT, POINTL};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{
    IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop, CF_HDROP, CF_UNICODETEXT, DROPEFFECT,
    DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, GetClientRect};

/// What is being dragged.
#[derive(Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DragData {
    /// "files" | "url" | "text"
    pub kind: &'static str,
    pub paths: Vec<String>,
    pub url: Option<String>,
    pub text: Option<String>,
}

unsafe fn global_bytes(h: HGLOBAL) -> Vec<u8> {
    let p = GlobalLock(h);
    if p.is_null() {
        return Vec::new();
    }
    let n = GlobalSize(h);
    let out = std::slice::from_raw_parts(p as *const u8, n).to_vec();
    let _ = GlobalUnlock(h);
    out
}

fn utf16_until_nul(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
    String::from_utf16_lossy(&units)
}

unsafe fn get_format(data: &IDataObject, format: u16) -> Option<Vec<u8>> {
    let fmt = FORMATETC { cfFormat: format, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
    let mut medium = data.GetData(&fmt).ok()?;
    let bytes = global_bytes(medium.u.hGlobal);
    ReleaseStgMedium(&mut medium);
    Some(bytes)
}

unsafe fn read(data: &IDataObject) -> Option<DragData> {
    // Files from Explorer (and anything else that offers paths).
    let fmt = FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
    if let Ok(mut medium) = data.GetData(&fmt) {
        let hdrop = HDROP(medium.u.hGlobal.0);
        let n = DragQueryFileW(hdrop, u32::MAX, None);
        let mut paths = Vec::new();
        for i in 0..n.min(50) {
            let len = DragQueryFileW(hdrop, i, None) as usize;
            let mut buf = vec![0u16; len + 1];
            DragQueryFileW(hdrop, i, Some(&mut buf));
            paths.push(String::from_utf16_lossy(&buf[..len]));
        }
        ReleaseStgMedium(&mut medium);
        if !paths.is_empty() {
            return Some(DragData { kind: "files", paths, ..Default::default() });
        }
    }
    // A link (from a page, or a browser's address bar).
    let url_fmt = RegisterClipboardFormatW(windows::core::w!("UniformResourceLocatorW"));
    if url_fmt != 0 {
        if let Some(b) = get_format(data, url_fmt as u16) {
            let url = utf16_until_nul(&b).trim().to_string();
            if !url.is_empty() {
                return Some(DragData { kind: "url", url: Some(url), ..Default::default() });
            }
        }
    }
    // Plain text: a search, or a URL written out.
    if let Some(b) = get_format(data, CF_UNICODETEXT.0) {
        let text: String = utf16_until_nul(&b).trim().chars().take(2000).collect();
        if !text.is_empty() {
            let looks_url = !text.contains(char::is_whitespace) && (text.starts_with("http://") || text.starts_with("https://"));
            return Some(if looks_url {
                DragData { kind: "url", url: Some(text), ..Default::default() }
            } else {
                DragData { kind: "text", text: Some(text), ..Default::default() }
            });
        }
    }
    None
}

#[implement(IDropTarget)]
struct Target {
    hwnd: HWND,
    app: Shared,
    data: RefCell<Option<DragData>>,
    effect: Cell<DROPEFFECT>,
}

impl Target {
    /// Client position in physical px plus the client width (the UI derives
    /// its CSS px from the width, like `ui_cursor`).
    fn position(&self, pt: &POINTL) -> (i32, i32, i32) {
        let mut p = POINT { x: pt.x, y: pt.y };
        let mut rc = Default::default();
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut p);
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        (p.x, p.y, rc.right)
    }

    fn emit(&self, phase: &str, pt: Option<&POINTL>) {
        let (x, y, w) = pt.map(|p| self.position(p)).unwrap_or((0, 0, 0));
        let data = self.data.borrow().clone();
        ui_emit(&self.app.handle, "serica:drag", serde_json::json!({ "phase": phase, "x": x, "y": y, "clientWidth": w, "data": data }));
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: windows::core::Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        let read = data.as_ref().and_then(|d| unsafe { read(d) });
        let e = match read.as_ref().map(|d| d.kind) {
            Some("files") => DROPEFFECT_COPY,
            Some(_) => DROPEFFECT_LINK | DROPEFFECT_COPY,
            None => DROPEFFECT_NONE,
        };
        let valid = read.is_some();
        *self.data.borrow_mut() = read;
        self.effect.set(e);
        unsafe { *effect = pick(e, *effect) };
        if valid {
            self.emit("enter", Some(pt));
        }
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        unsafe { *effect = pick(self.effect.get(), *effect) };
        if self.data.borrow().is_some() {
            self.emit("over", Some(pt));
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        if self.data.borrow().is_some() {
            self.emit("leave", None);
        }
        *self.data.borrow_mut() = None;
        Ok(())
    }

    fn Drop(&self, data: windows::core::Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        // Read again: some sources only render their data on drop.
        let fresh = data.as_ref().and_then(|d| unsafe { read(d) });
        if fresh.is_some() {
            *self.data.borrow_mut() = fresh;
        }
        unsafe { *effect = pick(self.effect.get(), *effect) };
        if self.data.borrow().is_some() {
            self.emit("drop", Some(pt));
        }
        *self.data.borrow_mut() = None;
        Ok(())
    }
}

/// The effect to show: ours, narrowed to what the source allows.
fn pick(ours: DROPEFFECT, allowed: DROPEFFECT) -> DROPEFFECT {
    let both = DROPEFFECT(ours.0 & allowed.0);
    if both.0 & DROPEFFECT_COPY.0 != 0 {
        DROPEFFECT_COPY
    } else if both.0 & DROPEFFECT_LINK.0 != 0 {
        DROPEFFECT_LINK
    } else {
        DROPEFFECT_NONE
    }
}

thread_local! {
    /// Registered targets, kept alive for the window's life.
    static TARGETS: RefCell<Vec<IDropTarget>> = RefCell::new(Vec::new());
}

/// Puts Serica's drop target on the main UI webview's windows. Runs once,
/// on the UI thread, after the webview exists.
pub fn install(app: &Shared, main: &tauri::Webview) {
    let app = app.clone();
    let _ = main.with_webview(move |pw| unsafe {
        let controller = pw.controller();
        // The engine's own drop handling would navigate the UI to a dropped file.
        if let Ok(c4) = windows::core::Interface::cast::<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller4>(&controller) {
            let _ = c4.SetAllowExternalDrop(false);
        }
        let mut parent = HWND::default();
        if controller.ParentWindow(&mut parent).is_err() || parent.0.is_null() {
            return;
        }
        let main_hwnd = parent;
        let mut hwnds: Vec<HWND> = vec![parent];
        unsafe extern "system" fn collect(h: HWND, l: LPARAM) -> BOOL {
            let list = &mut *(l.0 as *mut Vec<HWND>);
            list.push(h);
            BOOL(1)
        }
        let _ = EnumChildWindows(Some(parent), Some(collect), LPARAM(&mut hwnds as *mut Vec<HWND> as isize));
        for h in hwnds {
            let target: IDropTarget = Target { hwnd: main_hwnd, app: app.clone(), data: RefCell::new(None), effect: Cell::new(DROPEFFECT_NONE) }.into();
            if RevokeDragDrop(h) != Err(DRAGDROP_E_INVALIDHWND.into()) && RegisterDragDrop(h, &target).is_ok() {
                TARGETS.with(|t| t.borrow_mut().push(target));
            }
        }
    });
}

/// Drags files out of Serica (finished downloads) as a normal Windows file
/// drag: drop them on the desktop or a folder to copy them, into a web page
/// to upload them, or anywhere else that takes files.
///
/// The drag runs on its own OLE thread, not the main thread: a drag is a
/// modal loop, and while it ran on the main thread nothing else in the app
/// moved (the Downloads panel could not close, so the page under it stayed
/// hidden and could not take the drop).
pub fn start_file_drag(app: &Shared, paths: Vec<std::path::PathBuf>) {
    use tauri::Manager;
    use windows::core::HSTRING;
    use windows::Win32::System::Com::{CoTaskMemFree, IBindCtx};
    use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{SHCreateDataObject, SHDoDragDrop, SHParseDisplayName};
    if paths.is_empty() {
        return;
    }
    let hwnd = app.handle.get_window("main").and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize).unwrap_or(0);
    let _ = std::thread::Builder::new().name("serica-drag".into()).spawn(move || unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};
        // Only while the button is still held: a drag started after it was
        // released would drop the file at once, wherever the pointer is.
        // (GetAsyncKeyState reads physical buttons: mind swapped ones.)
        let primary = if GetSystemMetrics(SM_SWAPBUTTON) != 0 { VK_RBUTTON } else { VK_LBUTTON };
        if GetAsyncKeyState(primary.0 as i32) >= 0 {
            return;
        }
        if OleInitialize(None).is_err() {
            return;
        }
        // Absolute item ids of each file; the data object holds all of them.
        let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
        for p in &paths {
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            if SHParseDisplayName(&HSTRING::from(p.as_os_str()), None::<&IBindCtx>, &mut pidl, 0, None).is_ok() && !pidl.is_null() {
                pidls.push(pidl);
            }
        }
        if !pidls.is_empty() {
            let children: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const ITEMIDLIST).collect();
            if let Ok(data) = SHCreateDataObject(None, Some(&children), None::<&IDataObject>) {
                let data: IDataObject = data;
                let owner = HWND(hwnd as _);
                // Copy (or link); never move a download out from under the list.
                let _ = SHDoDragDrop(Some(owner), &data, None, DROPEFFECT_COPY | DROPEFFECT_LINK);
            }
        }
        for p in pidls {
            CoTaskMemFree(Some(p as _));
        }
        OleUninitialize();
    });
}
