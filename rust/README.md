# Neutrino Browser (Rust + Tauri v2)

The browser backend is Rust. TypeScript is used only for the UI (React).

## Architecture

```
src-tauri/src/            Rust backend
  app.rs                  Core: owns all services, throttled UI updates, debounced saves
  store.rs                Profiles, spaces, tabs, favorites, folders, archive, Zen sleep
  tabhost.rs              Native WebView2 child webview per tab, assistant drawer view
  webview_win.rs          WebView2 hooks: navigation, titles, favicons, audio, fullscreen,
                          shortcuts inside pages, find-in-page, capture, adblock
  shields.rs              Brave adblock-rust engine (network + cosmetic + scriptlets)
  vpn.rs                  Local tunnel proxy: DoH, Tor/SOCKS5/HTTP upstream, kill switch
  history.rs              Persistent history with search and suggestions
  settings.rs             Settings with validation (never resets on a bad key)
  mcp.rs                  MCP tools that let assistants drive the browser
  mcp_http.rs             Local MCP endpoint (127.0.0.1:7531/mcp, token auth, off by default)
  extensions.rs           Chrome Web Store extensions (CRX install, per-profile sync)
  downloads.rs            Download manager (WebView2 downloads, pause/resume, history)
  importer.rs             One-time read-only import of a profile from the old Electron build
  commands.rs             IPC commands (the `window.neutrino` API)
src/                      UI only
  adapter/tauriBridge.ts  Typed invoke/listen wrappers, no logic
  shared/                 Pure UI-side models (themes, sidebar layout, IPC types)
```

### How pages are shown
Every tab is a native WebView2 child webview placed over the UI's page rectangle,
which the UI reports with `set_viewport`. Native views always paint above HTML, so
while a modal, popover, context menu or hover card covers the page, the UI asks the
backend to *freeze* the viewport: the page is captured to an image, the image is
shown, and the native view is hidden until the overlay closes
(`src/hooks/useViewportOverlay.ts`).

Webviews are created lazily when a tab is first activated and are destroyed when a
tab sleeps, closes, or goes back to the new-tab page. Each profile has its own
WebView2 data directory, so cookies and storage are separate per profile.

### Ad blocking (Brave adblock-rust)
- Network: every request from a tab (`WebResourceRequested`, all resource types and
  frames) is checked by `adblock::Engine`, honouring `$redirect` resources,
  `$important` and exceptions. A blocked request gets a 403.
- Cosmetic: per-site hide selectors and uBlock/Brave scriptlets are registered for
  each navigation at document creation. Generic class/id hiding uses the classes
  and ids the page reports.
- Lists: EasyList, EasyPrivacy, and the uBlock filters, privacy, unbreak and
  quick-fixes lists, plus Brave's scriptlet resources. They are downloaded in the
  background, cached in `%APPDATA%\com.neutrino.browser\adblock`, and refreshed
  every 4 days. Startup uses the serialized engine. A built-in rule set covers the
  first run and offline use.

### Privacy tunnel (VPN)
Tab webviews always use a local proxy in `vpn.rs`, so modes switch live:
- Off and system VPN: direct connection.
- Encrypted DNS: DNS-over-HTTPS (RFC 8484).
- Tor: SOCKS5 on 127.0.0.1:9050 with remote DNS.
- Custom tunnel: SOCKS5 or HTTP upstream, with optional authentication.

HTTPS is tunnelled end-to-end with CONNECT, and the proxy never sees plaintext.
With the kill switch on, nothing falls back to a direct connection while the
tunnel is down. HTTPS (TLS) upstream proxies are not supported.

### Downloads
`downloads.rs` takes over every download a tab starts (WebView2 `DownloadStarting`,
marked handled, so Edge's own flyout never shows). Files go to the user's Downloads
folder. The top bar shows a Downloads button with a progress ring, and a new download
opens the panel (also Ctrl+J or `neutrino://downloads`). From the panel you can pause,
resume, cancel, retry, open a file, show it in its folder, and clear finished entries.
The list is kept in `downloads.json`. Downloads that were still running when the
browser closed are marked "Browser closed".

### Extensions (Chrome Web Store)
`extensions.rs` uses WebView2's browser-extension API (runtime 1.0.2420+).
- Install: on any `chromewebstore.google.com/detail/...` page, tabs get an
  **Add to Neutrino** button, because the store's own button needs Chrome. You
  can also paste a store link or extension ID into the Extensions panel. The CRX
  comes from Google's update service. It is unpacked to
  `%APPDATA%\com.neutrino.browser\extensions\<id>`, `_metadata` is removed, and
  the store public key is written to `manifest.json` so the extension keeps its
  store ID.
- `extensions.json` is the source of truth. Each tab profile is synced against
  it the first time one of its tabs opens in a session, and again after any
  change. Missing extensions are added, enabled state is applied, and removed
  extensions are uninstalled.
- Manage them in `neutrino://extensions`, the command bar (Manage Extensions),
  or Settings → Privacy. WebView2 has no toolbar, so an extension's popup and
  options pages open in a tab (`chrome-extension://<id>/...`).

## Build and run

Prerequisites: Rust (stable, MSVC toolchain on Windows), Node 20+, and the WebView2
runtime.

```powershell
npm install
npm run tauri dev                 # dev with Vite HMR
npm run tauri build -- --no-bundle  # release exe in src-tauri/target/release
cd src-tauri; cargo test          # backend unit tests
```

## Status

Working:
- Tabs: navigation, back/forward, reload, stop, titles, favicons, loading state,
  zoom, find in page, devtools (F12), `target=_blank` links opened as tabs, and
  real popups for OAuth.
- Sidebar: pinned, Today and favorites; folders; drag moves; move to another
  space; duplicate; close and reopen (Ctrl+Shift+T); tab cycling.
- Spaces and profiles, each profile with isolated storage.
- Zen sleep after 10 minutes idle, manual Zen, and archiving of old Today tabs.
- Live tab previews.
- Browser shortcuts while a page has focus: Ctrl+T/L/K/W/S/E/F/H/Tab/1–9,
  Ctrl+J, Ctrl+Shift+T, Ctrl+comma, F6, Alt+Home and Esc.
- History, Shields, the VPN modes above, the MCP tools, and the assistant drawer.
- Media mini player, docked above the page while a page is shown.
- HTML fullscreen and theme-color top-bar tint.

Not available yet:
- Password manager. The UI shows it as unavailable.
- Turning the WebRTC leak guard on or off applies to newly created tab webviews only.
