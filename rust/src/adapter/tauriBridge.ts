// Thin typed bridge from the UI to the Rust backend (src-tauri/src/commands.rs).
// Every call is a Tauri `invoke`; every `on*` subscribes to an event the
// backend emits to the main webview. No browser logic lives here.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { GeoPlace, GithubContributions, PageContextMenuRequest, PageDialogRequest, WeatherNow } from '../types';

type Unsubscribe = () => void;

export interface UpdateStatus {
  state: 'idle' | 'checking' | 'up-to-date' | 'downloading' | 'ready' | 'installing' | 'error';
  currentVersion: string;
  newVersion?: string | null;
  notes?: string | null;
  progress?: number | null;
  error?: string | null;
  lastChecked?: number | null;
}

/** Subscribes to a backend event; safe to unsubscribe before `listen` resolves. */
function on<T>(event: string, callback: (payload: T) => void): Unsubscribe {
  let unlisten: UnlistenFn | undefined;
  let cancelled = false;
  void listen<T>(event, (e) => callback(e.payload)).then((fn) => {
    if (cancelled) fn();
    else unlisten = fn;
  });
  return () => {
    cancelled = true;
    unlisten?.();
  };
}

const siteAction = (action: string) => invoke<void>('site_action', { action });

export const tauriBridge = {
  isElectron: false,
  isTauri: true,

  // Navigation
  navigate: (url: string) => invoke<void>('navigate', { url }),
  goBack: () => siteAction('back'),
  goForward: () => siteAction('forward'),
  reload: () => siteAction('reload'),
  hardReload: () => siteAction('hard-reload'),
  stopLoading: () => siteAction('stop'),
  siteAction,
  reopenClosedTab: () => invoke<any>('reopen_closed_tab'),
  cycleTab: (direction: 1 | -1) => invoke<any>('cycle_tab', { direction }),
  openExternalUrl: (url: string) => invoke<boolean>('open_external_url', { url }),
  answerPageDialog: (id: number, accept: boolean, text?: string) =>
    invoke<void>('answer_page_dialog', { id, accept, text: text ?? null }),
  openDevtools: () => invoke<void>('open_devtools'),

  // Native viewport (tab webviews are positioned over this rectangle)
  setViewport: (x: number, y: number, width: number, height: number) =>
    invoke<void>('set_viewport', { x, y, width, height }),
  captureViewport: () => invoke<string | null>('viewport_capture'),
  setViewportHole: (
    id: string,
    rect: [number, number, number, number],
    opts?: { from?: [number, number, number, number]; ms?: number; ease?: [number, number, number, number]; remove?: boolean; lead?: number; radius?: number },
  ) =>
    invoke<void>('set_viewport_hole', {
      id,
      rect,
      from: opts?.from ?? null,
      ms: opts?.ms ?? null,
      ease: opts?.ease ?? null,
      remove: opts?.remove ?? null,
      lead: opts?.lead ?? null,
      radius: opts?.radius ?? null,
    }),
  uiCursor: () => invoke<[number, number, number] | null>('ui_cursor'),
  freezeViewport: (capture = true) => invoke<string | null>('viewport_freeze', { capture }),
  thawViewport: () => invoke<void>('viewport_thaw'),
  findInPage: (query: string | null, options?: { forward?: boolean; findNext?: boolean }) =>
    invoke<void>('find_in_page', { query, forward: options?.forward ?? true, findNext: options?.findNext ?? false }),
  getMediaState: (tabId: string) => invoke<any>('get_media_state', { tabId }),
  mediaControl: (action: string, params?: { tabId?: string; value?: number }) =>
    params?.tabId ? invoke<void>('media_control', { tabId: params.tabId, action, value: params.value ?? null }) : Promise.resolve(),

  // State, tabs & spaces
  getState: () => invoke<any>('get_browser_state'),
  createTab: (url?: string, spaceId?: string) => invoke<any>('create_tab', { url: url ?? null, spaceId: spaceId ?? null }),
  updateTab: (tabId: string, partial: any) => invoke<void>('update_tab', { tabId, partial }),
  closeTab: (tabId: string) => invoke<void>('close_tab', { tabId }),
  selectTab: (tabId: string) => invoke<void>('select_tab', { tabId }),
  togglePinTab: (tabId: string) => invoke<void>('toggle_pin_tab', { tabId }),
  toggleSleepTab: (tabId: string) => invoke<void>('toggle_sleep_tab', { tabId }),
  reorderTabs: (draggedTabId: string, targetTabId: string) => invoke<void>('reorder_tabs', { draggedTabId, targetTabId }),
  selectSpace: (spaceId: string) => invoke<void>('select_space', { spaceId }),
  createSpace: (name: string, color?: string, profileId?: string) =>
    invoke<any>('create_space', { name, color: color ?? null, profileId: profileId ?? null }),
  deleteSpace: (spaceId: string) => invoke<any>('delete_space', { spaceId }),
  updateSpace: (spaceId: string, patch: any) => invoke<any>('update_space', { spaceId, patch }),
  reorderSpace: (spaceId: string, toIndex: number) => invoke<any>('reorder_space', { spaceId, toIndex }),

  // Profiles
  listProfiles: () => invoke<any>('list_profiles'),
  createProfile: (name?: string) => invoke<any>('create_profile', { name: name ?? null }),
  renameProfile: (profileId: string, name: string) => invoke<any>('rename_profile', { profileId, name }),
  deleteProfile: (profileId: string) => invoke<any>('delete_profile', { profileId }),

  // Sidebar layout
  moveTab: (op: any) => invoke<any>('move_tab', { op }),
  pinTab: (tabId: string) => invoke<any>('pin_tab', { tabId }),
  unpinTab: (tabId: string) => invoke<any>('unpin_tab', { tabId }),
  addPinnedSites: (sites: { url: string; title: string }[], spaceId?: string) =>
    invoke<number>('add_pinned_sites', { sites, spaceId: spaceId ?? null }),
  addFavorite: (tabId: string) => invoke<any>('add_favorite', { tabId }),
  removeFavorite: (favoriteId: string) => invoke<any>('remove_favorite', { favoriteId }),
  duplicateTab: (id: string, spaceId?: string) => invoke<any>('duplicate_tab', { id, spaceId: spaceId ?? null }),
  createFolder: (spaceId: string, name: string) => invoke<any>('create_folder', { spaceId, name }),
  updateFolder: (folderId: string, patch: any) => invoke<any>('update_folder', { folderId, patch }),
  deleteFolder: (folderId: string) => invoke<any>('delete_folder', { folderId }),
  moveTabToFolder: (tabId: string, folderId: string | null) => invoke<any>('move_tab_to_folder', { tabId, folderId }),
  listArchive: () => invoke<any[]>('list_archive'),
  restoreArchivedTab: (archivedId: string) => invoke<any>('restore_archived_tab', { archivedId }),
  updateSidebarState: (patch: any) => invoke<any>('update_settings', { patch }),
  setZoomFactor: (factor: number) => invoke<void>('set_zoom_factor', { factor, tabId: null }),

  // Shields (Brave adblock engine)
  toggleShields: () => invoke<boolean>('toggle_shields'),
  toggleSiteWhitelist: (domainOrUrl: string) => invoke<any>('toggle_site_whitelist', { domainOrUrl }),
  resetShieldStats: () => invoke<any>('reset_shield_stats'),
  getShieldStats: () => invoke<any>('get_shield_stats'),
  getShieldsInfo: () => invoke<any>('get_shields_info'),
  updateFilterLists: () => invoke<void>('update_filter_lists'),
  zenOptimize: () => invoke<any>('zen_optimize'),

  // Settings & theme
  getSettings: () => invoke<any>('get_settings'),
  updateSettings: (patch: any) => invoke<any>('update_settings', { patch }),
  getPerformanceStatus: () => invoke<any>('get_performance_status'),
  setThemeMode: (mode: 'dark' | 'light' | 'system') => invoke<any>('set_theme_mode', { mode }),
  windowControl: (action: 'minimize' | 'maximize' | 'close') => invoke<void>('window_control', { action }),

  // Previews
  captureTabPreview: (tabId: string) => invoke<string | null>('capture_tab_preview', { tabId }),
  captureTabPreviewById: (tabId: string) => invoke<string | null>('capture_tab_preview', { tabId }),
  getTabPreview: (tabId: string) => invoke<string | null>('get_tab_preview', { tabId }),

  // History
  getHistory: (options?: any) => invoke<any[]>('get_history', { options: options ?? null }),
  historySuggest: (text: string, limit?: number) => invoke<any[]>('history_suggest', { text, limit: limit ?? null }),
  deleteHistoryEntry: (target: any) => invoke<boolean>('delete_history_entry', { target }),
  clearHistory: (range?: string) => invoke<number>('clear_history', { range: range ?? null }),

  // VPN / privacy tunnel
  getVpnStatus: () => invoke<any>('get_vpn_status'),
  connectVpn: (patch?: any) => invoke<any>('connect_vpn', { patch: patch ?? null }),
  disconnectVpn: () => invoke<any>('disconnect_vpn'),
  updateVpnConfig: (patch: any) => invoke<any>('update_vpn_config', { patch }),
  checkVpnIp: () => invoke<any>('check_vpn_ip'),

  // MCP & assistant drawer
  callMcp: (method: string, params?: any) => invoke<any>('mcp_call', { method, params: params ?? null }),
  getMcpLogs: () => invoke<any[]>('get_mcp_logs'),
  mcpServerStatus: () => invoke<any>('mcp_server_status'),
  mcpRegenerateToken: () => invoke<any>('mcp_regenerate_token'),
  assistantOpen: (url: string, x: number, y: number, width: number, height: number) =>
    invoke<void>('assistant_open', { url, x, y, width, height }),
  assistantBounds: (x: number, y: number, width: number, height: number) =>
    invoke<void>('assistant_bounds', { x, y, width, height }),
  assistantClose: () => invoke<void>('assistant_close'),
  assistantInsertText: (text: string) => invoke<boolean>('assistant_insert_text', { text }),

  // Downloads
  listDownloads: () => invoke<any[]>('list_downloads'),
  downloadAction: (id: string, action: string) => invoke<boolean>('download_action', { id, action }),
  clearDownloads: () => invoke<any[]>('clear_downloads'),
  openDownloadsFolder: () => invoke<void>('open_downloads_folder'),
  openFile: () => invoke<void>('open_file'),
  readClipboardText: () => invoke<string | null>('read_clipboard_text'),
  cookieSites: () => invoke<{ ok: boolean; error?: string; sites?: { site: string; count: number }[] }>('cookie_sites'),
  deleteCookies: (site?: string) => invoke<{ ok: boolean; error?: string; deleted?: number }>('delete_cookies', { site: site ?? null }),

  // Extensions (Chrome Web Store)
  listExtensions: () => invoke<any[]>('list_extensions'),
  installExtension: (input: string) => invoke<any>('install_extension', { input }),
  setExtensionEnabled: (id: string, enabled: boolean) => invoke<boolean>('set_extension_enabled', { id, enabled }),
  removeExtension: (id: string) => invoke<boolean>('remove_extension', { id }),
  openExtensionPage: (id: string, page: 'popup' | 'options') => invoke<boolean>('open_extension_page', { id, page }),

  // Passwords (not available in this build; the backend says so)
  getPasswordStatus: () => invoke<any>('passwords_status'),
  listPasswords: () => invoke<any[]>('passwords_list'),

  // Updates
  getUpdateStatus: () => invoke<UpdateStatus>('get_update_status'),
  checkForUpdate: () => invoke<UpdateStatus>('check_for_update'),
  installUpdate: () => invoke<void>('install_update'),

  // Events from the backend
  onStateUpdate: (cb: (state: any) => void) => on('serica:state-update', cb),
  onTrackerBlocked: (cb: (item: any) => void) =>
    on('serica:trackers-blocked', (items: any) => {
      if (Array.isArray(items)) for (const it of items) cb(it);
    }),
  onSettingsUpdated: (cb: (settings: any) => void) => on('serica:settings-updated', cb),
  onPerformanceStatusUpdated: (cb: (status: any) => void) => on('serica:performance-updated', cb),
  onHistoryUpdated: (cb: (entries: any[]) => void) => on('serica:history-updated', cb),
  onVpnStatusUpdated: (cb: (status: any) => void) => on('serica:vpn-status-updated', cb),
  onArchiveUpdated: (cb: (entries: any[]) => void) => on('serica:archive-updated', cb),
  onDownloadStarted: (cb: (item: any) => void) => on('serica:download-started', cb),
  onDownloadUpdated: (cb: (item: any) => void) => on('serica:download-updated', cb),
  onDownloadsChanged: (cb: (list: any[]) => void) => on('serica:downloads-changed', cb),
  onOpenDownloads: (cb: () => void) => on('serica:open-downloads', () => cb()),
  onExtensionsUpdated: (cb: (list: any[]) => void) => on('serica:extensions-updated', cb),
  onExtensionInstalled: (cb: (ext: any) => void) => on('serica:extension-installed', cb),
  onOpenSettings: (cb: () => void) => on('serica:open-settings', () => cb()),
  onOpenHistory: (cb: () => void) => on('serica:open-history', () => cb()),
  onOpenCommandBar: (cb: (opts?: { newTab?: boolean }) => void) => on('serica:open-command-bar', cb),
  onToggleTabs: (cb: () => void) => on('serica:toggle-tabs', () => cb()),
  onEscapePressed: (cb: () => void) => on('serica:escape-pressed', () => cb()),
  onSiteAction: (cb: (payload: { action: string; source?: string }) => void) => on('serica:site-action', cb),
  onSpaceSwitched: (cb: (payload: { spaceId: string; spaceName: string }) => void) => on('serica:space-switched', cb),
  onZoomUpdated: (cb: (data: { tabId?: string; zoomFactor: number }) => void) => on('serica:zoom-updated', cb),
  onFindResult: (cb: (r: { tabId: string; activeMatch: number; totalMatches: number }) => void) => on('serica:find-result', cb),
  onPageColor: (cb: (r: { tabId: string; themeColor: string | null }) => void) => on('serica:page-color', cb),
  widgetGithub: (user: string) => invoke<GithubContributions>('widget_github', { user }),
  widgetGeocode: (query: string) => invoke<GeoPlace>('widget_geocode', { query }),
  widgetWeather: (lat: number, lon: number, fahrenheit: boolean) =>
    invoke<WeatherNow>('widget_weather', { lat, lon, fahrenheit }),
  getPdfDarkTab: (tabId: string) => invoke<{ available: boolean; dark: boolean }>('get_pdf_dark_tab', { tabId }),
  setPdfDarkTab: (tabId: string, dark: boolean) => invoke<void>('set_pdf_dark_tab', { tabId, dark }),
  onPageContextMenu: (cb: (m: PageContextMenuRequest) => void) => on('serica:page-context-menu', cb),
  pageMenuSelect: (id: number, pick: { command?: number; custom?: number; refocus?: boolean }) =>
    invoke<void>('page_menu_select', {
      id,
      command: pick.command ?? null,
      custom: pick.custom ?? null,
      refocus: pick.refocus ?? false,
    }),
  onUpdateStatus: (cb: (status: UpdateStatus) => void) => on('serica:update-status', cb),
  onHtmlFullscreen: (cb: (isFullscreen: boolean) => void) => on('serica:html-fullscreen', cb),
  onPageDialog: (cb: (d: PageDialogRequest) => void) => on('serica:page-dialog', cb),

  // Site permissions (camera, microphone, location...) asked in Serica's prompt
  onPermissionRequest: (cb: (r: PermissionRequest) => void) => on('serica:permission-request', cb),
  answerPermission: (id: number, decision: 'allow' | 'allow-once' | 'block' | 'deny') =>
    invoke<void>('answer_permission', { id, decision }),

  // Drag and drop onto Serica's own interface
  onDrag: (cb: (e: DragEventPayload) => void) => on('serica:drag', cb),
  openDroppedPaths: (paths: string[]) => invoke<number>('open_dropped_paths', { paths }),

  // Focus Mode
  focusGet: () => invoke<FocusView>('focus_get'),
  focusSaveBlock: (block: FocusBlock) => invoke<FocusView>('focus_save_block', { block }),
  focusDeleteBlock: (id: string) => invoke<FocusView>('focus_delete_block', { id }),
  focusStart: (id: string, minutes: number | null, lock: boolean) => invoke<FocusView>('focus_start', { id, minutes, lock }),
  focusStop: (id: string, text?: string) => invoke<FocusView>('focus_stop', { id, text: text ?? null }),
  focusChallenge: (id: string) => invoke<string>('focus_challenge', { id }),
  focusCheck: (url: string) => invoke<boolean>('focus_check', { url }),
  focusStats: (range: FocusRange) => invoke<FocusStats>('focus_stats', { range }),
  /** Local models, reached from the backend (no origin for Ollama to refuse). */
  localAiScan: () => invoke<LocalAiScan>('local_ai_scan'),
  localAiChat: (id: string, runtime: 'ollama' | 'lmstudio', model: string, messages: { role: 'user' | 'assistant'; content: string }[]) =>
    invoke<void>('local_ai_chat', { id, runtime, model, messages }),
  localAiStop: (id: string) => invoke<void>('local_ai_stop', { id }),
  onLocalAi: (cb: (c: LocalAiChunk) => void) => on('serica:local-ai', cb),
  focusStatsTracking: (on: boolean) => invoke<void>('focus_stats_tracking', { on }),
  focusStatsClear: () => invoke<void>('focus_stats_clear'),
  /** Saves every day as CSV where the user picks; the path, or null if cancelled. */
  focusStatsExport: () => invoke<string | null>('focus_stats_export'),
  onFocusUpdated: (cb: (v: FocusView) => void) => on('serica:focus-updated', cb),

  // New-tab background picture, default browser, importing
  pickBackgroundImage: () => invoke<{ file: string; url: string } | null>('pick_background_image'),
  defaultBrowserStatus: () => invoke<{ isDefault: boolean; supported: boolean }>('default_browser_status'),
  makeDefaultBrowser: () => invoke<{ isDefault: boolean; supported: boolean }>('make_default_browser'),
  importSources: () => invoke<ImportSource[]>('import_sources'),
  importFromBrowser: (request: ImportRequest) => invoke<ImportReport>('import_from_browser', { request }),
  importPasswordsFile: () => invoke<ImportReport | null>('import_passwords_file'),
};

export interface FocusSlot {
  /** Bit mask, bit 0 = Sunday. */
  days: number;
  /** Minutes after midnight. */
  start: number;
  end: number;
}

export interface FocusBlock {
  id: string;
  name: string;
  sites: string[];
  allow: string[];
  categories: string[];
  schedule: FocusSlot[];
  scheduleOn: boolean;
  scheduleLocked: boolean;
  unlock: 'strict' | 'randomText';
}

export interface FocusActive {
  blockId: string;
  source: 'session' | 'schedule';
  remainingMs: number | null;
  totalMs: number | null;
  locked: boolean;
}

export interface FocusView {
  blocks: FocusBlock[];
  active: FocusActive[];
  categories: { id: string; label: string; sites: string[] }[];
  clockVerified: boolean;
}

export interface LocalAiScan {
  ollama: boolean;
  lmstudio: boolean;
  models: { name: string; runtime: 'ollama' | 'lmstudio' }[];
}

/** A piece of a local model's reply; `done` ends it (with `error` if it failed). */
export interface LocalAiChunk {
  id: string;
  delta?: string;
  done: boolean;
  error?: string;
}

export type FocusRange = 'day' | 'week' | 'month' | 'year';

export interface FocusTotals {
  focusMs: number;
  browseMs: number;
  distractMs: number;
  blocked: number;
  started: number;
  completed: number;
  stopped: number;
  longestMs: number;
}

export interface FocusStats {
  range: FocusRange;
  tracking: boolean;
  /** Hours of today (`"14"`), days (`"2026-11-26"`) or months (`"2026-11"`). */
  buckets: { key: string; focusMs: number; browseMs: number; distractMs: number; blocked: number }[];
  totals: FocusTotals;
  previous: FocusTotals;
  days: number;
  streak: number;
  bestStreak: number;
  streakGoalMs: number;
  sites: { host: string; ms: number; listed: boolean }[];
  blockedSites: { host: string; count: number }[];
  blockedHours: number[];
  focusHours: number[];
  lists: { id: string; name: string; ms: number }[];
  hasData: boolean;
}

export interface ImportSource {
  id: string;
  name: string;
  engine: 'chromium' | 'gecko';
  profiles: { id: string; name: string }[];
}

export interface ImportRequest {
  source: string;
  profile: string;
  bookmarks: boolean;
  history: boolean;
  passwords: boolean;
  asNewProfile: boolean;
}

export interface ImportReport {
  bookmarks: number;
  pinned: number;
  history: number;
  passwords: number;
  passwordsLocked: number;
  primaryPassword: boolean;
  profileId: string | null;
  profileName: string | null;
  errors: string[];
}

export interface PermissionRequest {
  id: number;
  tabId: string;
  /** `https://host[:port]`: the unit an answer is remembered for. */
  origin: string;
  host: string;
  kind: string;
  userInitiated: boolean;
}

export interface DragEventPayload {
  phase: 'enter' | 'over' | 'leave' | 'drop';
  /** Physical client px of the main window, and its client width (to get CSS px). */
  x: number;
  y: number;
  clientWidth: number;
  data: { kind: 'files' | 'url' | 'text'; paths: string[]; url?: string | null; text?: string | null } | null;
}

export type TauriBridge = typeof tauriBridge;

if (typeof window !== 'undefined') {
  (window as any).serica = tauriBridge;
  (window as any).neutrino = tauriBridge;
}
