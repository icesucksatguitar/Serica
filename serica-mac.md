# Serica for macOS — Claude Code Implementation Prompt

**Objective:** Port Serica from Windows (Tauri v2 + WebView2) to macOS using WebKit / `WKWebView`, with complete feature, visual, architectural, and behavioral parity.

**Target:** macOS 14+ · Universal / Apple Silicon app · `.app` and `.dmg` packaging

**How to use:** Run Claude Code on macOS from the repository root, then provide the implementation prompt below in full.

---

## Contents

- [0. Hard Constraints & Directory Rules](#0-hard-constraints--directory-rules)
- [1. Inspect the Canonical Source First](#1-inspect-the-canonical-source-first)
- [2. Target Structure in `./macos/`](#2-target-structure-in-macos)
- [3. Complete Feature & Architecture Checklist](#3-complete-feature--architecture-checklist-must-preserve-11)
- [4. Windows-to-macOS / WebKit Technical Blueprint](#4-exact-windows-to-macos--webkit-technical-blueprint)
  - [4.1 Child WKWebView Architecture & Layering](#41-child-wkwebview-architecture--layering-tabhostrs-webview_macrs-clip_macrs)
  - [4.2 Profile Isolation & Proxy Routing](#42-per-profile-isolation-incognito--proxy--vpn-routing-wkwebviewedatastore)
  - [4.3 Shields & Focus Mode](#43-shields-adblock-rust--focus-mode-request-blocking-on-webkit)
  - [4.4 User Scripts & Message Bridge](#44-user-scripts-message-bridge--custom-errorpdf-scripts)
  - [4.5 Audio & Picture-in-Picture](#45-audio-multi-frame-media--picture-in-picture-no-cdp)
  - [4.6 Native Menus, Dialogs & Downloads](#46-native-context-menus-dialogs-permissions-downloads--find-in-page)
  - [4.7 Cursor, Drag & Drop, Clipboard](#47-os-cursor-edge-approach-drag--drop-and-clipboard-edge_watch_macrs-dragdrop_macrs-commandsrs)
  - [4.8 Default Browser, Import & VPN Detection](#48-default-browser-browsers-import-focus-sampling--vpn-detection)
  - [4.9 Extensions](#49-extensions-extensionsrs--extensionsmodaltsx)
- [5. macOS Window Chrome, Shortcuts & UI Polish](#5-macos-window-chrome-shortcuts-fonts--ui-polish)
- [6. Build, Packaging & Verification](#6-build-packaging--verification-plan-must-complete-before-finishing)

---

## Implementation Prompt

Treat `./rust/` as the canonical Windows reference. Build the macOS implementation independently under `./macos/`; do not alter the Windows application or its build scripts.

---

You are tasked with porting the **Serica** browser (formerly Neutrino, currently `v0.5.2` on Windows using Tauri v2 + WebView2 in `./rust/`) to **macOS (MacBook)** using **Apple WebKit (`WKWebView`)** with **1:1 feature, visual, architectural, and behavioral parity** with the Windows application.

## 0. Hard Constraints & Directory Rules

1. **Create a NEW directory inside this same repository folder**:
   - Put the entire macOS port in a new sibling directory **`./macos/`** at the repository root (alongside `./rust/`, `./docs/`, `./build.py`, etc.).
   - **Do NOT modify, overwrite, or break anything inside `./rust/`** or the Windows build scripts (`build.py`, `build.ps1`), except if you need a root-level helper or documentation note that does not alter Windows behavior. Treat `./rust/` as the read-only canonical specification and source of truth.
   - Inside `./macos/`, mirror the Tauri v2 layout (`macos/package.json`, `macos/src/`, `macos/public/`, `macos/src-tauri/`, `macos/tools/`, `macos/build_mac.py` or `build.sh`) so the macOS app builds, tests, and bundles cleanly and independently as a Universal / Apple Silicon macOS `.app` and `.dmg`.
2. **Engine Requirement — WebKit Only**:
   - All browser tabs, the main UI shell, the AI assistant drawer, and the probe profile must run on **WebKit (`WKWebView` via Tauri v2 / `wry` + direct `objc2` / `objc2-web-kit` / `objc2-app-kit` / `objc2-foundation` bindings)** — no Chromium/CEF/Electron/WebView2 runtime dependencies.
3. **1:1 Parity + Native macOS Ergonomics**:
   - Every user-facing feature, UI surface, animation feel, IPC command, background service, persistence format, and keyboard workflow must work 1:1 with `./rust/`.
   - Where Windows OS primitives have no direct analog or macOS requires OS conventions (e.g., `Cmd` shortcuts, native Cocoa Edit menu for `Cmd+C/V/X/A/Z`, Retina `backingScaleFactor`, bottom-left AppKit screen coordinates, traffic-light or frameless window controls, `Finder` instead of `Explorer`, Keychain instead of DPAPI), implement the true macOS native equivalent seamlessly.
4. **Do Not Stub Broken Features Silently**:
   - Every core system must be genuinely implemented and tested on macOS. Where WebKit has a hard platform boundary (specifically Chrome `.crx` MV3 extension APIs and WebView2 CDP), use the genuine WebKit/macOS mechanisms detailed in Section 4 below so the feature works cleanly without panics, dead buttons, or silent lies.

---

## 1. Inspect the Canonical Source First

Before writing code, read and trace the Windows reference implementation inside `./rust/` and the root engineering docs:
- Root docs & tooling: `README.md`, `serica_engineering_directive.md`, `build.py`
- Frontend (`./rust/src/`):
  - Core shell & bridge: `App.tsx`, `types.ts`, `adapter/tauriBridge.ts`, `shared/layout.ts`, `shared/settings.ts`
  - Hooks: `hooks/useBrowser.ts`, `hooks/useViewportOverlay.ts`, `hooks/useTopBarAutoHide.ts` (+ other hooks in `hooks/`)
  - Components: `BrowserViewport.tsx`, `TopTabStrip.tsx`, `Sidebar.tsx`, `RightPinsDock.tsx`, `CommandBar.tsx`, `Homescreen.tsx`, `ToolsMenu.tsx`, `McpModal.tsx`, `ExtensionsModal.tsx`, `DownloadsPanel.tsx`, `PageContextMenu.tsx`, `AppContextMenu.tsx`, `PageDialog.tsx`, `PermissionPrompt.tsx`, `PasswordPrompt.tsx`, `AuthHandoffNotice.tsx`, `DropZones.tsx`, `FloatingUrlPill.tsx`, `Setup.tsx`, `ProductTour.tsx`, `CookieSettings.tsx`, `AboutSettings.tsx`, `settings/Settings2.tsx`, `settings/ImportPanel.tsx`, `settings/parts.tsx`
  - Styles & assets: `styles/chrome.css`, `styles/window-controls.css`, `styles/theme.css`, `src/fonts/`, `public/artworks/`
- Rust Backend (`./rust/src-tauri/src/`):
  - Entry & state: `lib.rs`, `main.rs`, `app.rs`, `commands.rs`, `model.rs`, `store.rs`, `settings.rs`, `window_state.rs`
  - WebView host & Windows native modules: `tabhost.rs`, `webview_win.rs`, `clip_win.rs`, `dragdrop_win.rs`, `edge_watch.rs`
  - Subsystems: `shields.rs`, `vpn.rs`, `focus.rs`, `focus_stats.rs`, `downloads.rs`, `extensions.rs`, `browser_import.rs`, `default_browser.rs`, `local_ai.rs`, `mcp.rs`, `mcp_http.rs`, `bookmarks.rs`, `history.rs`, `widgets.rs`, `backgrounds.rs`, `files.rs`, `previews.rs`, `updater.rs`, `ui_patch.rs`, `importer.rs`
  - Config: `Cargo.toml`, `tauri.conf.json`, `tauri.test.conf.json`, `capabilities/default.json`

---

## 2. Target Structure in `./macos/`

Create `./macos/` mirroring `./rust/` cleanly:
```text
macos/
├── package.json
├── tsconfig.json
├── vite.config.ts
├── index.html
├── build_mac.py                 # macOS build/dev/test/bundle script (like root build.py)
├── public/
│   └── artworks/                # Copy Serica Special artworks (serica-special-{1,2,3}.avif & -thumb.webp)
├── src/                         # React 18 + TS 5.7 + Vite 6 frontend (adapted for macOS shortcuts/fonts/copy)
└── src-tauri/
    ├── Cargo.toml               # Same cross-platform crates + macOS target deps (objc2, core-foundation, etc.)
    ├── tauri.conf.json          # identifier: com.serica.browser, macOS bundle targets, Info.plist schemes
    ├── tauri.test.conf.json     # Isolated test bundle ID (com.serica.browser.test / "Serica Test")
    ├── capabilities/default.json
    ├── icons/                   # Include .icns alongside PNG icons
    └── src/
        ├── lib.rs
        ├── main.rs
        ├── app.rs
        ├── commands.rs
        ├── tabhost.rs
        ├── webview_mac.rs       # Replaces webview_win.rs (WKWebView + delegates + JS bridges)
        ├── clip_mac.rs          # Replaces clip_win.rs (NSView / CALayer CAShapeLayer mask + CVDisplayLink)
        ├── dragdrop_mac.rs      # Replaces dragdrop_win.rs (NSDraggingDestination + NSDraggingSession)
        ├── edge_watch_mac.rs    # Replaces edge_watch.rs (NSEvent::mouseLocation + NSWindow geometry)
        ├── menu_mac.rs          # Native macOS NSMenu (Edit/Copy/Paste/SelectAll/Undo/Redo + App menu)
        ├── default_browser.rs   # macOS LaunchServices / NSWorkspace default browser + UTType handlers
        ├── browser_import.rs    # macOS browser paths + Safari importer + Keychain AES password decrypt
        ├── ... (cross-platform modules adapted cleanly for macOS paths/OS APIs)
```

Also inspect `./rust/tools/tests/` (used by `npm test` -> `node tools/tests/run.mjs`) and copy/adapt the frontend test suite into `./macos/tools/tests/` so `npm test` runs cleanly in `./macos/`.

---

## 3. Complete Feature & Architecture Checklist (Must Preserve 1:1)

Ensure every single feature from `./rust/` is present and functional in `./macos/`:

1. **Tauri Shell & Custom Protocols**:
   - Window: `1280x800` default (min `800x600`), frameless / transparent (`decorations: false` or integrated macOS titlebar strategy — see Section 5), `shadow: true`, `dragDropEnabled: false`.
   - `tauri-plugin-single-instance`, `tauri-plugin-opener`, `tauri-plugin-updater`.
   - Custom URI schemes registered in `lib.rs`:
     - `sericabg` (`http://sericabg.localhost/<file>`) serving custom homepage backgrounds.
     - `sericacap` (`http://sericacap.localhost/<tab_id>/<ver>.jpg`) serving tab captures, strictly gated to `req_webview == "main"` (or `webview_label() == "main"`).
   - Capabilities (`capabilities/default.json`) restricting Tauri IPC exclusively to `"main"` so loaded websites in child webviews cannot call Tauri backend commands.
   - Internal URLs: `serica://newtab`, `neutrino://newtab`, `serica://settings`, `serica://history`, `serica://downloads`, `serica://extensions`, `serica://vpn`.
   - Legacy data migration (`migrate_legacy_data`: `neutrino` -> `serica` app data directory) adapted to `~/Library/Application Support/` (`app_data_dir` / `app_local_data_dir`).

2. **Spaces, Profiles & Tab Management (`store.rs`, `model.rs`, `app.rs`)**:
   - Spaces with icons, colors, custom layout/theme overrides, pinned tabs (grouped in folders or top favs), Today tabs, Archive with auto-archive timer (30s sweep), Zen Sleep after configurable idle time, and memory-pressure sleeping (`pressure_sleep_plan` triggered when system RAM usage exceeds 85%).
   - Multiple isolated Profiles with separate browser storage dirs/identifiers, cookie policy (`allow`, `block-third-party`, `block-all`), and private (`incognito`) ephemeral tabs.
   - All tab lifecycle events (`TabEvent`: `Title`, `Url`, `Favicon`, `Loading`, `Audio`, `Fullscreen`, `Zoom`, `Error`, `Crashed`, `Find`, `Pdf`, `Blocked`), undo closed tab (`reopen_closed`), mute/unmute, zoom per host/tab, live capture (`sericacap`), tab hover preview cards, duplicate/move between spaces.
   - Full Keyboard Shortcuts (adapted to macOS `Cmd` + `Ctrl`/`Opt` conventions where appropriate — see Section 5).

3. **Two Navigation Layouts & Chrome (`Sidebar.tsx`, `TopTabStrip.tsx`, `RightPinsDock.tsx`, `useTopBarAutoHide.ts`)**:
   - `Sidebar` layout (`serica_engineering_directive.md` standard): compact/expandable left rail, **Control Absorption** (single collapsed control cluster vs expanded horizontal bar), Favorites grid, Pinned folders & items, Today list, **Two-finger horizontal swipe between Spaces** (`useSpaceSwipe`), hover edge peek when hidden.
   - `TopTabs` (`top-tabs`) layout: 35px top tab strip, auto-hide top bar in fullscreen/focus (`useTopBarAutoHide`), **RightPinsDock** with macOS Dock-style cursor magnification.
   - **Viewport Overlay & Rounded-Rect Hole Cutting (`useViewportOverlay.ts` + `BrowserViewport.tsx` + `clip_mac.rs`)**:
     - Whenever modals, dropdowns, context menus, command bar, hover cards, or slide-over drawers sit over the active webview, the main UI paints on top or punches animated rounded-rect clipping holes synced to display refresh (`60/120Hz` ProMotion on MacBook!) plus optional JPEG viewport capture freeze (`viewport_freeze` / `viewport_capture`).

4. **Command Bar (`CommandBar.tsx`)**:
   - `Cmd+K` / `Cmd+L` centered spotlight-style omnibox with instant search across bookmarks, open tabs (switch-to-tab), browsing history, top-domain autocomplete, and `>` command palette actions (e.g., toggle Focus Mode, AI assistant, Theme Studio, Settings, Splits/Layouts, Clear cookies, DevTools, New Private Tab, Zoom, etc.).
   - 7 search engines (Google, DuckDuckGo, Bing, Brave, Kagi, Perplexity, Ecosia) + configurable default.

5. **Homescreen / New Tab (`Homescreen.tsx`, `widgets.rs`, `backgrounds.rs`)**:
   - Minimal bottom-corner live clock + clean widget canvas (no heavy glow/greeting header/centered searchbar unless user configures widgets).
   - Apple-style widget grid with drag/reorder/resize, Clock, Weather, Notes, Bookmarks/Quick Links, Focus Timer/Stats, Search, Custom Widgets.
   - 4 background modes including **Serica Special** (3 French Revolution oil-pastel artworks `serica-special-{1,2,3}.avif` slideshow), Solid, Gradient, Custom uploaded image/video via `sericabg://` (preserving `backgrounds.rs` limits/formats).

6. **Theme Studio & Settings (`Settings2.tsx`, `Theme` state, `CookieSettings.tsx`, `AboutSettings.tsx`, `ImportPanel.tsx`)**:
   - Search-inspired Settings UI with live search across all sections, Theme Studio (accents, surface glass/tint, typography fonts including `Inter`, `AnthropicSans`, `Outfit`, `FiraSans`, `InstrumentSerif`, `Fraunces`, `ClaudeSerif`, `PixelifySans`, compact/comfortable density, UI text scale).
   - Cookie manager per site (`cookie_sites`, `delete_cookies`, `sweep`), About page with live updater check + UI hot-patch indicator.
   - Import wizard (`ImportPanel.tsx`) supporting Chrome, Edge, Brave, Vivaldi, Opera, Arc, Firefox, Zen, LibreWolf, Waterfox, **Safari**, plus CSV bookmarks/passwords.

7. **Focus Mode & Focus Stats (`focus.rs`, `focus_stats.rs`)**:
   - Cold Turkey-style site/domain/URL/keyword blocker with active timers, recurring schedules, blocklists, `whole_site` blocking vs page blocking, `Strict` / `RandomText` unlock challenges, and `BLOCK_PAGE` HTML.
   - Tamper-resistant `TrustedClock` combining monotonic instant + wall clock + periodic network `Date` header verification.
   - 400-day persistent `focus-stats.json` tracking daily focus time, browsing/site time, blocked attempts, foreground activity sampling (`sample_browsing`), and CSV export.

8. **Shields (`shields.rs`)**:
   - Brave `adblock-rust` (`0.13`) engine with 28 built-in filter lists + Brave/uBO scriptlets, compiled `engine.dat` cache, 6h background updater, `BUILTIN_RULES`, per-site allowlist, cosmetic element hiding CSS, scriptlet injection, blocked request counter & live log per tab (`shield_stats`), and aggressive/standard levels.

9. **Privacy Tunnel / VPN (`vpn.rs`)**:
   - Built-in local HTTP CONNECT/forwarding proxy on `127.0.0.1:<port>` supporting: `doh-only` (DNS-over-HTTPS RFC 8484), `tor-local` (SOCKS5 `127.0.0.1:9050`/`9150`), `custom-proxy` (HTTP/SOCKS5 with optional auth), and `system-vpn` detection (`detect_os_vpn`).
   - Kill switch (`kill_switch`) and `WEBRTC_GUARD_JS` leak protection injected at document start.

10. **Local AI, AI Assistants Drawer & MCP Server (`local_ai.rs`, `mcp.rs`, `mcp_http.rs`, `McpModal.tsx`)**:
    - Auto-detection & model listing for local Ollama (`11434`), LM Studio (`1234`), and custom OpenAI-compatible endpoints, plus page summarization / tab assistant integration.
    - Embedded AI side drawer (`assistant` child `WKWebView` opened via `assistant_open`/`assistant_bounds`/`assistant_close` supporting providers like Claude, ChatGPT, Gemini, Perplexity, etc.).
    - Built-in MCP Streamable HTTP server on `127.0.0.1:7531/mcp` (`mcp_http.rs`, `mcp.rs`) with live approval prompts, tool permissions, and Claude Desktop / Cursor / VS Code config snippets in `McpModal.tsx`.

11. **Downloads, History, Extensions, Onboarding & Updater**:
    - `downloads.rs`: Native download interception (`downloads.json`), smoothed speed + ETA, 250ms UI progress throttle, pause/resume/cancel, reveal in Finder, open file, plus **drag downloaded file chip directly from Serica into macOS apps/Finder** (`start_download_drag`).
    - `history.rs`: Full SQLite/JSON persistent searchable history + top sites/visit frequency for omnibox ranking.
    - `extensions.rs`: Extensions manager (`ExtensionsModal.tsx`, `serica://extensions`) + Web Store detection (`WEBSTORE_SCRIPT`) + WebKit extension integration (see Section 4.10).
    - Onboarding: First-run/upgrade `Setup.tsx` (non-destructive import + theme + default browser + profile migration), `ProductTour.tsx`, `AuthHandoffNotice.tsx`.
    - `updater.rs` + `ui_patch.rs`: Minisign-verified hot UI patcher and Tauri app auto-updater (`latest.json` target `darwin-aarch64` / `darwin-x86_64`).

12. **Native Page Integrations**:
    - Right-click `PageContextMenu.tsx` merging Serica tab actions (Open in new/background tab, Open image, Search with default engine, Copy link/image, View source, Inspect Element) with WebKit native editing/lookup/spelling actions (`finish_page_menu`).
    - `PageDialog.tsx` handling `alert`, `confirm`, `prompt`, `beforeunload`, and external URI scheme launch confirmation (`LaunchingExternalUriScheme` equivalent).
    - `PermissionPrompt.tsx` handling site permissions (camera, microphone, geolocation, notifications, clipboard, autoplay, etc.) persisted in `settings.site_permissions`.
    - `DropZones.tsx` accepting dragged files, URLs, and text from Finder/Safari/other apps onto the Serica window.
    - `FindBar` (matching count, active match index, next/prev, case sensitivity).
    - `PDF_DARK_SCRIPT` dark-mode styling for PDF viewer tabs.
    - `ERROR_PAGE_JS` custom Serica branded error page with reload/diagnostics.

---

## 4. Exact Windows-to-macOS / WebKit Technical Blueprint

Implement these specific replacements so there are zero missing subsystems or WebView2 leaks:

### 4.1 Child `WKWebView` Architecture & Layering (`tabhost.rs`, `webview_mac.rs`, `clip_mac.rs`)
On Windows, WebView2 uses child `HWND`s (`WRY_WEBVIEW`) clipped via `SetWindowRgn` + `DwmFlush`.
On macOS:
- Use `wry` / Tauri v2 child webviews (`window.add_child(...)`) OR manage child `WKWebView`s (`NSView` subviews of the `NSWindow`'s `contentView`).
- **Clipping & Animated Holes (`clip_mac.rs` replacing `clip_win.rs`)**:
  - Replace Win32 `SetWindowRgn` + `DwmFlush` with `NSView` layer masking: set `wantsLayer = true` on the child `WKWebView` container `NSView`, attach a `CAShapeLayer` as `layer.mask`, and build an **even-odd `CGPath`** (`kCAFillRuleEvenOdd`) equal to `bounds` minus each `Hole` rounded rectangle (`CGPathAddRoundedRect` matching `radius` in logical points — note Cocoa points match CSS pixels directly!).
  - Drive smooth hole transitions (`cubic_bezier`) using **`CVDisplayLink`** (or **`CADisplayLink`** on macOS 14+) dispatched to the main thread (`NSRunLoop` / GCD main queue) so animations run smoothly at 60Hz or 120Hz ProMotion without tearing.
  - Because WebKit composites in-process/GPU via CoreAnimation, ensure `CAShapeLayer` updates happen inside `CATransaction.begin(); CATransaction.setDisableActions(true); ... CATransaction.commit()` so CoreAnimation does not add an unwanted implicit 250ms fade/lag on top of Serica's custom `cubic_bezier` curve!
- **Snapshot / Freeze / Hover Previews (`capture`)**:
  - Replace WebView2 `CapturePreview` with `WKWebView.takeSnapshot(with: WKSnapshotConfiguration)` converted to JPEG bytes (`NSBitmapImageRep` with JPEG compression factor) for `sericacap://` and `viewport_freeze`.
- **Memory Management (`schedule_suspend`, `app.rs::memory_load_percent`)**:
  - Replace Win32 `GlobalMemoryStatusEx` in `app.rs` with Mach **`host_statistics64`** (`HOST_VM_INFO64` / `vm_statistics64`) + `sysctlbyname("hw.memsize")` to calculate real RAM pressure (`active + wired + compressed` / `total`) for `pressure_sleep_plan`.
  - For tab suspension (`try_suspend`), `WKWebView` has no Chromium `TrySuspend`; pause all media (`pauseAllMediaPlayback` on macOS 12+), optionally evaluate pause timers/visibility override, or rely on `Store` Zen sleep (unloading inactive webviews cleanly while preserving tab state & snapshot).

### 4.2 Per-Profile Isolation, Incognito & Proxy / VPN Routing (`WKWebsiteDataStore`)
WebView2 accepted Chromium CLI flags (`--proxy-server`, `--proxy-bypass-list`, `.data_directory()`). **`WKWebView` ignores Chromium flags!**
- **Profiles & Ephemeral Tabs**:
  - On macOS 14+ (Sonoma), use `[WKWebsiteDataStore dataStoreForIdentifier:uuid]` (deriving a deterministic UUID v5 from Serica's `profile_id`) on `WKWebViewConfiguration.websiteDataStore` so each Serica profile gets an isolated persistent cookie/IndexedDB/localStorage store managed by WebKit. (Store a fallback map if supporting macOS 13, or set minimum target **macOS 14.0** and document this cleanly in `Info.plist` `LSMinimumSystemVersion = 14.0` — recommended because macOS 14 also gives native per-WebView proxy configs!).
  - For Private (`incognito`) tabs, use `WKWebsiteDataStore.nonPersistentDataStore()`.
- **Cookie Manager (`cookie_sites`, `delete_cookies`, `sweep`)**:
  - Replace WebView2 `ICoreWebView2CookieManager` with `WKWebsiteDataStore.httpCookieStore` (`getAllCookies`, `deleteCookie`, `removeDataOfTypes:forDataRecords:`).
- **Third-Party Cookie Policy**:
  - `WKWebsiteDataStore` / `HTTPCookieStorage` + WebKit ITP (Intelligent Tracking Prevention is on by default in `WKWebView`); apply explicit third-party cookie blocking / allow rules via injected storage/cookie guard + `WKHTTPCookieStore` filtering according to profile `CookiePolicy`.
- **VPN / Proxy Routing (`vpn.rs` integration)**:
  - On macOS 14+, configure `WKWebsiteDataStore.proxyConfigurations` using Network framework `nw_proxy_config_create_http_connect(...)` pointing to `127.0.0.1:<vpn_port>` (and clear `proxyConfigurations` when `vpn` proxy is disabled).
  - Test that toggling DoH-only, Tor local, Custom Proxy, and Kill Switch immediately applies to both existing and newly created `WKWebView`s via `sync_network()`.

### 4.3 Shields (`adblock-rust`) & Focus Mode Request Blocking on WebKit
WebView2 has synchronous `WebResourceRequested`, which `WKWebView` does **not** expose for `http://`/`https://` subresources. Do **both** of the following so Shields and Focus Mode have real, rock-solid parity:
1. **Top-level & Subframe Navigation Interception (`WKNavigationDelegate`)**:
   - In `decidePolicyForNavigationAction:decisionHandler:`, run `focus::check_url(...)` and download/external-scheme checks synchronously before allowing any navigation.
   - If blocked by Focus Mode, cancel the policy decision and load `focus::block_page(...)` HTML (`loadHTMLString:baseURL:` or custom error state), and update tab URL/state cleanly.
   - Also observe `WKWebView.URL` via KVO (`SourceChanged` equivalent) + `didCommitNavigation` / `didFinishNavigation` / `history` `pushState`/`replaceState` hook in `PAGE_SCRIPT` -> call `focus_recheck` so SPA URL changes (YouTube Shorts, X, Reddit) trigger Focus Mode immediately.
2. **Subresource Adblocking & `whole_site` Focus Mode**:
   - **WebKit Content Blocking (`WKContentRuleListStore`)**: Convert Serica's active `adblock-rust` rules (using `adblock::Engine::convert_rules_to_content_blocking` if enabled/available, or generating Safari Content Blocker JSON from `shields.rs` filter lists + Focus `whole_site` rules, chunked under WebKit's per-list rule limit and compiled asynchronously into `WKContentRuleListStore.default()`), then attach to `WKUserContentController.addContentRuleList(...)` on non-whitelisted tabs!
   - **Local Proxy Inspection / `$redirect` & Shield Live Log**: Because `WKContentRuleList` blocks silently in WebKit without reporting each blocked subresource URL to your Rust counter, **also** run subresource interception when appropriate OR combine:
     - `WKUserScript` at `DocumentStart` (`forMainFrameOnly: false`) providing `shield_stats` / cosmetic hiding / Brave scriptlets / `fetch` & `XHR` telemetry (`window.webkit.messageHandlers.serica.postMessage(...)`), AND/OR
     - Routing through `vpn.rs`'s local proxy when `$redirect` / exact network-level blocking + live per-domain block counting is active (or checking blocked URLs cleanly). Clearly ensure toggling Shields on/off per site immediately reloads/updates `WKContentRuleList` + cosmetic CSS + scriptlets and increments live `blocked` count in the Shields UI!

### 4.4 User Scripts, Message Bridge & Custom Error/PDF Scripts
- Replace `AddScriptToExecuteOnDocumentCreated` with `WKUserContentController.addUserScript(WKUserScript(source, injectionTime: .atDocumentStart, forMainFrameOnly: ...))`.
- Replace `window.chrome.webview.postMessage(...)` across `PAGE_SCRIPT`, `SWIPE_SCRIPT`, `WEBSTORE_SCRIPT`, `WEBRTC_GUARD_JS`, `MEDIA_FIND_JS` with `window.webkit.messageHandlers.serica.postMessage(...)` (implement a `WKScriptMessageHandler` named `"serica"`, or define a tiny shim at the top of injected scripts if helpful, while isolating it so untrusted pages only reach typed handlers for `settled`, `swipe-nav`, `ext-install`, `ext-query`, `cosmetic`, `audio-state`, `find-result`, `selection`).
- **Two-Finger Swipe Navigation**:
  - Enable `webView.allowsBackForwardNavigationGestures = true` on macOS `WKWebView` for authentic macOS two-finger back/forward trackpad swipe, AND verify it does not conflict with `useSpaceSwipe` when the cursor is over the Sidebar vs web viewport!
- **Custom Error Page (`ERROR_PAGE_JS`)**:
  - Catch `didFailProvisionalNavigation` and `didFailNavigation` (ignoring `-999` `NSURLErrorCancelled`!). Inject/render Serica's styled offline/SSL/DNS error page (`loadHTMLString` with `failingURL` preserved cleanly or Serica error state) and emit `TabEvent::Error`.
- **PDF Dark Mode (`PDF_DARK_SCRIPT`)**:
  - Note: WebKit renders PDFs via macOS PDFKit inside `WKWebView` rather than Chromium's `#toolbar` PDF viewer. Verify whether CSS inversion (`mix-blend-mode` / `filter: invert(0.9) hue-rotate(180deg)` overlay `NSView` or CSS in `WKWebView` / `PDFView` layer filter) cleanly darkens PDFs without inverting images/UI badly, and trigger `TabEvent::Pdf` when `MIME` is `application/pdf` (`decidePolicyForNavigationResponse`).

### 4.5 Audio, Multi-Frame Media & Picture-in-Picture (No CDP)
WebKit has no Chrome DevTools Protocol (`Target.setAutoAttach` / `Runtime.evaluate`).
- **Audio Indicator**: Poll or observe audio playback across frames via injected lightweight `<audio>`/`<video>` play/pause/volume listeners in `forMainFrameOnly: false` `WKUserScript` posting `{ kind: "audio", playing: bool }` (plus `WKWebView` media state APIs where available on macOS 12+/14+) -> emit `TabEvent::Audio`.
- **Media Control & Auto-PiP (`media_eval`, `pip_enter_js`, `PIP_EXIT_JS`)**:
  - Iterate `WKFrameInfo` or inject `WKUserScript` with `forMainFrameOnly: false` so media play/pause/seek/mute commands work inside embedded `<iframe>` players too!
  - In `pip_enter_js` / `PIP_EXIT_JS`, support both standard `requestPictureInPicture()` / `exitPictureInPicture()` AND WebKit's native macOS API: `video.webkitSupportsPresentationMode?.('picture-in-picture')` -> `video.webkitSetPresentationMode('picture-in-picture')` / `'inline'`, triggered from a user-gesture / `evaluateJavaScript` path.

### 4.6 Native Context Menus, Dialogs, Permissions, Downloads & Find-in-Page
- **Page Context Menu (`PageContextMenu.tsx`)**:
  - Subclass `WKWebView` (or attach runtime subclass/swizzle on `wry`'s `WKWebView`) overriding `willOpenMenu(_ menu: NSMenu, with event: NSEvent)`:
    - Inspect the `NSMenu` items WebKit generated + query hit-test info via JS (`linkURL`, `srcURL`, `mediaType`, `selectionText`, `isEditable`).
    - If `MenuKind` or user right-clicked normally: capture WebKit's native action items (`Copy`, `Paste`, `Cut`, `Look Up`, `Translate`, `Inspect Element`, `Save Image`, etc.) into a thread-safe `PENDING_MENUS` map, **clear/suppress the native popup** (e.g., `menu.removeAllItems()` or dismiss cleanly), and emit `serica:page-context-menu` to React's `PageContextMenu.tsx` with the exact same item schema as `webview_win.rs`!
    - On `finish_page_menu`, trigger the retained native `NSMenuItem` target/action (`NSApp.sendAction:to:from:`) or handle Serica's custom items (`open-tab`, `open-bg-tab`, `open-image`, `search-selection`, `view-source`, `devtools`).
    - Verify `Inspect Element` opens WebKit's built-in **Web Inspector** (`webView.configuration.preferences.setValue(true, forKey: "developerExtrasEnabled")` + `isInspectable = true` on macOS 13.3+).
- **JS Dialogs & External Schemes (`PageDialog.tsx`)**:
  - Implement `WKUIDelegate`: `runJavaScriptAlertPanelWithMessage`, `runJavaScriptConfirmPanelWithMessage`, `runJavaScriptTextInputPanelWithPrompt` routing through `serica:page-dialog` and resuming the completion handler on `answer_page_dialog` (guaranteeing completion handlers are NEVER leaked or called twice, even if tab closes!).
  - In `WKNavigationDelegate` (`decidePolicyForNavigationAction`), intercept non-`http`/`https`/`about`/`blob`/`data`/`serica`/`file` schemes (e.g. `mailto:`, `vscode:`, `zoommtg:`, `slack:`), emit `serica:page-dialog` kind `"external"`, and open via `NSWorkspace.shared.open(url)` if confirmed.
- **Permissions (`PermissionPrompt.tsx`)**:
  - Implement `WKUIDelegate`: `requestMediaCapturePermissionFor` (camera/mic/screen), `requestDeviceOrientationAndMotionPermissionFor`, plus geolocation/notification gating synced with `settings.site_permissions`. Note macOS `Info.plist` MUST include `NSCameraUsageDescription`, `NSMicrophoneUsageDescription`, `NSLocationWhenInUseUsageDescription`, `NSDownloadsFolderUsageDescription` so macOS doesn't crash the app when websites request them!
- **Downloads (`downloads.rs`)**:
  - Handle `navigationAction:didBecomeDownload:` and `navigationResponse:didBecomeDownload:` in `WKNavigationDelegate`, set `download.delegate = DownloadDelegate`.
  - In `WKDownloadDelegate`: decide destination path using `downloads.rs` collision naming & auto-folder, observe `download.progress` (`fractionCompleted`, `completedUnitCount`, `totalUnitCount`) with 250ms throttling, support cancel + pause/resume (`cancel { resumeData in ... }` + `resumeDownloadFromResumeData:`), and `reveal` via `NSWorkspace.shared.activateFileViewerSelectingURLs` (or `open -R`).
- **Find in Page (`find_in_page`)**:
  - `WKWebView.findString` only highlights one match without returning total match count. Implement a fast injected Range + `CSS.highlights` (`Highlight` API, supported in Safari/WebKit 17.2+!) or highlight wrapper across frames that highlights **all** matches, scrolls to the active match, and returns exact `{ active, total }` to `FindBar.tsx` + updates live on next/prev/query change and cleans up on close.

### 4.7 OS Cursor, Edge Approach, Drag & Drop, and Clipboard (`edge_watch_mac.rs`, `dragdrop_mac.rs`, `commands.rs`)
- **Coordinate System Warning (CRITICAL)**:
  - AppKit screen coordinates have `(0, 0)` at the **bottom-left** of the primary display (`NSScreen.screens[0].frame`), while `WKWebView` / CSS / Windows coordinates have `(0, 0)` at **top-left**. Always convert: `topLeftY = primaryScreenHeight - cocoaY` and convert `NSWindow.frame` / `contentLayoutRect` consistently!
- **`edge_watch_mac.rs`**:
  - Replace Win32 `GetCursorPos`/`GetForegroundWindow`/`GetDpiForWindow` polling with `NSEvent::mouseLocation()` + `NSWindow` active/key check + frame bounds check every 16ms when active, emitting `serica:edge-approach` (`left`, `right`, `top`) and returning logical CSS coordinates from `ui_cursor` (scaled appropriately relative to `backingScaleFactor` vs CSS pixels).
- **`dragdrop_mac.rs`**:
  - Drop-in: Register `NSDraggingDestination` on the window's content view / main webview container (`registerForDraggedTypes`: `NSPasteboardTypeFileURL`, `NSPasteboardTypeURL`, `NSPasteboardTypeString`) emitting `serica:drag` events (`enter`, `over`, `leave`, `drop`) with `{ files, url, text, x, y }` so `DropZones.tsx` works 1:1.
  - Drag-out (`start_download_drag`): Use `NSView.beginDraggingSessionWithItems:event:source:` with an `NSURL` file pasteboard item (`NSDraggingItem`) so users can drag completed download chips from `DownloadsPanel.tsx` directly onto macOS Desktop/Finder/Slack/Mail.
- **Clipboard (`read_clipboard_text`)**:
  - Replace Win32 `OpenClipboard` with `NSPasteboard.generalPasteboard().stringForType(NSPasteboardTypeString)`.
- **File Picker & Open-With-Serica (`files.rs`, `backgrounds.rs`)**:
  - Ensure `NSOpenPanel` / `NSSavePanel` (or `rfd` using Cocoa) works smoothly without blocking the tokio executor, and implement `NSApplicationDelegate` `application(_:open:)` (`openURLs` / `openFiles`) so opening `.html`/`.pdf` or `http(s)://` links from Finder/Spotlight/other apps launches or opens tabs in Serica properly via single-instance forwarding.

### 4.8 Default Browser, Browsers Import, Focus Sampling & VPN Detection
- **`default_browser.rs`**:
  - Check default browser using `LSCopyDefaultHandlerForURLScheme("https")` / `NSWorkspace.shared.urlForApplicationToOpenURL("https://example.com")` comparing bundle ID `com.serica.browser`.
  - Set default browser via `NSWorkspace.shared.setDefaultApplicationAtURL` (macOS 12+) / `LSSetDefaultHandlerForURLScheme` for `http` & `https` + `UTType` (`public.html`, `com.adobe.pdf`), which presents macOS's native confirmation dialog.
- **`browser_import.rs`**:
  - Update macOS browser paths under `~/Library/Application Support/`:
    - Google Chrome (`Google/Chrome`), Chrome Canary, Microsoft Edge (`Microsoft Edge`), Brave (`BraveSoftware/Brave-Browser`), Vivaldi (`Vivaldi`), Opera (`com.operasoftware.Opera`), Arc (`Arc/User Data`), Firefox (`Firefox`), Zen (`zen`), LibreWolf (`LibreWolf`), Waterfox (`Waterfox`).
    - **Add macOS Safari importer**: read `~/Library/Safari/Bookmarks.plist` (binary/XML plist via `plist` crate -> recursive `WebBookmarkTypeList`/`WebBookmarkTypeLeaf`) and `~/Library/Safari/History.db` (`history_items` + `history_visits` in CoreData timestamps — note macOS Full Disk Access prompt gracefully handled with clear message if TCC denies Safari `History.db`!).
  - **Chromium macOS Keychain Password Decryption**:
    - Replace DPAPI `CryptUnprotectData` with macOS Security framework Keychain lookup (`SecKeychainFindGenericPassword` for `"Chrome Safe Storage"` / `"Brave Safe Storage"` / `"Microsoft Edge Safe Storage"` / etc.), derive AES-128-CBC key using PBKDF2-HMAC-SHA1 (`salt = b"saltysalt"`, `1003` iterations, 16-byte key, IV = 16 spaces `b"                "`), decrypt `"v10"`/`"v11"` prefixes cleanly!
    - Also inspect `WKWebView` / macOS password story: keep imported/CSV exportable passwords in Serica's store or macOS Keychain cleanly without breaking `ImportPanel.tsx` counts/results.
- **`focus_stats.rs` (`sample_browsing`)**:
  - Replace `GetForegroundWindow` + `GetLastInputInfo` with `NSApplication.shared.isActive` (or `NSWorkspace.sharedApplication.frontmostApplication.bundleIdentifier == "com.serica.browser"`) and `CGEventSourceSecondsSinceLastEventType(kCGEventSourceStateCombinedSessionState, kCGAnyInputEventType)` (idle threshold matching Windows).
- **`vpn.rs` (`detect_os_vpn`)**:
  - Replace Win32 `GetAdaptersAddresses` with macOS `getifaddrs` (detecting active `utun*` with non-link-local IP or non-Apple VPN interface, `ipsec*`, `ppp*`, `wg*`, `tailscale*`) supplemented by `scutil --nc list` / `SystemConfiguration` check for active macOS VPN tunnels.

### 4.9 Extensions (`extensions.rs` & `ExtensionsModal.tsx`)
WebKit does **not** run Chromium `.crx` MV2/MV3 `chrome.*` APIs natively like WebView2's `ICoreWebView2Profile7::AddBrowserExtension`.
Handle this cleanly and honestly without crashing or leaving broken UI:
- Keep Serica's Extensions manager (`serica://extensions`, `ExtensionsModal.tsx`) working cleanly:
  - **Content Scripts / Userscript / Safari Web Extension Path**:
    - Parse `manifest.json` (`content_scripts` matches, JS/CSS files, `_locales`, icons).
    - If running on **macOS 15.4+**, wire `WKWebExtensionController` + `WKWebExtension` when an extension bundle is compatible, AND/OR automatically inject parsed `content_scripts` (`matches`, `exclude_matches`, `run_at`, `all_frames`, JS & CSS) via `WKUserContentController` so content-script extensions work on **all** macOS 14+ versions!
    - For Chrome Web Store (`WEBSTORE_SCRIPT`) `.crx` downloads that depend on unsupported Chromium-only background APIs (`chrome.webRequest`, native binary hosts, etc.), surface a clear, accurate status indicator in `ExtensionsModal.tsx` (e.g., `"Content scripts active"` vs `"Requires Chromium extension runtime (unsupported on WebKit)"`) rather than silently pretending background service workers work.

---

## 5. macOS Window Chrome, Shortcuts, Fonts & UI Polish

Pay close attention to these critical macOS details so the browser feels 100% native on a MacBook while staying 1:1 with Serica:

1. **CRITICAL — Cocoa Application Edit Menu (`menu_mac.rs`)**:
   - In Tauri/WKWebView on macOS, if the app does not install an `NSApplication` main `NSMenu` containing standard `Edit` menu items (`Undo`, `Redo`, `Cut`, `Copy`, `Paste`, `Select All`), **`Cmd+C`, `Cmd+V`, `Cmd+X`, `Cmd+A`, `Cmd+Z` will just beep and do nothing inside `<input>`s, `<textarea>`s, and websites!**
   - Build a clean macOS menu bar in `menu_mac.rs` (App menu with About/Settings/Hide/Quit + File/Edit/View/Window) using Tauri's `Menu` / `PredefinedMenuItem` or `NSMenu`, and verify `Cmd+C`/`Cmd+V`/`Cmd+X`/`Cmd+A`/`Cmd+Z` work inside BOTH the main UI (CommandBar, Settings, Notes widget) AND child `WKWebView` pages!

2. **Window Chrome & Traffic Lights (`App.tsx`, `window-controls.css`, `window_state.rs`)**:
   - Notice `./rust/src/App.tsx` already contains `.macos-tabs-toggle-pill` and Windows-style `.wctl` controls on the right.
   - On macOS, adapt cleanly while preserving 1:1 visual hierarchy:
     - Either use macOS native traffic lights (`Close`, `Minimize`, `Fullscreen` green button) neatly positioned in the top-left of the sidebar header / topbar (`titleBarStyle: "Overlay"` with `hiddenTitle: true` or custom positioned native traffic-light buttons) **or** keep matching layout padding so traffic lights never overlap the Sidebar toggle button, space selector, or top tabs! Never show duplicate Windows minimize/maximize/close icons alongside macOS traffic lights.
     - Double-clicking empty drag region in the topbar/sidebar header should trigger macOS zoom (`performZoom:`).
     - Support macOS native fullscreen (`NSWindow.toggleFullScreen`, Space/green button) **and** HTML5 fullscreen inside `WKWebView` cleanly (`TabEvent::Fullscreen`), keeping topbar auto-hide and window resizing fluid.
     - Persist window frame in `window.json` using multi-monitor `NSScreen.screens` bounds check so unplugged external monitors never leave Serica off-screen.

3. **Keyboard Shortcuts (`app.rs`, `webview_mac.rs`, `App.tsx`)**:
   - Intercept `Cmd` (`Meta`) equivalents in `WKWebView` (`performKeyEquivalent:` / `keyDown:`) + React `keydown` handler so shortcuts fire even when a child `WKWebView` has focus:
     - `Cmd+T` (New tab), `Cmd+W` (Close tab), `Cmd+Shift+T` (Reopen closed), `Cmd+K` / `Cmd+L` (Command Bar), `Cmd+R` / `Cmd+Shift+R` (Reload / Hard reload), `Cmd+S` / `Cmd+E` (Toggle sidebar — avoid clobbering webpage Save if needed or match Serica's binding cleanly), `Cmd+F` (Find), `Cmd+,` (Settings), `Cmd+H` / `Cmd+Y` (History), `Cmd+Shift+J` / `Cmd+J` (Downloads), `Cmd+Shift+N` (Private tab), `Cmd+1..9` (Switch tab), `Cmd+[` & `Cmd+]` + `Cmd+Left` & `Cmd+Right` (Back/Forward), `Cmd++` / `Cmd+-` / `Cmd+0` (Zoom), `Cmd+Option+I` & `F12` (Web Inspector), `Ctrl+Tab` / `Ctrl+Shift+Tab` (Next/Prev tab).
     - Update every shortcut hint badge in React UI (`TopTabStrip`, `Sidebar`, `CommandBar`, `Settings2`, `ProductTour`) to display macOS glyphs (`⌘`, `⌥`, `⇧`, `⌃`) on macOS!

4. **Typography, UI Copy & Trackpad Physics**:
   - Update `chrome.css`, `ERROR_PAGE_JS`, `BLOCK_PAGE`, `WEBSTORE_SCRIPT` font stacks so Windows `'Segoe UI Variable Text', 'Segoe UI'` falls back/maps cleanly on macOS to `-apple-system, BlinkMacSystemFont, "SF Pro Text", "SF Pro Display"` (alongside Serica's bundled `Inter` and Theme Studio fonts) and monospace `'Cascadia Mono'` -> `"SF Mono", ui-monospace, Menlo, monospace`.
   - Update Windows-specific UI copy cleanly: `"Match Windows"` -> `"Match macOS"`, `"Explorer"` -> `"Finder"`, `"Windows Downloads folder"` -> `"Downloads folder"`, `"on this PC"` -> `"on this Mac"`, `"Windows asks in Settings"` -> `"macOS asks in System Settings"`, `"Windows VPN"` -> `"System VPN"`.
   - Ensure smooth 120Hz ProMotion trackpad inertia scrolling, pinch-to-zoom, and two-finger space/history swipes work naturally without fighting AppKit.

---

## 6. Build, Packaging & Verification Plan (Must Complete Before Finishing)

1. **Create `./macos/build_mac.py`** (mirroring root `build.py`):
   - `--dev` (isolated or standard dev run), `--test` (builds/runs with `tauri.test.conf.json` -> `Serica Test` profile), `--fresh` (clean Cargo build), `--bundle` (produces signed/ad-hoc `.app` + `.dmg`), `--status`.
2. **Run Automated Verification**:
   - Inside `./macos/`:
     - `npx tsc --noEmit` (0 errors)
     - `npm run build` (0 Vite/TS errors)
     - `npm test` (`node tools/tests/run.mjs` — all frontend tests passing)
     - `cargo test` inside `./macos/src-tauri/` (all Rust unit/integration tests passing: shields, focus, focus_stats, bookmarks, history, vpn, store, settings, etc.)
     - `cargo tauri build` (or `python3 build_mac.py --bundle`) producing a launchable `Serica.app` (and `.dmg`).
3. **Interactive / Runtime Verification on MacBook**:
   - Launch the built app (using isolated `--test` profile so personal data isn't affected) and verify:
     - Main window renders at full Retina sharpness with zero clipped/overlapping traffic lights.
     - Navigation to real HTTPS websites (`https://example.com`, YouTube, GitHub) renders smoothly in `WKWebView`.
     - `Cmd+C`, `Cmd+V`, `Cmd+A` work inside both the Command Bar and loaded web pages.
     - Rounded-rect `CALayer` mask holes animate smoothly when opening Command Bar (`Cmd+K`), Right-click `PageContextMenu`, Extensions modal, Downloads panel, and Hover previews.
     - Both **Sidebar** and **Top Tabs + Right Pins Dock** layouts work, including two-finger Space swipe and topbar auto-hide.
     - **Shields** blocks ads/trackers and increments blocked count; **Focus Mode** blocks domains/SPAs and records **Focus Stats**.
     - **Privacy Tunnel (VPN)** DoH/Proxy toggles connect cleanly through `WKWebsiteDataStore.proxyConfigurations`.
     - **Downloads** save files, report progress, reveal in Finder, and drag-out works; **Drag-and-drop** onto window triggers `DropZones`.
     - **MCP HTTP server** responds on `http://127.0.0.1:7531/mcp`.
     - **Browser Import** detects installed macOS browsers + Safari.
   - Fix every compiler warning, clip/coordinate mismatch, deadlock, or runtime quirk you discover before reporting completion.
4. **Final Check**:
   - Run `git status` to confirm `./rust/` and existing Windows files were NOT modified or broken, and that the complete macOS port lives cleanly inside `./macos/`.
