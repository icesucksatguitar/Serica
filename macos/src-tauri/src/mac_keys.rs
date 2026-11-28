//! Browser shortcuts on macOS: which key presses inside a web page belong to
//! Serica (the Windows build's Ctrl shortcuts, with Command).
//!
//! These are taken before the page sees them, like the Windows build's
//! `AcceleratorKeyPressed` handler. Everything else goes to the page first,
//! and only then to the menu bar (so Copy, Paste, Undo, Command-Left in a
//! text field and the like keep their usual meaning).

use crate::app::Shortcut;

pub const CMD: u64 = 1 << 20; // NSEventModifierFlagCommand
pub const SHIFT: u64 = 1 << 17; // NSEventModifierFlagShift
pub const CTRL: u64 = 1 << 18; // NSEventModifierFlagControl
pub const OPT: u64 = 1 << 19; // NSEventModifierFlagOption

// Virtual key codes (Carbon kVK_*) for keys whose character is not useful.
pub const KEY_TAB: u16 = 0x30;
pub const KEY_ESCAPE: u16 = 0x35;
pub const KEY_F5: u16 = 0x60;
pub const KEY_F6: u16 = 0x61;
pub const KEY_F12: u16 = 0x6F;
pub const KEY_PAGE_UP: u16 = 0x74;
pub const KEY_PAGE_DOWN: u16 = 0x79;
pub const KEY_KEYPAD_PLUS: u16 = 0x45;
pub const KEY_KEYPAD_MINUS: u16 = 0x4E;
/// Number row 1..9 (layout independent, so Command-1 works on AZERTY too).
const NUMBER_ROW: [u16; 9] = [0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19];
const KEY_0: u16 = 0x1D;

/// A key press as AppKit reports it.
#[derive(Clone, Copy, Debug)]
pub struct Key<'a> {
    pub code: u16,
    /// `charactersIgnoringModifiers`, lowercased by the caller.
    pub chars: &'a str,
    pub flags: u64,
}

impl Key<'_> {
    fn only(&self, want: u64) -> bool {
        self.flags & (CMD | SHIFT | CTRL | OPT) == want
    }
    fn ch(&self) -> char {
        self.chars.chars().next().unwrap_or('\0')
    }
}

/// The Serica shortcut for a key press inside a page, if it is one.
pub fn shortcut_for(k: Key) -> Option<Shortcut> {
    let c = k.ch();
    if k.only(CMD) {
        if let Some(i) = NUMBER_ROW.iter().position(|&n| n == k.code) {
            return Some(Shortcut::SpaceIndex(i + 1));
        }
        return match c {
            't' => Some(Shortcut::NewTab),
            'l' | 'k' => Some(Shortcut::OpenCommandBar),
            'w' => Some(Shortcut::CloseTab),
            's' | 'e' => Some(Shortcut::ToggleSidebar),
            'f' => Some(Shortcut::Find),
            'y' | 'h' => Some(Shortcut::History),
            'o' => Some(Shortcut::OpenFile),
            'j' => Some(Shortcut::Downloads),
            ',' => Some(Shortcut::Settings),
            'r' => Some(Shortcut::Reload),
            'p' => Some(Shortcut::Print),
            '=' | '+' => Some(Shortcut::ZoomIn),
            '-' => Some(Shortcut::ZoomOut),
            '0' => Some(Shortcut::ZoomReset),
            '.' => Some(Shortcut::Stop),
            'g' => Some(Shortcut::FindNext),
            _ if k.code == KEY_0 => Some(Shortcut::ZoomReset),
            _ if k.code == KEY_KEYPAD_PLUS => Some(Shortcut::ZoomIn),
            _ if k.code == KEY_KEYPAD_MINUS => Some(Shortcut::ZoomOut),
            _ => None,
        };
    }
    if k.only(CMD | SHIFT) {
        return match c {
            't' => Some(Shortcut::ReopenTab),
            'n' => Some(Shortcut::NewPrivateTab),
            'r' => Some(Shortcut::HardReload),
            'j' => Some(Shortcut::Downloads),
            'h' => Some(Shortcut::Home),
            ']' | '}' => Some(Shortcut::CycleTab(1)),
            '[' | '{' => Some(Shortcut::CycleTab(-1)),
            '=' | '+' => Some(Shortcut::ZoomIn),
            'g' => Some(Shortcut::FindPrevious),
            _ => None,
        };
    }
    if k.only(CMD | OPT) && (c == 'i' || k.code == 0x22) {
        return Some(Shortcut::DevTools);
    }
    if k.code == KEY_TAB && (k.only(CTRL) || k.only(CTRL | SHIFT)) {
        return Some(Shortcut::CycleTab(if k.flags & SHIFT != 0 { -1 } else { 1 }));
    }
    if k.only(CTRL) && k.code == KEY_PAGE_DOWN {
        return Some(Shortcut::CycleTab(1));
    }
    if k.only(CTRL) && k.code == KEY_PAGE_UP {
        return Some(Shortcut::CycleTab(-1));
    }
    // Function keys (the fn modifier bit is ignored).
    let plain = k.flags & (CMD | SHIFT | CTRL | OPT) == 0;
    match k.code {
        KEY_F12 if plain => Some(Shortcut::DevTools),
        KEY_F6 if plain => Some(Shortcut::OpenCommandBar),
        KEY_F5 if plain => Some(Shortcut::Reload),
        KEY_F5 if k.only(SHIFT) || k.only(CMD) => Some(Shortcut::HardReload),
        KEY_ESCAPE if plain => Some(Shortcut::Escape),
        _ => None,
    }
}

/// Shortcut glyphs as macOS shows them (`⌘⇧T`).
pub fn glyphs(flags: u64, key: &str) -> String {
    let mut s = String::new();
    if flags & CTRL != 0 {
        s.push('⌃');
    }
    if flags & OPT != 0 {
        s.push('⌥');
    }
    if flags & SHIFT != 0 {
        s.push('⇧');
    }
    if flags & CMD != 0 {
        s.push('⌘');
    }
    s.push_str(&key.to_uppercase());
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: u16, chars: &str, flags: u64) -> Option<Shortcut> {
        shortcut_for(Key { code, chars, flags })
    }

    #[test]
    fn command_shortcuts() {
        assert!(matches!(k(0x11, "t", CMD), Some(Shortcut::NewTab)));
        assert!(matches!(k(0x11, "t", CMD | SHIFT), Some(Shortcut::ReopenTab)));
        assert!(matches!(k(0x2D, "n", CMD | SHIFT), Some(Shortcut::NewPrivateTab)));
        assert!(matches!(k(0x25, "l", CMD), Some(Shortcut::OpenCommandBar)));
        assert!(matches!(k(0x28, "k", CMD), Some(Shortcut::OpenCommandBar)));
        assert!(matches!(k(0x0D, "w", CMD), Some(Shortcut::CloseTab)));
        assert!(matches!(k(0x01, "s", CMD), Some(Shortcut::ToggleSidebar)));
        assert!(matches!(k(0x0E, "e", CMD), Some(Shortcut::ToggleSidebar)));
        assert!(matches!(k(0x03, "f", CMD), Some(Shortcut::Find)));
        assert!(matches!(k(0x10, "y", CMD), Some(Shortcut::History)));
        assert!(matches!(k(0x04, "h", CMD), Some(Shortcut::History)));
        assert!(matches!(k(0x26, "j", CMD), Some(Shortcut::Downloads)));
        assert!(matches!(k(0x26, "j", CMD | SHIFT), Some(Shortcut::Downloads)));
        assert!(matches!(k(0x2B, ",", CMD), Some(Shortcut::Settings)));
        assert!(matches!(k(0x1F, "o", CMD), Some(Shortcut::OpenFile)));
        assert!(matches!(k(0x0F, "r", CMD), Some(Shortcut::Reload)));
        assert!(matches!(k(0x0F, "r", CMD | SHIFT), Some(Shortcut::HardReload)));
        assert!(matches!(k(0x22, "i", CMD | OPT), Some(Shortcut::DevTools)));
        assert!(matches!(k(KEY_F12, "\u{F70F}", 0), Some(Shortcut::DevTools)));
        assert!(matches!(k(0x05, "g", CMD), Some(Shortcut::FindNext)));
        assert!(matches!(k(0x05, "g", CMD | SHIFT), Some(Shortcut::FindPrevious)));
    }

    #[test]
    fn spaces_and_tabs() {
        assert!(matches!(k(0x14, "3", CMD), Some(Shortcut::SpaceIndex(3))));
        // AZERTY: Command-1 reports "&" but the key is the same.
        assert!(matches!(k(0x12, "&", CMD), Some(Shortcut::SpaceIndex(1))));
        assert!(matches!(k(KEY_TAB, "\t", CTRL), Some(Shortcut::CycleTab(1))));
        assert!(matches!(k(KEY_TAB, "\t", CTRL | SHIFT), Some(Shortcut::CycleTab(-1))));
        assert!(matches!(k(0x1E, "}", CMD | SHIFT), Some(Shortcut::CycleTab(1))));
        assert!(matches!(k(KEY_PAGE_UP, "", CTRL), Some(Shortcut::CycleTab(-1))));
    }

    #[test]
    fn zoom() {
        assert!(matches!(k(0x18, "=", CMD), Some(Shortcut::ZoomIn)));
        assert!(matches!(k(0x18, "+", CMD | SHIFT), Some(Shortcut::ZoomIn)));
        assert!(matches!(k(0x1B, "-", CMD), Some(Shortcut::ZoomOut)));
        assert!(matches!(k(0x1D, "0", CMD), Some(Shortcut::ZoomReset)));
    }

    #[test]
    fn page_keys_stay_with_the_page() {
        // Editing, Command-arrows and Command-[ / ] go to the page first.
        for (code, ch) in [(0x08, "c"), (0x09, "v"), (0x07, "x"), (0x00, "a"), (0x06, "z"), (0x21, "["), (0x1E, "]")] {
            assert!(k(code, ch, CMD).is_none(), "{ch}");
        }
        assert!(k(0x7B, "\u{F702}", CMD).is_none(), "Command-Left moves to the line start");
        assert!(k(0x7C, "\u{F703}", CMD).is_none(), "Command-Right moves to the line end");
        assert!(k(0x11, "t", 0).is_none());
        assert!(k(0x11, "t", CTRL).is_none(), "Control-T transposes in text fields");
        assert!(k(0x28, "k", CTRL).is_none(), "Control-K kills a line in text fields");
        assert!(matches!(k(KEY_ESCAPE, "\u{1b}", 0), Some(Shortcut::Escape)));
    }

    #[test]
    fn glyph_labels() {
        assert_eq!(glyphs(CMD | SHIFT, "t"), "⇧⌘T");
        assert_eq!(glyphs(CTRL, "tab"), "⌃TAB");
    }
}
