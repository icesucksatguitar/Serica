//! The macOS menu bar.
//!
//! Without an Edit menu, Command-C/V/X/A/Z beep and do nothing in text
//! fields and pages: AppKit sends them as menu actions (`copy:` and so on)
//! to the focused view. The menus also carry Serica's commands with their
//! shortcuts, as a Mac app should. In a page, Serica's own shortcuts are
//! taken before the page (`mac_tabview`); in the interface, the UI handles
//! them and the menu only gets the keys it leaves alone.

use crate::app::Shortcut;
use crate::mac_keys::{CMD, CTRL, OPT, SHIFT};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::{NSObjectProtocol, NSString};
use std::cell::RefCell;

/// Serica commands behind menu items, by item tag.
fn command(tag: isize) -> Option<Shortcut> {
    Some(match tag {
        1 => Shortcut::NewTab,
        2 => Shortcut::NewPrivateTab,
        3 => Shortcut::OpenCommandBar,
        4 => Shortcut::OpenFile,
        5 => Shortcut::CloseTab,
        6 => Shortcut::ReopenTab,
        7 => Shortcut::Print,
        8 => Shortcut::Find,
        9 => Shortcut::FindNext,
        10 => Shortcut::FindPrevious,
        11 => Shortcut::ToggleSidebar,
        12 => Shortcut::Reload,
        13 => Shortcut::HardReload,
        14 => Shortcut::Stop,
        15 => Shortcut::ZoomReset,
        16 => Shortcut::ZoomIn,
        17 => Shortcut::ZoomOut,
        18 => Shortcut::DevTools,
        19 => Shortcut::Back,
        20 => Shortcut::Forward,
        21 => Shortcut::Home,
        22 => Shortcut::History,
        23 => Shortcut::CycleTab(1),
        24 => Shortcut::CycleTab(-1),
        25 => Shortcut::Downloads,
        26 => Shortcut::Settings,
        27 => Shortcut::About,
        _ => return None,
    })
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct SericaMenuTarget;

    unsafe impl NSObjectProtocol for SericaMenuTarget {}

    impl SericaMenuTarget {
        #[unsafe(method(sericaCommand:))]
        fn run(&self, sender: &NSMenuItem) {
            if let (Some(sc), Some(app)) = (command(sender.tag()), crate::app::global()) {
                app.shortcut(sc);
            }
        }

        #[unsafe(method(sericaHelp:))]
        fn help(&self, _sender: &NSMenuItem) {
            let _ = open::that_detached("https://github.com/icesucksatguitar/Serica#readme");
        }
    }
);

thread_local! {
    static TARGET: RefCell<Option<Retained<SericaMenuTarget>>> = const { RefCell::new(None) };
}

fn flags(mask: u64) -> NSEventModifierFlags {
    NSEventModifierFlags(mask as usize)
}

struct Builder {
    mtm: MainThreadMarker,
    target: Retained<SericaMenuTarget>,
}

impl Builder {
    fn menu(&self, title: &str) -> Retained<NSMenu> {
        let m = NSMenu::initWithTitle(self.mtm.alloc(), &NSString::from_str(title));
        m.setAutoenablesItems(true);
        m
    }

    /// An item sending a standard action up the responder chain.
    fn std(&self, menu: &NSMenu, title: &str, action: Sel, key: &str, mask: u64) -> Retained<NSMenuItem> {
        let item = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(self.mtm.alloc(), &NSString::from_str(title), Some(action), &NSString::from_str(key)) };
        if !key.is_empty() {
            item.setKeyEquivalentModifierMask(flags(mask));
        }
        menu.addItem(&item);
        item
    }

    /// An item running a Serica command.
    fn cmd(&self, menu: &NSMenu, title: &str, tag: isize, key: &str, mask: u64) -> Retained<NSMenuItem> {
        let item = self.std(menu, title, sel!(sericaCommand:), key, mask);
        unsafe { item.setTarget(Some(&self.target)) };
        item.setTag(tag);
        item
    }

    /// A second shortcut for a command, active though not shown.
    fn alt(&self, menu: &NSMenu, title: &str, tag: isize, key: &str, mask: u64) {
        let item = self.cmd(menu, title, tag, key, mask);
        item.setHidden(true);
        if crate::mac_util::responds(&*item, sel!(setAllowsKeyEquivalentWhenHidden:)) {
            item.setAllowsKeyEquivalentWhenHidden(true);
        }
    }

    fn sep(&self, menu: &NSMenu) {
        menu.addItem(&NSMenuItem::separatorItem(self.mtm));
    }

    fn sub(&self, parent: &NSMenu, title: &str) -> Retained<NSMenu> {
        let item = NSMenuItem::new(self.mtm);
        item.setTitle(&NSString::from_str(title));
        let m = self.menu(title);
        item.setSubmenu(Some(&m));
        parent.addItem(&item);
        m
    }
}

/// Builds the menu bar. Main thread, once, at start.
pub fn install() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let target: Retained<SericaMenuTarget> = unsafe { msg_send![SericaMenuTarget::alloc(mtm), init] };
    TARGET.with(|t| *t.borrow_mut() = Some(target.clone()));
    let b = Builder { mtm, target };
    let ns_app = NSApplication::sharedApplication(mtm);
    // One window, one set of tabs: no system window tabs.
    objc2_app_kit::NSWindow::setAllowsAutomaticWindowTabbing(false, mtm);

    let bar = b.menu("");
    let name = "Serica";

    // App menu
    let app = b.sub(&bar, name);
    b.cmd(&app, &format!("About {name}"), 27, "", 0);
    b.cmd(&app, "Check for Updates…", 27, "", 0);
    b.sep(&app);
    b.cmd(&app, "Settings…", 26, ",", CMD);
    b.sep(&app);
    let services = b.sub(&app, "Services");
    ns_app.setServicesMenu(Some(&services));
    b.sep(&app);
    // Command-H shows History in Serica; hiding keeps the menu item.
    b.std(&app, &format!("Hide {name}"), sel!(hide:), "", 0);
    b.std(&app, "Hide Others", sel!(hideOtherApplications:), "h", CMD | OPT);
    b.std(&app, "Show All", sel!(unhideAllApplications:), "", 0);
    b.sep(&app);
    b.std(&app, &format!("Quit {name}"), sel!(terminate:), "q", CMD);

    // File
    let file = b.sub(&bar, "File");
    b.cmd(&file, "New Tab", 1, "t", CMD);
    b.cmd(&file, "New Private Tab", 2, "n", CMD | SHIFT);
    b.cmd(&file, "Open Location…", 3, "l", CMD);
    b.cmd(&file, "Open File…", 4, "o", CMD);
    b.sep(&file);
    b.cmd(&file, "Close Tab", 5, "w", CMD);
    b.cmd(&file, "Reopen Closed Tab", 6, "t", CMD | SHIFT);
    b.std(&file, "Close Window", sel!(performClose:), "w", CMD | SHIFT);
    b.sep(&file);
    b.cmd(&file, "Print…", 7, "p", CMD);

    // Edit (AppKit adds Emoji & Symbols, Dictation and AutoFill to it)
    let edit = b.sub(&bar, "Edit");
    b.std(&edit, "Undo", sel!(undo:), "z", CMD);
    b.std(&edit, "Redo", sel!(redo:), "z", CMD | SHIFT);
    b.sep(&edit);
    b.std(&edit, "Cut", sel!(cut:), "x", CMD);
    b.std(&edit, "Copy", sel!(copy:), "c", CMD);
    b.std(&edit, "Paste", sel!(paste:), "v", CMD);
    b.std(&edit, "Paste and Match Style", sel!(pasteAsPlainText:), "v", CMD | OPT | SHIFT);
    b.std(&edit, "Delete", sel!(delete:), "", 0);
    b.std(&edit, "Select All", sel!(selectAll:), "a", CMD);
    b.sep(&edit);
    let find = b.sub(&edit, "Find");
    b.cmd(&find, "Find…", 8, "f", CMD);
    b.cmd(&find, "Find Next", 9, "g", CMD);
    b.cmd(&find, "Find Previous", 10, "g", CMD | SHIFT);
    let spelling = b.sub(&edit, "Spelling and Grammar");
    b.std(&spelling, "Show Spelling and Grammar", sel!(showGuessPanel:), ":", CMD);
    b.std(&spelling, "Check Document Now", sel!(checkSpelling:), ";", CMD);
    b.sep(&spelling);
    b.std(&spelling, "Check Spelling While Typing", sel!(toggleContinuousSpellChecking:), "", 0);
    b.std(&spelling, "Check Grammar With Spelling", sel!(toggleGrammarChecking:), "", 0);
    b.std(&spelling, "Correct Spelling Automatically", sel!(toggleAutomaticSpellingCorrection:), "", 0);
    let subs = b.sub(&edit, "Substitutions");
    b.std(&subs, "Show Substitutions", sel!(orderFrontSubstitutionsPanel:), "", 0);
    b.sep(&subs);
    b.std(&subs, "Smart Copy/Paste", sel!(toggleSmartInsertDelete:), "", 0);
    b.std(&subs, "Smart Quotes", sel!(toggleAutomaticQuoteSubstitution:), "", 0);
    b.std(&subs, "Smart Dashes", sel!(toggleAutomaticDashSubstitution:), "", 0);
    b.std(&subs, "Smart Links", sel!(toggleAutomaticLinkDetection:), "", 0);
    b.std(&subs, "Text Replacement", sel!(toggleAutomaticTextReplacement:), "", 0);
    let transforms = b.sub(&edit, "Transformations");
    b.std(&transforms, "Make Upper Case", sel!(uppercaseWord:), "", 0);
    b.std(&transforms, "Make Lower Case", sel!(lowercaseWord:), "", 0);
    b.std(&transforms, "Capitalize", sel!(capitalizeWord:), "", 0);
    let speech = b.sub(&edit, "Speech");
    b.std(&speech, "Start Speaking", sel!(startSpeaking:), "", 0);
    b.std(&speech, "Stop Speaking", sel!(stopSpeaking:), "", 0);

    // View (AppKit adds Enter Full Screen)
    let view = b.sub(&bar, "View");
    b.cmd(&view, "Show or Hide Sidebar", 11, "s", CMD);
    b.cmd(&view, "Address Bar and Commands", 3, "k", CMD);
    b.sep(&view);
    b.cmd(&view, "Reload Page", 12, "r", CMD);
    b.cmd(&view, "Reload Ignoring Cache", 13, "r", CMD | SHIFT);
    b.cmd(&view, "Stop", 14, ".", CMD);
    b.sep(&view);
    b.cmd(&view, "Actual Size", 15, "0", CMD);
    b.cmd(&view, "Zoom In", 16, "+", CMD);
    b.alt(&view, "Zoom In", 16, "=", CMD);
    b.cmd(&view, "Zoom Out", 17, "-", CMD);
    b.sep(&view);
    let dev = b.sub(&view, "Developer");
    b.cmd(&dev, "Web Inspector", 18, "i", CMD | OPT);

    // History
    let history = b.sub(&bar, "History");
    b.cmd(&history, "Back", 19, "[", CMD);
    b.cmd(&history, "Forward", 20, "]", CMD);
    b.alt(&history, "Back", 19, "\u{F702}", CMD);
    b.alt(&history, "Forward", 20, "\u{F703}", CMD);
    b.cmd(&history, "Home", 21, "h", CMD | SHIFT);
    b.sep(&history);
    b.cmd(&history, "Show All History", 22, "y", CMD);
    b.alt(&history, "Show All History", 22, "h", CMD);

    // Window
    let window = b.sub(&bar, "Window");
    b.std(&window, "Minimize", sel!(performMiniaturize:), "m", CMD);
    b.std(&window, "Zoom", sel!(performZoom:), "", 0);
    b.sep(&window);
    b.cmd(&window, "Show Next Tab", 23, "\t", CTRL);
    b.cmd(&window, "Show Previous Tab", 24, "\t", CTRL | SHIFT);
    b.sep(&window);
    b.cmd(&window, "Downloads", 25, "j", CMD);
    b.alt(&window, "Downloads", 25, "j", CMD | SHIFT);
    b.sep(&window);
    b.std(&window, "Bring All to Front", sel!(arrangeInFront:), "", 0);
    ns_app.setWindowsMenu(Some(&window));

    // Help
    let help = b.sub(&bar, "Help");
    let item = b.std(&help, &format!("{name} Help"), sel!(sericaHelp:), "?", CMD);
    unsafe { item.setTarget(Some(&b.target)) };
    ns_app.setHelpMenu(Some(&help));

    ns_app.setMainMenu(Some(&bar));
    let _: Option<&AnyObject> = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tag_is_a_command() {
        for tag in 1..=27 {
            assert!(command(tag).is_some(), "tag {tag}");
        }
        assert!(command(0).is_none());
    }
}
