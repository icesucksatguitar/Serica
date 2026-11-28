//! Serica's own menu for a right-click in a page.
//!
//! WebKit builds its context menu and AppKit is about to show it
//! (`willOpenMenu:withEvent:` on the tab view). Its items are taken out (so
//! nothing native shows), described to the UI, which draws the menu, and kept:
//! a pick runs the item's own action, exactly as the native menu would
//! (WebKit keeps the menu's context until the next right-click), so copy,
//! paste, spelling, Look Up and Inspect Element behave as in Safari.

use super::*;
use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem, NSUserInterfaceItemIdentification};
use std::sync::atomic::{AtomicU64, Ordering};

struct PendingMenu {
    tab_id: String,
    webview: Weak<WKWebView>,
    /// Native items by the id sent to the UI.
    items: Vec<Retained<NSMenuItem>>,
    /// Serica's own entries: (url, open in foreground).
    custom: Vec<(String, bool)>,
}

thread_local! {
    static PENDING: RefCell<Option<(u64, PendingMenu)>> = const { RefCell::new(None) };
}
static NEXT_MENU_ID: AtomicU64 = AtomicU64::new(1);

/// WebKit's items that don't fit Serica: "new window" (Serica opens tabs and
/// offers its own entries), the system web search (Serica searches with the
/// chosen engine), and services that need the native menu to stay on screen.
const HIDDEN_IDS: &[&str] = &[
    "WKMenuItemIdentifierOpenLinkInNewWindow",
    "WKMenuItemIdentifierOpenImageInNewWindow",
    "WKMenuItemIdentifierOpenFrameInNewWindow",
    "WKMenuItemIdentifierOpenMediaInNewWindow",
    "WKMenuItemIdentifierSearchWeb",
    "WKMenuItemIdentifierShareMenu",
    "WKMenuItemIdentifierSpeechMenu",
    "WKMenuItemIdentifierTranslate",
    "WKMenuItemIdentifierAddHighlightToCurrentQuickNote",
    "WKMenuItemIdentifierAddHighlightToNewQuickNote",
];

const HIDDEN_TITLES: &[&str] = &["Services", "Share", "Speech", "Writing Tools", "AutoFill", "Translate", "Open Link in New Window", "Open Image in New Window", "Open Frame in New Window", "Open Video in New Window", "Open Audio in New Window"];

/// Serica's names for WebKit's items (the UI picks icons by these, as it did
/// for WebView2's item names).
fn item_name(identifier: &str, action: Option<Sel>, title: &str) -> String {
    let by_id = match identifier {
        "WKMenuItemIdentifierCopy" => "copy",
        "WKMenuItemIdentifierCopyImage" => "copyImage",
        "WKMenuItemIdentifierCopyLink" | "WKMenuItemIdentifierCopyMediaLink" => "copyLinkLocation",
        "WKMenuItemIdentifierDownloadImage" => "saveImageAs",
        "WKMenuItemIdentifierDownloadLinkedFile" => "saveLinkAs",
        "WKMenuItemIdentifierDownloadMedia" => "saveMediaAs",
        "WKMenuItemIdentifierGoBack" => "back",
        "WKMenuItemIdentifierGoForward" => "forward",
        "WKMenuItemIdentifierReload" => "reload",
        "WKMenuItemIdentifierInspectElement" => "inspectElement",
        "WKMenuItemIdentifierLookUp" => "lookUp",
        "WKMenuItemIdentifierPaste" => "paste",
        "WKMenuItemIdentifierOpenLink" => "openLink",
        "WKMenuItemIdentifierSpellingMenu" => "spellCheck",
        _ => "",
    };
    if !by_id.is_empty() {
        return by_id.to_string();
    }
    let by_action = match action.map(|s| s.name().to_string_lossy().to_string()).as_deref() {
        Some("cut:") => "cut",
        Some("copy:") => "copy",
        Some("paste:") => "paste",
        Some("pasteAsPlainText:") => "pasteAndMatchStyle",
        Some("selectAll:") => "selectAll",
        Some("undo:") => "undo",
        Some("redo:") => "redo",
        Some("orderFrontCharacterPalette:") => "emoji",
        _ => "",
    };
    if !by_action.is_empty() {
        return by_action.to_string();
    }
    match title {
        "Cut" => "cut",
        "Copy" => "copy",
        "Paste" => "paste",
        "Paste and Match Style" => "pasteAndMatchStyle",
        "Select All" => "selectAll",
        "Spelling and Grammar" => "spellCheck",
        "Print…" | "Print..." => "print",
        _ => "other",
    }
    .to_string()
}

fn search_engine_label(id: &str) -> &'static str {
    crate::settings::search_engine_label(id)
}

/// The native items as JSON for the UI (and the flat list they index).
fn describe(menu: &NSMenu, keep: &mut Vec<Retained<NSMenuItem>>, editable: bool) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for item in menu.itemArray().iter() {
        if item.isHidden() {
            continue;
        }
        if item.isSeparatorItem() {
            out.push(serde_json::json!({ "type": "separator" }));
            continue;
        }
        let title = item.title().to_string();
        let identifier = item.identifier().map(|i| i.to_string()).unwrap_or_default();
        if HIDDEN_IDS.contains(&identifier.as_str()) || HIDDEN_TITLES.iter().any(|t| title == *t) || title.starts_with("Search with ") {
            continue;
        }
        let name = item_name(&identifier, item.action(), &title);
        if name == "other" && !editable && item.submenu().is_some() {
            continue;
        }
        let key = item.keyEquivalent().to_string();
        let shortcut = if key.is_empty() { String::new() } else { crate::mac_keys::glyphs(item.keyEquivalentModifierMask().0 as u64, &key) };
        let (children, submenu) = match item.submenu() {
            Some(sub) => (describe(&sub, keep, editable), true),
            None => (Vec::new(), false),
        };
        if submenu && children.is_empty() {
            continue;
        }
        let id = keep.len();
        keep.push(item.clone());
        out.push(serde_json::json!({
            "type": if submenu { "submenu" } else { "command" },
            "id": id,
            "name": name,
            "label": title,
            "shortcut": shortcut,
            "enabled": item.isEnabled(),
            "checked": item.state() == objc2_app_kit::NSControlStateValueOn,
            "children": children,
        }));
    }
    // No doubled, leading or trailing separators after the removals.
    let mut clean: Vec<serde_json::Value> = Vec::with_capacity(out.len());
    for v in out {
        let sep = v["type"] == "separator";
        if sep && clean.last().is_none_or(|l| l["type"] == "separator") {
            continue;
        }
        clean.push(v);
    }
    if clean.last().is_some_and(|l| l["type"] == "separator") {
        clean.pop();
    }
    clean
}

/// Takes WebKit's menu for a right-click in a page: the items go to the UI,
/// and the native menu is left empty, so AppKit shows nothing.
pub fn handle_context_menu(wk: &WKWebView, menu: &NSMenu, event: Option<&NSEvent>) -> bool {
    let Some(ctx) = ctx_of(wk) else { return false };
    if ctx.kind != Kind::Tab {
        return false;
    }
    let mtm = MainThreadMarker::new().expect("main thread");
    let info = ctx.st.borrow_mut().last_ctx.take().unwrap_or_default();
    let has = |id: &str| menu.itemArray().iter().any(|i| i.identifier().is_some_and(|x| x.to_string() == id));
    let link = info.link.clone().or_else(|| has("WKMenuItemIdentifierCopyLink").then(String::new).filter(|s: &String| !s.is_empty()));
    let image = info.image.clone();
    let selection = info.selection.clone();
    let editable = info.editable;
    let page = if info.page.is_empty() { unsafe { url_string(wk.URL().as_deref()) } } else { info.page.clone() };

    // Serica's entries: (label, url, foreground).
    let mut top: Vec<(String, String, bool)> = Vec::new();
    if let Some(l) = link.as_ref().filter(|l| l.starts_with("http") || l.starts_with("file:")) {
        top.push(("Open link in new tab".into(), l.clone(), true));
        top.push(("Open link in background tab".into(), l.clone(), false));
    }
    if let Some(src) = image.as_ref().filter(|s| s.starts_with("http")) {
        top.push(("Open image in new tab".into(), src.clone(), true));
    }
    if let Some(sel) = selection.as_ref().filter(|_| !editable) {
        let settings = ctx.app.settings();
        let short: String = if sel.chars().count() > 32 { sel.chars().take(30).collect::<String>() + "\u{2026}" } else { sel.clone() };
        let q: String = url::form_urlencoded::byte_serialize(sel.as_bytes()).collect();
        top.push((
            format!("Search {} for \u{201c}{short}\u{201d}", search_engine_label(&settings.search_engine)),
            format!("{}{q}", settings.search_prefix()),
            true,
        ));
    }
    let source = (link.is_none() && image.is_none() && selection.is_none() && !editable && page.starts_with("http"))
        .then(|| ("View page source".to_string(), format!("view-source:{page}"), true));

    let mut keep: Vec<Retained<NSMenuItem>> = Vec::new();
    let native = describe(menu, &mut keep, editable);
    let mut list: Vec<serde_json::Value> = Vec::new();
    let mut custom: Vec<(String, bool)> = Vec::new();
    for (label, url, fg) in &top {
        list.push(serde_json::json!({ "type": "custom", "index": custom.len(), "label": label, "name": "serica" }));
        custom.push((url.clone(), *fg));
    }
    if !list.is_empty() && !native.is_empty() {
        list.push(serde_json::json!({ "type": "separator" }));
    }
    let inspect_at = native.iter().position(|v| v["name"] == "inspectElement");
    let has_inspect = inspect_at.is_some();
    for (i, v) in native.into_iter().enumerate() {
        if Some(i) == inspect_at {
            if let Some((label, url, fg)) = &source {
                list.push(serde_json::json!({ "type": "custom", "index": custom.len(), "label": label, "name": "viewSource" }));
                custom.push((url.clone(), *fg));
            }
        }
        list.push(v);
    }
    if !has_inspect {
        if let Some((label, url, fg)) = &source {
            list.push(serde_json::json!({ "type": "custom", "index": custom.len(), "label": label, "name": "viewSource" }));
            custom.push((url.clone(), *fg));
        }
    }
    if list.is_empty() {
        return false; // nothing to offer: leave it to WebKit
    }

    // Where the click was: physical px of the window's content (top-left
    // origin), like WebView2's coordinates, plus the content width in the
    // same unit so the UI can convert to CSS px whatever the zoom. The page's
    // own `contextmenu` event knows the spot (CSS px of the top document);
    // the event WebKit hands over here is only a fallback.
    let (x, y, width) = wk
        .window()
        .map(|w| {
            let scale = w.backingScaleFactor();
            let content = w.contentView().map(|v| v.frame()).unwrap_or_else(|| w.frame());
            let loc = match info.point {
                Some((cx, cy)) => {
                    let z = unsafe { wk.pageZoom() * wk.magnification() };
                    let view: &objc2_app_kit::NSView = wk.as_ref();
                    let (vx, vy) = (cx * z, cy * z);
                    // WKWebView is flipped (top-left origin).
                    let vy = if view.isFlipped() { vy } else { view.bounds().size.height - vy };
                    view.convertPoint_toView(objc2_foundation::NSPoint::new(vx, vy), None)
                }
                None => event.map(|e| e.locationInWindow()).unwrap_or_else(|| w.mouseLocationOutsideOfEventStream()),
            };
            (loc.x * scale, (content.size.height - loc.y) * scale, content.size.width * scale)
        })
        .unwrap_or((0.0, 0.0, 0.0));

    // The native menu shows nothing; its items stay valid for the pick.
    menu.removeAllItems();
    let id = NEXT_MENU_ID.fetch_add(1, Ordering::Relaxed);
    PENDING.with(|p| *p.borrow_mut() = Some((id, PendingMenu { tab_id: ctx.tab_id.clone(), webview: Weak::from(wk), items: keep, custom })));
    let context = if editable {
        "editable"
    } else if link.is_some() {
        "link"
    } else if image.is_some() {
        "image"
    } else if selection.is_some() {
        "selection"
    } else {
        "page"
    };
    ui_emit(
        &ctx.app.handle,
        "serica:page-context-menu",
        serde_json::json!({
            "id": id,
            "tabId": ctx.tab_id,
            "x": x,
            "y": y,
            "clientWidth": width,
            "context": context,
            "link": link,
            "image": image,
            "selection": selection,
            "items": list,
        }),
    );
    // The menu takes keyboard focus (arrows, Enter, Escape); a click back
    // into the page then blurs the UI, which closes it.
    if let Some(main) = tauri::Manager::get_webview(&ctx.app.handle, "main") {
        let _ = main.set_focus();
    }
    let _ = mtm;
    true
}

/// Ends a page menu: runs the picked native item (`command`) or Serica entry
/// (`custom`), or nothing when it was dismissed. Main thread only.
/// `refocus` hands keyboard focus back to the page (a pick, or Escape).
pub fn finish_page_menu(app: &Shared, id: u64, command: Option<i32>, custom: Option<usize>, refocus: bool) {
    let Some(menu) = PENDING.with(|p| {
        let mut p = p.borrow_mut();
        if p.as_ref().is_some_and(|(pid, _)| *pid == id) {
            p.take().map(|(_, m)| m)
        } else {
            None
        }
    }) else {
        return;
    };
    let mtm = MainThreadMarker::new().expect("main thread");
    let wk = menu.webview.load();
    if command.is_some() || refocus {
        // The UI had focus while its menu was open; commands such as paste
        // act on the page's focused field.
        if let Some(wk) = &wk {
            if let Some(w) = wk.window() {
                w.makeFirstResponder(Some(wk));
            }
        }
    }
    if let Some(item) = command.and_then(|c| usize::try_from(c).ok()).and_then(|c| menu.items.get(c)) {
        if item.isEnabled() {
            if let Some(action) = item.action() {
                let target = item.target();
                let app_ns = NSApplication::sharedApplication(mtm);
                unsafe {
                    let _ = app_ns.sendAction_to_from(action, target.as_deref(), Some(item));
                }
            }
        }
    }
    if let Some((url, foreground)) = custom.and_then(|i| menu.custom.get(i).cloned()) {
        let tab_id = menu.tab_id;
        app.mutate(move |s| {
            let space = s.tab(&tab_id).map(|t| t.space_id.clone());
            s.create_tab(Some(url), space, foreground, Some(&tab_id));
        });
    }
}

/// A closed tab's menu is dropped.
pub fn forget(tab_id: &str) {
    PENDING.with(|p| {
        let mut p = p.borrow_mut();
        if p.as_ref().is_some_and(|(_, m)| m.tab_id == tab_id) {
            *p = None;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_ui_icons() {
        assert_eq!(item_name("WKMenuItemIdentifierCopyLink", None, "Copy Link"), "copyLinkLocation");
        assert_eq!(item_name("", Some(sel!(cut:)), "Cut"), "cut");
        assert_eq!(item_name("", None, "Paste and Match Style"), "pasteAndMatchStyle");
        assert_eq!(item_name("", None, "Something"), "other");
    }
}
