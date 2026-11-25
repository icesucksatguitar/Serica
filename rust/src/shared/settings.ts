import { normalizePinnedTools, type ToolKey } from './tools';
// Pure browser-settings rules shared by the main process and the renderer.
// No Electron import: the renderer imports this module too (Vite resolves it), and tests run it directly.

import {
  type CustomSpaceThemePreset,
  type SpaceTheme,
  type SpaceThemesMap,
  getDefaultSpaceTheme,
  normalizeCustomThemePresets,
  normalizeSpaceThemesMap,
  validateAndClampSpaceTheme,
} from './spaceTheme';

export type { CustomSpaceThemePreset, SpaceTheme, SpaceThemesMap };

export type SearchEngineId = 'google' | 'duckduckgo' | 'bing' | 'brave';

export const SEARCH_ENGINES: Readonly<Record<SearchEngineId, { readonly label: string; readonly prefix: string }>> =
  Object.freeze({
    google: Object.freeze({ label: 'Google', prefix: 'https://www.google.com/search?q=' }),
    duckduckgo: Object.freeze({ label: 'DuckDuckGo', prefix: 'https://duckduckgo.com/?q=' }),
    bing: Object.freeze({ label: 'Bing', prefix: 'https://www.bing.com/search?q=' }),
    brave: Object.freeze({ label: 'Brave Search', prefix: 'https://search.brave.com/search?q=' }),
  });

export type ReducedEffectsMode = 'auto' | 'on' | 'off';

export interface PerformanceStatus {
  hardwareAcceleration: boolean;
  gpuDisabledFlag: boolean;
  softwareRasterized: boolean;
  reducedEffectsMode: ReducedEffectsMode;
  reducedEffectsActive: boolean;
  gpuFeatureStatus: Record<string, string>;
}

export function normalizeReducedEffectsMode(value: unknown): ReducedEffectsMode {
  return value === 'on' || value === 'off' || value === 'auto' ? value : 'auto';
}

function isStatusSoftwareOrDisabled(val: unknown): boolean {
  if (typeof val !== 'string') return false;
  const lower = val.toLowerCase();
  return (
    lower.includes('software') ||
    lower.includes('disabled') ||
    lower === 'unavailable_off' ||
    lower === 'unavailable_software'
  );
}

export function isSoftwareGpuStatus(gpuStatus?: Record<string, unknown> | null): boolean {
  if (!gpuStatus || typeof gpuStatus !== 'object') return false;
  const direct3d = gpuStatus['direct_rendering_display_compositor'];
  const raster = gpuStatus['rasterization'];
  const comp = gpuStatus['gpu_compositing'];
  if (isStatusSoftwareOrDisabled(direct3d) || isStatusSoftwareOrDisabled(raster) || isStatusSoftwareOrDisabled(comp)) {
    return true;
  }
  return false;
}

export function resolveReducedEffectsActive(
  mode: unknown,
  gpuDisabledFlag: boolean,
  gpuStatus?: Record<string, unknown> | null
): boolean {
  const normalized = normalizeReducedEffectsMode(mode);
  if (normalized === 'on') return true;
  if (normalized === 'off') return false;
  return Boolean(gpuDisabledFlag) || isSoftwareGpuStatus(gpuStatus);
}

export type TabLayout = 'sidebar' | 'top';

/** Look of the top bar's controls: rounded bordered pills, or flat borderless buttons. */
export type TopBarStyle = 'pill' | 'flat';

/** Text size: scales the interface and is the default zoom for new pages. */
export const TEXT_SCALE_OPTIONS: readonly { value: number; label: string }[] = Object.freeze([
  { value: 0.9, label: 'Small' },
  { value: 1, label: 'Default' },
  { value: 1.1, label: 'Large' },
  { value: 1.25, label: 'Larger' },
]);

function normalizeTextScale(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return 1;
  return TEXT_SCALE_OPTIONS.reduce((best, o) => (Math.abs(o.value - value) < Math.abs(best - value) ? o.value : best), 1);
}

export interface BrowserSettings {
  adblockEnabled: boolean;
  zenSleepEnabled: boolean;
  /** Minutes a background tab stays unused before Zen sleeps it. */
  tabSleepMinutes: TabSleepMinutes;
  /** Pop a playing video into picture-in-picture when switching away from its tab. */
  autoPictureInPicture: boolean;
  /** Show PDFs with black pages and white text. */
  pdfDarkMode: boolean;
  /** Vertical sidebar with hover previews, or a horizontal strip of tabs under the top bar. */
  tabLayout: TabLayout;
  topBarStyle: TopBarStyle;
  /** On web pages, hide the top bar until the pointer reaches the top edge. */
  topBarAutoHide: boolean;
  /** Block cookies from sites other than the one in the address bar. */
  blockThirdPartyCookies: boolean;
  /** Delete cookies when Serica closes, except for `cookieKeepSites`. */
  clearCookiesOnExit: boolean;
  /** Sites (with their subdomains) whose cookies are always kept. */
  cookieKeepSites: string[];
  /** Tools shown in the top bar's menu besides light/dark and Settings. */
  pinnedTools: ToolKey[];
  searchEngine: SearchEngineId;
  reducedEffectsMode?: ReducedEffectsMode;
  spaceThemes?: SpaceThemesMap;
  customThemePresets?: CustomSpaceThemePreset[];
  shieldWhitelist?: string[];
  archiveIntervalHours: ArchiveIntervalHours;
  rightPinsDockVisible: boolean;
  sidebarExpanded: boolean;
  sidebarAutoHide: boolean;
  sidebarWidth?: number;
  textScale: number;
  /** Widgets on the new-tab page. */
  homeWidgets: HomeWidgets;
  /** Separate sizes for the sidebar, dock and new-tab parts. */
  uiScale: UiScale;
  [unknownKey: string]: unknown; // kept as saved, e.g. the legacy `theme` key
}

/** Widget kinds: built-in ones, and app widgets (`app:<id>`, see home/apps.ts). */
export type WidgetKind = 'github' | 'weather' | 'notes' | 'calendar' | 'topsites' | `app:${string}`;
/** Grid sizes: s 2x2, m 4x2, l 4x4, w 8x2 cells. */
export type WidgetSize = 's' | 'm' | 'l' | 'w';

export interface WidgetItem {
  id: string;
  kind: WidgetKind;
  size: WidgetSize;
}

/** The new-tab page: what shows, in which order and size, and widget settings. */
export interface HomeWidgets {
  /** Greeting, date and clock. */
  clock: boolean;
  /** The search field under the clock. */
  search: boolean;
  /** Placed widgets, in grid order. */
  items: WidgetItem[];
  github: { user: string };
  weather: { place: string; lat: number | null; lon: number | null; fahrenheit: boolean };
  /** Text of each notes widget, by widget id. */
  notes: Record<string, string>;
  /** The widget guide was shown (it can be replayed from the edit bar). */
  guideSeen: boolean;
}

export const DEFAULT_HOME_WIDGETS: Readonly<HomeWidgets> = Object.freeze({
  clock: true,
  search: true,
  items: [],
  github: { user: '' },
  weather: { place: '', lat: null, lon: null, fahrenheit: false },
  notes: {},
  guideSeen: false,
});

const WIDGET_SIZES: readonly WidgetSize[] = ['s', 'm', 'l', 'w'];

function normalizeHomeWidgets(raw: unknown): HomeWidgets {
  const o = raw !== null && typeof raw === 'object' ? (raw as Record<string, any>) : {};
  const g = o.github && typeof o.github === 'object' ? o.github : {};
  const w = o.weather && typeof o.weather === 'object' ? o.weather : {};
  const num = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : null);
  let items: WidgetItem[] = [];
  const notes: Record<string, string> = {};
  if (Array.isArray(o.items)) {
    const seen = new Set<string>();
    for (const it of o.items) {
      if (!it || typeof it.id !== 'string' || typeof it.kind !== 'string' || seen.has(it.id)) continue;
      seen.add(it.id);
      items.push({ id: it.id, kind: it.kind as WidgetKind, size: WIDGET_SIZES.includes(it.size) ? it.size : 'm' });
    }
    if (o.notes && typeof o.notes === 'object' && !('text' in o.notes)) {
      for (const [k, v] of Object.entries(o.notes)) if (typeof v === 'string') notes[k] = v.slice(0, 4000);
    }
  } else {
    // 0.4.6 kept one flag per widget; turn the enabled ones into placed widgets.
    if (w.enabled === true) items.push({ id: 'weather-1', kind: 'weather', size: 'm' });
    if (o.notes?.enabled === true) {
      items.push({ id: 'notes-1', kind: 'notes', size: 'm' });
      if (typeof o.notes.text === 'string') notes['notes-1'] = o.notes.text.slice(0, 4000);
    }
    if (g.enabled === true) items.push({ id: 'github-1', kind: 'github', size: 'w' });
  }
  items = items.slice(0, 24);
  return {
    clock: typeof o.clock === 'boolean' ? o.clock : true,
    search: typeof o.search === 'boolean' ? o.search : true,
    items,
    github: { user: typeof g.user === 'string' ? g.user.slice(0, 39) : '' },
    weather: {
      place: typeof w.place === 'string' ? w.place.slice(0, 80) : '',
      lat: num(w.lat),
      lon: num(w.lon),
      fahrenheit: w.fahrenheit === true,
    },
    notes,
    guideSeen: o.guideSeen === true,
  };
}

/** Sizes of separate parts of the interface, relative to their default (1). */
export interface UiScale {
  sidebar: number;
  dock: number;
  /** Greeting and clock on the new tab. */
  hero: number;
  search: number;
  widgets: number;
}

export const DEFAULT_UI_SCALE: Readonly<UiScale> = Object.freeze({ sidebar: 1, dock: 1, hero: 1, search: 1, widgets: 1 });
export const UI_SCALE_MIN = 0.8;
export const UI_SCALE_MAX = 1.4;

function normalizeUiScale(raw: unknown): UiScale {
  const o = raw !== null && typeof raw === 'object' ? (raw as Record<string, unknown>) : {};
  const v = (x: unknown) =>
    typeof x === 'number' && Number.isFinite(x) ? Math.round(Math.min(UI_SCALE_MAX, Math.max(UI_SCALE_MIN, x)) * 100) / 100 : 1;
  return { sidebar: v(o.sidebar), dock: v(o.dock), hero: v(o.hero), search: v(o.search), widgets: v(o.widgets) };
}

/** How long an unused Today tab lives before it is archived. */
export type ArchiveIntervalHours = 12 | 24 | 168 | 720 | 'never';

export const ARCHIVE_INTERVAL_OPTIONS: readonly ArchiveIntervalHours[] = Object.freeze([12, 24, 168, 720, 'never']);

/** How long an unused background tab stays awake (minutes). Mirrors `TAB_SLEEP_MINUTES` in settings.rs. */
export type TabSleepMinutes = 15 | 30 | 60 | 120 | 240 | 720;

export const TAB_SLEEP_OPTIONS: readonly TabSleepMinutes[] = Object.freeze([15, 30, 60, 120, 240, 720]);

export const DEFAULT_SETTINGS: Readonly<BrowserSettings> = Object.freeze({
  adblockEnabled: true,
  zenSleepEnabled: true,
  tabSleepMinutes: 60 as TabSleepMinutes,
  autoPictureInPicture: true,
  pdfDarkMode: true,
  tabLayout: 'sidebar' as TabLayout,
  topBarStyle: 'flat' as TopBarStyle,
  topBarAutoHide: false,
  blockThirdPartyCookies: false,
  clearCookiesOnExit: false,
  cookieKeepSites: [] as string[],
  pinnedTools: [] as ToolKey[],
  searchEngine: 'google' as SearchEngineId,
  reducedEffectsMode: 'auto' as ReducedEffectsMode,
  archiveIntervalHours: 12 as ArchiveIntervalHours,
  rightPinsDockVisible: true,
  sidebarExpanded: true,
  sidebarAutoHide: false,
  sidebarWidth: 240,
  textScale: 1,
  homeWidgets: DEFAULT_HOME_WIDGETS,
  uiScale: DEFAULT_UI_SCALE,
});

function isArchiveIntervalHours(value: unknown): value is ArchiveIntervalHours {
  return (ARCHIVE_INTERVAL_OPTIONS as readonly unknown[]).includes(value);
}

function boolOr(value: unknown, fallback: boolean): boolean {
  return typeof value === 'boolean' ? value : fallback;
}

function hasOwn(obj: object, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(obj, key);
}

function isSearchEngineId(value: unknown): value is SearchEngineId {
  return typeof value === 'string' && hasOwn(SEARCH_ENGINES, value);
}

function encodeQuery(query: string): string {
  try {
    return encodeURIComponent(query);
  } catch {
    // Lone surrogates cannot be percent-encoded; replace them so the URL is still valid.
    return encodeURIComponent(query.replace(/[\uD800-\uDFFF]/g, '\uFFFD'));
  }
}

export function buildSearchUrl(engine: SearchEngineId, query: string): string {
  const id: SearchEngineId = isSearchEngineId(engine) ? engine : DEFAULT_SETTINGS.searchEngine;
  return SEARCH_ENGINES[id].prefix + encodeQuery(query);
}

function normalizeSearchEngine(value: unknown): SearchEngineId {
  if (isSearchEngineId(value)) return value;
  if (typeof value === 'string') {
    // Legacy saved value: the full URL prefix (the old default was the Google prefix).
    const match = (Object.keys(SEARCH_ENGINES) as SearchEngineId[]).find((id) => SEARCH_ENGINES[id].prefix === value);
    if (match) return match;
  }
  return DEFAULT_SETTINGS.searchEngine;
}

const DEFAULT_SPACE_IDS = ['space-study', 'space-work', 'space-leisure'] as const;

function buildSpaceThemes(rawMap: unknown, source: Record<string, unknown>): SpaceThemesMap {
  const hasExplicitLegacyOverride =
    source.theme === 'light' ||
    (source.customTheme !== null && typeof source.customTheme === 'object' && !Array.isArray(source.customTheme));

  const legacyContext = hasExplicitLegacyOverride ? source : undefined;
  const normalized = normalizeSpaceThemesMap(rawMap, legacyContext);

  if (Object.keys(normalized).length === 0) {
    for (const spaceId of DEFAULT_SPACE_IDS) {
      normalized[spaceId] = getDefaultSpaceTheme(spaceId, legacyContext);
    }
  }
  return normalized;
}

export function resolveSpaceTheme(settings: BrowserSettings | Record<string, unknown>, spaceId: string): SpaceTheme {
  const src = settings as Record<string, unknown>;
  const hasExplicitLegacyOverride =
    src.theme === 'light' ||
    (src.customTheme !== null && typeof src.customTheme === 'object' && !Array.isArray(src.customTheme));
  const fallback = getDefaultSpaceTheme(spaceId, hasExplicitLegacyOverride ? src : undefined);
  const map = src.spaceThemes as Record<string, unknown> | undefined;
  if (map && typeof map === 'object' && !Array.isArray(map) && hasOwn(map, spaceId)) {
    return validateAndClampSpaceTheme(map[spaceId], fallback);
  }
  return fallback;
}

function normalizeShieldDomain(value: unknown): string {
  if (typeof value !== 'string') return '';
  const trimmed = value.trim().toLowerCase();
  if (!trimmed) return '';
  try {
    const parsed = trimmed.includes('://') ? new URL(trimmed) : new URL(`https://${trimmed}`);
    return parsed.hostname.replace(/\.$/, '');
  } catch {
    return trimmed.replace(/^\.+|\.+$/g, '');
  }
}

function normalizeShieldWhitelist(raw: unknown): string[] {
  if (!Array.isArray(raw)) return [];
  const seen = new Set<string>();
  for (const item of raw) {
    const host = normalizeShieldDomain(item);
    if (host) seen.add(host);
  }
  return Array.from(seen).sort();
}

/** Fills missing or invalid fields with defaults, keeps unknown keys. Idempotent. */
export function normalizeSettings(raw: unknown): BrowserSettings {
  const source: Record<string, unknown> =
    raw !== null && typeof raw === 'object' && !Array.isArray(raw) ? (raw as Record<string, unknown>) : {};
  return {
    ...source,
    adblockEnabled: typeof source.adblockEnabled === 'boolean' ? source.adblockEnabled : DEFAULT_SETTINGS.adblockEnabled,
    zenSleepEnabled:
      typeof source.zenSleepEnabled === 'boolean' ? source.zenSleepEnabled : DEFAULT_SETTINGS.zenSleepEnabled,
    tabSleepMinutes: (TAB_SLEEP_OPTIONS as readonly unknown[]).includes(source.tabSleepMinutes)
      ? (source.tabSleepMinutes as TabSleepMinutes)
      : DEFAULT_SETTINGS.tabSleepMinutes,
    autoPictureInPicture:
      typeof source.autoPictureInPicture === 'boolean'
        ? source.autoPictureInPicture
        : DEFAULT_SETTINGS.autoPictureInPicture,
    pdfDarkMode:
      typeof source.pdfDarkMode === 'boolean' ? source.pdfDarkMode : DEFAULT_SETTINGS.pdfDarkMode,
    tabLayout: source.tabLayout === 'top' ? 'top' : 'sidebar',
    topBarStyle: source.topBarStyle === 'pill' ? 'pill' : 'flat',
    topBarAutoHide:
      typeof source.topBarAutoHide === 'boolean' ? source.topBarAutoHide : DEFAULT_SETTINGS.topBarAutoHide,
    blockThirdPartyCookies: source.blockThirdPartyCookies === true,
    clearCookiesOnExit: source.clearCookiesOnExit === true,
    cookieKeepSites: Array.isArray(source.cookieKeepSites)
      ? source.cookieKeepSites.filter((v): v is string => typeof v === 'string')
      : [],
    pinnedTools: normalizePinnedTools(source.pinnedTools),
    searchEngine: normalizeSearchEngine(source.searchEngine),
    reducedEffectsMode: normalizeReducedEffectsMode(source.reducedEffectsMode),
    spaceThemes: buildSpaceThemes(source.spaceThemes, source),
    customThemePresets: normalizeCustomThemePresets(source.customThemePresets),
    shieldWhitelist: normalizeShieldWhitelist(source.shieldWhitelist),
    archiveIntervalHours: isArchiveIntervalHours(source.archiveIntervalHours)
      ? source.archiveIntervalHours
      : DEFAULT_SETTINGS.archiveIntervalHours,
    rightPinsDockVisible: boolOr(source.rightPinsDockVisible, DEFAULT_SETTINGS.rightPinsDockVisible),
    sidebarExpanded: boolOr(source.sidebarExpanded, DEFAULT_SETTINGS.sidebarExpanded),
    sidebarAutoHide: boolOr(source.sidebarAutoHide, DEFAULT_SETTINGS.sidebarAutoHide),
    textScale: normalizeTextScale(source.textScale),
    homeWidgets: normalizeHomeWidgets(source.homeWidgets),
    uiScale: normalizeUiScale(source.uiScale),
  };
}
