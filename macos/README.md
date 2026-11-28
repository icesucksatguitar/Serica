# Serica for macOS

The macOS build of Serica: the same browser as the Windows app in [`../rust`](../rust)
(same UI, same Rust core, same features), running on Apple's **WebKit**
(`WKWebView`) instead of WebView2, with a native Mac window, menu bar and
keyboard shortcuts.

- **macOS 14 Sonoma or later**, Apple Silicon (or Universal with `--universal`)
- Ships as `Serica.app` in a `.dmg`; bundle id `com.serica.browser`

## Building

**Requirements:** macOS 14+, the Xcode Command Line Tools (`xcode-select --install`),
[Rust](https://rustup.rs) 1.80+ and Node.js 20+.

```bash
cd macos && npm install
python3 build_mac.py            # a menu of everything below
```

Everything goes through `build_mac.py` (the Mac counterpart of the root `build.py`);
`python3 build_mac.py --help` lists every flag. What it makes lands in `macos/release/`.

**Try it out.** Nothing is shipped, and your everyday Serica profile is never touched:

```bash
python3 build_mac.py --test             # build "Serica Test.app" and open it
python3 build_mac.py --test --fresh     # ...from a clean build and an empty test profile
python3 build_mac.py --dev              # hot reload while you edit the UI
```

The test copy has its own identifier, `com.serica.browser.test`
(`src-tauri/tauri.test.conf.json`), so its own profile folders
(`~/Library/Application Support/com.serica.browser.test`, its own website data),
its own Keychain item ("Serica Test Safe Storage") and its own single-instance
lock: it opens while your installed Serica runs. It never auto-updates or loads UI
patches. Its log is written to `release/test/serica-test.log`.

**Local build** (`release/app/Serica.app`, uses your real profile):

```bash
python3 build_mac.py                    # or --bump patch | minor | major, --fresh, --run, --universal
```

**Shipping to users (both systems at once):** Windows and macOS share one
version and one GitHub release. From the repository root:

```bash
python3 release.py patch     # bumps rust/ and macos/ together, commits, tags vX.Y.Z, pushes
```

The tag starts `.github/workflows/release.yml`: it checks both apps are on the
tag's version, builds the Windows MSI and the Universal Mac disk image side by
side, and puts both on the release `vX.Y.Z` with one `latest.json`
(`windows-x86_64`, `darwin-aarch64`, `darwin-x86_64`). The release is a draft
until you publish it on GitHub (or run the workflow by hand with *live*).
It needs the `TAURI_SIGNING_PRIVATE_KEY` secret (the updater key); the
`APPLE_*` secrets add Developer ID signing and notarization.
`.github/workflows/ci.yml` builds and tests both apps on every push.

**Shipping the Mac build by hand** (the same release, from this Mac):

```bash
python3 build_mac.py -b patch --bundle            # .app + .dmg + signed update archive -> release/github
git commit -am "macOS version X.Y.Z" && git push
python3 build_mac.py --publish-release            # adds the Mac assets to the GitHub release (a draft)
python3 build_mac.py --publish-release --live     # ...or publishes it
python3 build_mac.py --ui-patch --notes "Fixed X" [--publish]   # React/CSS-only fix, no reinstall
```

The app is ad-hoc signed unless `APPLE_SIGNING_IDENTITY` names a Developer ID
(Tauri also notarizes when `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID` are
set). The update archive is signed with the updater key at
`~/.tauri/serica-updater.key`; without it the app and disk image are built but no
update archive. `--publish-release` merges the `darwin-aarch64` / `darwin-x86_64`
entries into the release's `latest.json`, next to Windows. Mac UI patches use
`ui-latest-macos.json`, so Windows and Mac patches never cross.

**Tests:**

```bash
cd macos && npx tsc --noEmit && npm run build && npm test
cd macos/src-tauri && cargo test
```

`npm test` includes `tools/tests/macos.test.ts`, which fails if Windows wording,
Ctrl/Alt shortcut hints, Windows fonts or window buttons come back into the UI.

`python3 build_mac.py --test --automation` builds the test copy with verification
hooks (`src-tauri/src/automation.rs`, Cargo feature `automation`, never in a
release) on `127.0.0.1:7599`: scripts in the UI or a page, key and mouse events
through `NSApp.sendEvent`, window geometry and composited snapshots.

## How it maps to the Windows build

The UI (`src/`) and most of the Rust core (`src-tauri/src/`: store, sidebar model,
Shields engine, Focus, VPN, downloads, history, bookmarks, MCP, updater…) are the
same code. What WebView2 did is done with WebKit and AppKit:

| Windows (WebView2, Win32) | macOS (WebKit, AppKit) |
| --- | --- |
| `webview_win.rs` (WebView2 events) | `webview_mac/` — `WKNavigationDelegate`, `WKUIDelegate`, script message handler in Serica's own content world, key-value observing |
| WebView2 profile per Serica profile | `WKWebsiteDataStore(forIdentifier:)` with a UUID v5 of the profile; private tabs use a non-persistent store |
| `WebResourceRequested` blocking (Shields, Focus) | compiled `WKContentRuleList`s (`content_rules.rs`), switched per tab; the live log names the matching filter |
| Region clipping of page views (`clip_win.rs`) | `CAShapeLayer` masks driven by `CADisplayLink` (`clip_mac.rs`) |
| WebView2 context menu | `willOpenMenu:withEvent:` hands the items to Serica's own menu (`webview_mac/menu.rs`) |
| WebView2 password manager | Serica's own store: AES-256-GCM files, key in the login Keychain (`passwords.rs`), with the save / update prompt |
| `AddBrowserExtension` | `WKWebExtensionController` on macOS 15.4+, with Serica's window and tabs behind `chrome.tabs` / `chrome.windows` (`extensions/webkit_tabs.rs`); before that, extensions' content scripts are injected (`extensions/webkit.rs`); the Extensions panel says which |
| `view-source:` | Serica's source viewer (WebKit has none) |
| Window buttons drawn by the UI | native traffic lights (hidden with an auto-hidden top bar), drag regions handled by `useMacWindow.ts` |
| Ctrl shortcuts | ⌘ shortcuts, a full menu bar (`menu_mac.rs`) with the standard Edit menu, so ⌘C / ⌘V / ⌘A / ⌘Z work in the UI and in pages |
| DPAPI-protected Chrome passwords | Keychain "… Safe Storage" key, PBKDF2 → AES-128-CBC (`browser_import.rs`), plus a Safari importer |
| Taskbar / Explorer / default apps | Dock, Finder (reveal, drag out, quarantine), `NSWorkspace` default browser for http, https, HTML and PDF |

## Keyboard shortcuts

| | |
| --- | --- |
| ⌘T / ⌘W / ⇧⌘T | New tab / close tab / reopen closed tab |
| ⌘K or ⌘L | Address bar, open tabs and commands |
| ⌃Tab / ⌃⇧Tab (also ⇧⌘] / ⇧⌘[) | Next / previous tab |
| ⌘1 … ⌘9 | Spaces 1–9 |
| ⌘S or ⌘E | Show or hide the sidebar |
| ⌘[ / ⌘] (⌘← / ⌘→ outside text) | Back / forward |
| ⌘R / ⇧⌘R / ⌘. | Reload / reload ignoring the cache / stop |
| ⌘F, ⌘G / ⇧⌘G | Find in page, next / previous match |
| ⌘Y (or ⌘H) / ⌘J / ⇧⌘H | History / downloads / new tab page |
| ⇧⌘N | New private tab |
| ⌘+ / ⌘− / ⌘0 | Zoom |
| ⌘, | Settings |
| ⌥⌘I or F12 | Web Inspector |

## Notes and limits

- **Extensions.** WebKit runs Chrome Web Store extensions in its own extension
  runtime from macOS 15.4: background workers, content scripts, popups and
  options pages, `chrome.tabs` / `chrome.windows` over the pages of the space
  in use (private pages are left out). `runtime.id` is the extension's Chrome
  id and its pages live at `webkit-extension://<id>/`. Popups and options pages
  open in a tab (Serica has no per-extension toolbar button). On macOS 14–15.3
  only their content scripts (and styles) run, with a minimal `chrome.runtime` /
  `chrome.i18n` / in-memory `chrome.storage`; extensions that need a background
  runtime say so in the Extensions panel.
- **Importing from Safari** needs Full Disk Access for Serica (System Settings ›
  Privacy & Security), which macOS requires for any app reading Safari's files;
  Serica opens that page when it's needed. Safari's passwords live in iCloud
  Keychain: export them from the Passwords app (File › Export All Passwords…) and
  use "Import passwords from a file".
- **Chromium browsers' passwords:** macOS asks once whether Serica may read that
  browser's "Safe Storage" key in your Keychain.
- **Data:** `~/Library/Application Support/com.serica.browser` (Serica's own
  files) and `~/Library/WebKit/com.serica.browser` (website data). A legacy
  "neutrino" folder is migrated on first run.
