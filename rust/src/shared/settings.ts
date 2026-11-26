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
  /** Widgets and clock on the new-tab page. */
  homeWidgets: HomeWidgets;
  /** What fills the new-tab page. */
  homeBackground: HomeBackground;
  /** Version of the setup screen last completed (0: never). */
  setupVersion: number;
  /** The first-run tour was played or dismissed. */
  tourDone: boolean;
  /** Separate sizes for the sidebar, dock and new-tab parts. */
  uiScale: UiScale;
  [unknownKey: string]: unknown; // kept as saved, e.g. the legacy `theme` key
}

/** Widget kinds: built-in ones, and app widgets (`app:<id>`, see home/apps.ts). */
export type WidgetKind =
  | 'github'
  | 'weather'
  | 'notes'
  | 'calendar'
  | 'topsites'
  | 'worldclock'
  | 'focus'
  | `app:${string}`;
/** Grid sizes in cells (columns x rows): s 2x2, m 4x2, l 4x4, w 8x2. */
export type WidgetSize = 's' | 'm' | 'l' | 'w';

export const WIDGET_SPAN: Readonly<Record<WidgetSize, readonly [number, number]>> = Object.freeze({
  s: [2, 2] as const,
  m: [4, 2] as const,
  l: [4, 4] as const,
  w: [8, 2] as const,
});

/** Columns of the new-tab grid (rows follow the window's height). */
export const WIDGET_COLS = 12;

export interface WidgetItem {
  id: string;
  kind: WidgetKind;
  size: WidgetSize;
  /** Top-left cell (column, row). */
  x: number;
  y: number;
  /** The widget's own settings: what it shows and how. */
  config: Record<string, unknown>;
}

/** The new-tab page: placed widgets, the clock, and widget data. */
export interface HomeWidgets {
  items: WidgetItem[];
  /** Text of each notes widget, by widget id. */
  notes: Record<string, string>;
  /** The widget guide was shown (it can be replayed from the edit bar). */
  guideSeen: boolean;
  /** `side`: the bottom corner the clock sits in. */
  clock: { hour12: boolean; seconds: boolean; side: 'left' | 'right' };
}

export const DEFAULT_HOME_WIDGETS: Readonly<HomeWidgets> = Object.freeze({
  items: [],
  notes: {},
  guideSeen: false,
  clock: { hour12: false, seconds: true, side: 'right' as const },
});

const WIDGET_SIZES: readonly WidgetSize[] = ['s', 'm', 'l', 'w'];
const KNOWN_KINDS = ['github', 'weather', 'notes', 'calendar', 'topsites', 'worldclock', 'focus'];

export interface CellRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Whether cell rectangles overlap. */
export function rectsOverlap(a: CellRect, b: CellRect): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

/** First free top-left cell (row by row) for a `w` x `h` widget, or null. */
export function firstFreeCell(placed: CellRect[], w: number, h: number, cols: number, rows: number): { x: number; y: number } | null {
  for (let y = 0; y + h <= rows; y++) {
    for (let x = 0; x + w <= cols; x++) {
      const r = { x, y, w, h };
      if (!placed.some((p) => rectsOverlap(p, r))) return { x, y };
    }
  }
  return null;
}

function normalizeHomeWidgets(raw: unknown): HomeWidgets {
  const o = raw !== null && typeof raw === 'object' ? (raw as Record<string, any>) : {};
  const g = o.github && typeof o.github === 'object' ? o.github : {};
  const w = o.weather && typeof o.weather === 'object' ? o.weather : {};
  const num = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : null);
  const notes: Record<string, string> = {};
  const loose: { id: string; kind: WidgetKind; size: WidgetSize; x: number | null; y: number | null; config: Record<string, unknown> }[] = [];
  const legacyConfig = (kind: string): Record<string, unknown> => {
    // 0.4.x kept one GitHub user and one place for every widget of the kind.
    if (kind === 'github' && typeof g.user === 'string' && g.user) return { user: g.user.slice(0, 39) };
    if (kind === 'weather' && num(w.lat) !== null && num(w.lon) !== null) {
      return { place: typeof w.place === 'string' ? w.place.slice(0, 80) : '', lat: num(w.lat), lon: num(w.lon), fahrenheit: w.fahrenheit === true };
    }
    return {};
  };
  if (Array.isArray(o.items)) {
    const seen = new Set<string>();
    for (const it of o.items) {
      if (!it || typeof it.id !== 'string' || typeof it.kind !== 'string' || seen.has(it.id)) continue;
      if (!KNOWN_KINDS.includes(it.kind) && !it.kind.startsWith('app:')) continue;
      seen.add(it.id);
      const config = it.config && typeof it.config === 'object' && !Array.isArray(it.config) ? { ...it.config } : legacyConfig(it.kind);
      loose.push({
        id: it.id,
        kind: it.kind as WidgetKind,
        size: WIDGET_SIZES.includes(it.size) ? it.size : 'm',
        x: Number.isInteger(it.x) ? it.x : null,
        y: Number.isInteger(it.y) ? it.y : null,
        config,
      });
    }
    if (o.notes && typeof o.notes === 'object' && !('text' in o.notes)) {
      for (const [k, v] of Object.entries(o.notes)) if (typeof v === 'string') notes[k] = v.slice(0, 4000);
    }
  } else {
    // 0.4.6 kept one flag per widget; turn the enabled ones into placed widgets.
    if (w.enabled === true) loose.push({ id: 'weather-1', kind: 'weather', size: 'm', x: null, y: null, config: legacyConfig('weather') });
    if (o.notes?.enabled === true) {
      loose.push({ id: 'notes-1', kind: 'notes', size: 'm', x: null, y: null, config: {} });
      if (typeof o.notes.text === 'string') notes['notes-1'] = o.notes.text.slice(0, 4000);
    }
    if (g.enabled === true) loose.push({ id: 'github-1', kind: 'github', size: 'w', x: null, y: null, config: legacyConfig('github') });
  }
  // Widgets from before positions existed (0.4.7: an ordered list) are laid
  // out in their order, row by row; positioned ones keep their cells unless
  // they overlap an earlier one or run off the grid.
  const placed: CellRect[] = [];
  const items: WidgetItem[] = [];
  for (const it of loose.slice(0, 24)) {
    const [cw, ch] = WIDGET_SPAN[it.size];
    let pos: { x: number; y: number } | null = null;
    if (it.x !== null && it.y !== null && it.x >= 0 && it.y >= 0 && it.x + cw <= WIDGET_COLS && it.y < 64) {
      const r = { x: it.x, y: it.y, w: cw, h: ch };
      if (!placed.some((p) => rectsOverlap(p, r))) pos = { x: it.x, y: it.y };
    }
    pos ??= firstFreeCell(placed, cw, ch, WIDGET_COLS, 64);
    if (!pos) continue;
    placed.push({ ...pos, w: cw, h: ch });
    items.push({ id: it.id, kind: it.kind, size: it.size, x: pos.x, y: pos.y, config: it.config });
  }
  const c = o.clock && typeof o.clock === 'object' ? o.clock : {};
  return {
    items,
    notes,
    guideSeen: o.guideSeen === true,
    clock: { hour12: c.hour12 === true, seconds: c.seconds !== false, side: c.side === 'left' ? 'left' : 'right' },
  };
}

/** What fills the new-tab page behind the widgets. */
export type HomeBackgroundMode = 'theme' | 'solid' | 'gradient' | 'image' | 'special';

export interface GradientStop {
  color: string;
  /** 0..100 */
  at: number;
}

export interface HomeBackground {
  mode: HomeBackgroundMode;
  solid: string;
  gradient: { type: 'linear' | 'radial'; angle: number; stops: GradientStop[] };
  image: { file: string; fit: 'cover' | 'contain' | 'center'; dim: number };
  /** Serica Special: seconds each artwork stays before cross-fading to the next. */
  special: { seconds: number };
}

export const DEFAULT_HOME_BACKGROUND: Readonly<HomeBackground> = Object.freeze({
  mode: 'theme' as HomeBackgroundMode,
  solid: '#1E2A26',
  gradient: {
    type: 'linear' as const,
    angle: 160,
    stops: [
      { color: '#283C63', at: 0 },
      { color: '#B06A6A', at: 55 },
      { color: '#F0B27A', at: 100 },
    ],
  },
  image: { file: '', fit: 'cover' as const, dim: 0.15 },
  special: { seconds: 45 },
});

const HEX = /^#[0-9a-f]{6}$/i;

function normalizeHomeBackground(raw: unknown): HomeBackground {
  const o = raw !== null && typeof raw === 'object' ? (raw as Record<string, any>) : {};
  const d = DEFAULT_HOME_BACKGROUND;
  const mode: HomeBackgroundMode = ['theme', 'solid', 'gradient', 'image', 'special'].includes(o.mode) ? o.mode : d.mode;
  const g = o.gradient && typeof o.gradient === 'object' ? o.gradient : {};
  const stops: GradientStop[] = Array.isArray(g.stops)
    ? g.stops
        .filter((s: any) => s && typeof s.color === 'string' && HEX.test(s.color))
        .map((s: any) => ({ color: s.color.toUpperCase(), at: Math.max(0, Math.min(100, Number(s.at) || 0)) }))
        .slice(0, 5)
    : [];
  const img = o.image && typeof o.image === 'object' ? o.image : {};
  const file = typeof img.file === 'string' && /^[A-Za-z0-9._-]{1,99}$/.test(img.file) ? img.file : '';
  const sp = o.special && typeof o.special === 'object' ? o.special : {};
  const seconds = Number(sp.seconds);
  return {
    mode: mode === 'image' && !file ? 'theme' : mode,
    solid: typeof o.solid === 'string' && HEX.test(o.solid) ? o.solid.toUpperCase() : d.solid,
    gradient: {
      type: g.type === 'radial' ? 'radial' : 'linear',
      angle: Number.isFinite(Number(g.angle)) && g.angle !== undefined ? ((Math.round(Number(g.angle)) % 360) + 360) % 360 : d.gradient.angle,
      stops: stops.length >= 2 ? stops.sort((a, b) => a.at - b.at) : d.gradient.stops.map((s) => ({ ...s })),
    },
    image: {
      file,
      fit: img.fit === 'contain' || img.fit === 'center' ? img.fit : 'cover',
      dim: Math.max(0, Math.min(0.7, Number.isFinite(Number(img.dim)) && img.dim !== undefined ? Number(img.dim) : d.image.dim)),
    },
    special: { seconds: Number.isFinite(seconds) && sp.seconds !== undefined ? Math.max(15, Math.min(600, Math.round(seconds))) : d.special.seconds },
  };
}

/** CSS `background` for solid and gradient modes. */
export function homeBackgroundCss(bg: HomeBackground): string | null {
  if (bg.mode === 'solid') return bg.solid;
  if (bg.mode === 'gradient') {
    const stops = bg.gradient.stops.map((s) => `${s.color} ${s.at}%`).join(', ');
    return bg.gradient.type === 'radial' ? `radial-gradient(circle at 50% 40%, ${stops})` : `linear-gradient(${bg.gradient.angle}deg, ${stops})`;
  }
  return null;
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
  homeBackground: DEFAULT_HOME_BACKGROUND,
  setupVersion: 0,
  tourDone: false,
  uiScale: DEFAULT_UI_SCALE,
});

/** The setup screen this build shows to anyone who hasn't completed it. */
export const SETUP_VERSION = 2;

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
    homeBackground: normalizeHomeBackground(source.homeBackground),
    setupVersion: typeof source.setupVersion === 'number' && Number.isFinite(source.setupVersion) ? Math.max(0, Math.floor(source.setupVersion)) : 0,
    tourDone: source.tourDone === true,
    uiScale: normalizeUiScale(source.uiScale),
  };
}
