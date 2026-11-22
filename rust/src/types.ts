import type {
  BrowserSettings,
  PerformanceStatus,
  ReducedEffectsMode,
  SpaceTheme,
  SpaceThemesMap,
} from './shared/settings';
import type {
  CustomSpaceThemePreset,
  SpaceThemeMode,
  SpaceThemePoint,
  SpaceThemePreset,
  ThemePalette,
} from './shared/spaceTheme';

export type {
  BrowserSettings,
  CustomSpaceThemePreset,
  PerformanceStatus,
  ReducedEffectsMode,
  SpaceTheme,
  SpaceThemesMap,
  SpaceThemeMode,
  SpaceThemePoint,
  SpaceThemePreset,
  ThemePalette,
};

import type { FavoriteEntry, FolderRecord, TabSection } from './shared/profileTypes';
import type { LayoutSection, MoveOp } from './shared/layout';

export type { FavoriteEntry, FolderRecord, TabSection, LayoutSection, MoveOp };

export interface Space {
  id: string;
  name: string;
  icon: string;
  color: string;
  /** Owning profile (R1.10). Absent only in the renderer's pre-IPC placeholder state. */
  profileId?: string;
  /** Position within the owning profile's spaces in the Space_Switcher. */
  order?: number;
}

/** A profile as sent by the main process, with its session partition (R1.1). */
export interface Profile {
  id: string;
  name: string;
  createdAt: number;
  /** Set by `listProfiles`/`createProfile`/`renameProfile`; not part of `state-update`. */
  partition?: string;
}

export interface ProfileList {
  profiles: Profile[];
  spaces: Space[];
  activeProfileId: string;
  activeSpaceId: string;
}

/** Common result of the profile/space/layout IPC; `error` is a short code such as `last-space`. */
export type IpcResult<T = {}> = ({ ok: true } & T) | { ok: false; error: string };

export interface SpacePatch {
  name?: string;
  color?: string;
  icon?: string;
}

/** Sent when a tab must remount its webview in another profile's session (R1.12, R17.5). */
export interface TabPartitionChange {
  tabId: string;
  partition: string;
  url: string;
}

/** Sidebar UI preferences persisted through the settings IPC. */
export type SidebarState = Pick<
  BrowserSettings,
  'sidebarExpanded' | 'sidebarAutoHide' | 'sidebarWidth' | 'rightPinsDockVisible' | 'archiveIntervalHours'
>;

export type SpaceState = Space;

export interface TabState {
  id: string;
  spaceId: string;
  title: string;
  url: string;
  favicon?: string;
  isLoading: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
  isPinned: boolean;
  isSleeping: boolean;
  hasActiveMedia?: boolean;
  blockedCount: number;
  zoomFactor?: number;
  previewImage?: string;
  /** Session partition of the profile that owns the tab's space (R1.2). */
  partition?: string;
  /** Sidebar section; `isPinned` mirrors `section === 'pinned'`. */
  section?: TabSection;
  /** Closed pinned tab: entry kept, page not loaded (R15.3). */
  unloaded?: boolean;
  /** Epoch ms of the last time this tab was active (R16.4). */
  lastActiveAt?: number;
  /** Optional folder in the pinned section of this space. */
  folderId?: string | null;
}

export interface ActiveMediaState {
  tabId: string;
  tabTitle: string;
  url: string;
  isPlaying: boolean;
  currentTime: number;
  duration: number;
  volume: number;
  hasVideo: boolean;
  /** The tab's video is showing in a native picture-in-picture window. */
  inPip?: boolean;
}

export interface ShieldStats {
  totalBlocked: number;
  adsBlocked: number;
  trackersBlocked: number;
  analyticsBlocked: number;
  fingerprintersBlocked: number;
  dataSavedKb: number;
}

export interface ResourceStats {
  memorySavedMb: number;
  sleepingCount: number;
}

export interface BlockedTrackerItem {
  url: string;
  domain: string;
  category?: string;
  rule?: string;
  timestamp: number;
}

export interface BrowserAppState {
  spaces: Space[];
  activeSpaceId: string;
  tabs: TabState[];
  activeTabId: string | null;
  shieldStats: ShieldStats;
  resourceStats: ResourceStats;
  /** Absent until the first `state-update` from the main process. */
  profiles?: Profile[];
  activeProfileId?: string;
  favorites?: FavoriteEntry[];
  folders?: FolderRecord[];
}

export interface ThemeConfig {
  id: string;
  name: string;
  mode: 'dark' | 'light';
  bgApp: string;
  bgSidebar: string;
  bgCard: string;
  bgInput: string;
  accent: string;
  textMain: string;
  textMuted: string;
  borderSubtle: string;
  fontSize?: number;
}

/** Sent when Google refuses a sign-in inside Neutrino (see src/main/authHandoff.ts). */
export interface AuthHandoffNotice {
  handoffId: string;
  provider: 'google';
  reason: 'browser-not-secure' | 'disallowed-useragent';
  siteHost: string;
  isPopup: boolean;
}

export type { HistoryEntry, HistoryClearRange, HistoryQueryOptions } from './shared/ipcTypes';
export type {
  VpnMode,
  DohProviderId,
  ProxyProtocol,
  VpnConfig,
  VpnStatus,
  VpnIpCheckResult,
} from './shared/ipcTypes';
import type { HistoryEntry, HistoryClearRange, HistoryQueryOptions } from './shared/ipcTypes';
import type { VpnConfig, VpnStatus, VpnIpCheckResult } from './shared/ipcTypes';

export type AuthHandoffResult = { ok: true; siteHost: string } | { ok: false; error: string };

export type DownloadState = 'inProgress' | 'paused' | 'completed' | 'canceled' | 'failed';

/** A download tracked by the download manager (src-tauri/src/downloads.rs). */
export interface DownloadItem {
  id: string;
  url: string;
  filename: string;
  path: string;
  mime: string;
  /** -1 when unknown. */
  totalBytes: number;
  receivedBytes: number;
  bytesPerSec: number;
  state: DownloadState;
  error?: string | null;
  canResume: boolean;
  startedAt: number;
  endedAt?: number | null;
  tabId?: string | null;
  fileExists: boolean;
}

export type DownloadAction = 'pause' | 'resume' | 'cancel' | 'open' | 'show' | 'remove' | 'retry';

/** A browser extension installed from the Chrome Web Store (src-tauri/src/extensions.rs). */
export interface ExtensionInfo {
  id: string;
  runtimeId?: string;
  name: string;
  version: string;
  description: string;
  enabled: boolean;
  icon?: string | null;
  popup?: string | null;
  options?: string | null;
  permissions: string[];
  source: string;
  installedAt: number;
}

export type SiteAction = 'back' | 'forward' | 'reload' | 'hard-reload' | 'stop' | 'home' | 'open-find';

declare global {
  interface Window {
    neutrino?: {
      isElectron: boolean;
      isTauri?: boolean;
      // Native tab viewport (Rust tab host)
      siteAction?: (action: SiteAction) => Promise<void>;
      setViewport?: (x: number, y: number, width: number, height: number) => Promise<void>;
      freezeViewport?: () => Promise<string | null>;
      thawViewport?: () => Promise<void>;
      findInPage?: (query: string | null, options?: { forward?: boolean; findNext?: boolean }) => Promise<void>;
      onFindResult?: (cb: (r: { tabId: string; activeMatch: number; totalMatches: number }) => void) => () => void;
      onPageColor?: (cb: (r: { tabId: string; themeColor: string | null }) => void) => () => void;
      onHtmlFullscreen?: (cb: (isFullscreen: boolean) => void) => () => void;
      getMediaState?: (tabId: string) => Promise<Omit<ActiveMediaState, 'tabId' | 'url'> | null>;
      openDevtools?: () => Promise<void>;
      historySuggest?: (text: string, limit?: number) => Promise<HistoryEntry[]>;
      getShieldsInfo?: () => Promise<{ engine: string; lists: string[]; lastUpdated: number | null; updating: boolean; lastError: string | null }>;
      updateFilterLists?: () => Promise<void>;
      mcpServerStatus?: () => Promise<{ enabled: boolean; running: boolean; port: number; url: string; token: string; error?: string | null }>;
      mcpRegenerateToken?: () => Promise<{ enabled: boolean; running: boolean; port: number; url: string; token: string; error?: string | null }>;
      listDownloads?: () => Promise<DownloadItem[]>;
      downloadAction?: (id: string, action: DownloadAction) => Promise<boolean>;
      clearDownloads?: () => Promise<DownloadItem[]>;
      openDownloadsFolder?: () => Promise<void>;
      onDownloadStarted?: (cb: (item: DownloadItem) => void) => () => void;
      onDownloadUpdated?: (cb: (item: DownloadItem) => void) => () => void;
      onDownloadsChanged?: (cb: (list: DownloadItem[]) => void) => () => void;
      onOpenDownloads?: (cb: () => void) => () => void;
      listExtensions?: () => Promise<ExtensionInfo[]>;
      installExtension?: (input: string) => Promise<ExtensionInfo>;
      setExtensionEnabled?: (id: string, enabled: boolean) => Promise<boolean>;
      removeExtension?: (id: string) => Promise<boolean>;
      openExtensionPage?: (id: string, page: 'popup' | 'options') => Promise<boolean>;
      onExtensionsUpdated?: (cb: (list: ExtensionInfo[]) => void) => () => void;
      onExtensionInstalled?: (cb: (ext: ExtensionInfo) => void) => () => void;
      assistantOpen?: (url: string, x: number, y: number, width: number, height: number) => Promise<void>;
      assistantBounds?: (x: number, y: number, width: number, height: number) => Promise<void>;
      assistantClose?: () => Promise<void>;
      assistantInsertText?: (text: string) => Promise<boolean>;
      navigate: (url: string) => Promise<void>;
      goBack: () => Promise<void>;
      goForward: () => Promise<void>;
      reload: () => Promise<void>;
      hardReload?: () => Promise<void>;
      stopLoading?: () => Promise<void>;
      reopenClosedTab?: () => Promise<TabState | null>;
      cycleTab?: (direction: 1 | -1) => Promise<TabState | null>;
      openExternalUrl?: (url: string) => Promise<boolean>;
      onSiteAction?: (cb: (payload: { action: SiteAction; source?: string }) => void) => () => void;
      onSpaceSwitched?: (cb: (payload: { spaceId: string; spaceName: string }) => void) => () => void;
      getState?: () => Promise<BrowserAppState>;
      createTab: (url?: string, spaceId?: string) => Promise<TabState>;
      updateTab?: (tabId: string, partial: Partial<TabState>) => Promise<void>;
      closeTab: (tabId: string) => Promise<void>;
      selectTab: (tabId: string) => Promise<void>;
      togglePinTab: (tabId: string) => Promise<void>;
      toggleSleepTab: (tabId: string) => Promise<void>;
      selectSpace: (spaceId: string) => Promise<void>;
      /** `profileId` defaults to the active profile (R1.11). Resolves `null` for an unknown profile. */
      createSpace?: (name: string, color?: string, profileId?: string) => Promise<Space | null>;
      /** Rejects with `last-space` for a profile's only space (R13.9). */
      deleteSpace?: (spaceId: string) => Promise<IpcResult>;
      updateSpace?: (spaceId: string, patch: SpacePatch) => Promise<IpcResult<{ space: Space }>>;
      reorderSpace?: (spaceId: string, toIndex: number) => Promise<IpcResult>;

      // Profiles (R1.5, R1.6, R1.8, R1.9)
      listProfiles?: () => Promise<ProfileList>;
      createProfile?: (name?: string) => Promise<IpcResult<{ profile: Profile; space: Space }>>;
      renameProfile?: (profileId: string, name: string) => Promise<IpcResult<{ profile: Profile }>>;
      /** Errors: `unknown-profile`, `last-profile`, `erase-failed`. */
      deleteProfile?: (profileId: string) => Promise<IpcResult>;
      onTabPartitionChanged?: (cb: (change: TabPartitionChange) => void) => () => void;

      // Sidebar layout (R14, R15, R17)
      moveTab?: (op: MoveOp) => Promise<IpcResult<{ needsReload: boolean }>>;
      pinTab?: (tabId: string) => Promise<IpcResult>;
      unpinTab?: (tabId: string) => Promise<IpcResult>;
      addFavorite?: (tabId: string) => Promise<IpcResult>;
      removeFavorite?: (favoriteId: string) => Promise<IpcResult>;
      duplicateTab?: (id: string, spaceId?: string) => Promise<IpcResult<{ tab: TabState }>>;

      // Persistent Folders in Pinned section
      createFolder?: (spaceId: string, name: string) => Promise<IpcResult<{ folder: FolderRecord }>>;
      updateFolder?: (folderId: string, patch: Partial<FolderRecord>) => Promise<IpcResult<{ folder: FolderRecord }>>;
      deleteFolder?: (folderId: string) => Promise<IpcResult>;
      moveTabToFolder?: (tabId: string, folderId: string | null) => Promise<IpcResult>;

      // Sidebar state, persisted through the settings IPC
      updateSidebarState?: (patch: Partial<SidebarState>) => Promise<BrowserSettings>;
      reorderTabs?: (draggedTabId: string, targetTabId: string) => Promise<void>;
      setZoomFactor?: (factor: number) => Promise<void>;
      onZoomUpdated?: (cb: (data: { tabId?: string; zoomFactor: number }) => void) => () => void;
      toggleShields: () => Promise<boolean>;
      toggleSiteWhitelist?: (domainOrUrl: string) => Promise<{ domain: string; whitelisted: boolean; shieldWhitelist: string[] }>;
      resetShieldStats?: () => Promise<ShieldStats>;
      zenOptimize: () => Promise<{ memorySavedMb: number; sleepingCount: number }>;
      getShieldStats: () => Promise<ShieldStats>;
      callMcp: (method: string, params?: unknown) => Promise<any>;
      getMcpLogs: () => Promise<any[]>;
      mediaControl?: (action: string, params?: any) => Promise<any>;
      captureTabPreview?: (tabUrl: string) => Promise<string | null>;
      captureTabPreviewById?: (tabId: string) => Promise<string | null>;
      getTabPreview?: (tabId: string) => Promise<string | null>;
      registerTabWebview?: (tabId: string, webContentsId: number) => Promise<boolean>;
      sampleTabTopColor?: (tabId: string) => Promise<{ r: number; g: number; b: number; a: number } | null>;
      windowControl: (action: 'minimize' | 'maximize' | 'close') => Promise<void>;
      setThemeMode?: (mode: 'dark' | 'light' | 'system') => Promise<{ success: boolean; isDark?: boolean }>;
      getSettings?: () => Promise<BrowserSettings>;
      getPerformanceStatus?: () => Promise<PerformanceStatus>;
      updateSettings?: (patch: Partial<BrowserSettings>) => Promise<BrowserSettings>;
      onSettingsUpdated?: (cb: (settings: BrowserSettings) => void) => () => void;
      onPerformanceStatusUpdated?: (cb: (status: PerformanceStatus) => void) => () => void;
      onOpenSettings?: (cb: () => void) => () => void;
      onStateUpdate: (cb: (state: BrowserAppState) => void) => () => void;
      onTrackerBlocked: (cb: (item: BlockedTrackerItem) => void) => () => void;
      onCloseActiveTab?: (cb: () => void) => () => void;
      onOpenCommandBar?: (cb: (opts?: { newTab?: boolean }) => void) => () => void;
      onToggleTabs?: (cb: () => void) => () => void;
      onEscapePressed?: (cb: () => void) => () => void;
      onAuthHandoffRequired?: (cb: (notice: AuthHandoffNotice) => void) => () => void;
      openAuthHandoff?: (handoffId: string) => Promise<AuthHandoffResult>;
      copyAuthHandoffLink?: (handoffId: string) => Promise<AuthHandoffResult>;
      dismissAuthHandoff?: (handoffId: string) => Promise<AuthHandoffResult>;
      getHistory?: (options?: HistoryQueryOptions) => Promise<HistoryEntry[]>;
      deleteHistoryEntry?: (target: { id?: string; url?: string; timestamp?: number }) => Promise<boolean>;
      clearHistory?: (range?: HistoryClearRange) => Promise<number>;
      onHistoryUpdated?: (cb: (entries: HistoryEntry[]) => void) => () => void;
      onOpenHistory?: (cb: () => void) => () => void;
      getVpnStatus?: () => Promise<VpnStatus>;
      connectVpn?: (patch?: Partial<VpnConfig>) => Promise<VpnStatus>;
      disconnectVpn?: () => Promise<VpnStatus>;
      updateVpnConfig?: (patch: Partial<VpnConfig>) => Promise<VpnStatus>;
      checkVpnIp?: () => Promise<VpnIpCheckResult | { error: string }>;
      onVpnStatusUpdated?: (cb: (status: VpnStatus) => void) => () => void;

      // Archive (R16.7, R16.8): the Active_Profile's archived tabs, newest first
      listArchive?: () => Promise<ArchiveListEntry[]>;
      /** Errors: `forbidden`, `unknown-archived-tab`, `restore-failed`. */
      restoreArchivedTab?: (archivedId: string) => Promise<IpcResult<{ tab: TabState }>>;
      onArchiveUpdated?: (cb: (entries: ArchiveListEntry[]) => void) => () => void;
      // Password manager, Settings side (R9, R10, R11). Only revealPassword can carry plaintext.
      getPasswordStatus?: () => Promise<PasswordStatusResult>;
      listPasswords?: () => Promise<CredentialListItem[]>;
      editPassword?: (
        credentialId: string,
        patch: { username?: string; password?: string },
      ) => Promise<PasswordEditResult>;
      deletePassword?: (credentialId: string) => Promise<PasswordSimpleResult>;
      revealPassword?: (credentialId: string) => Promise<RevealResult | PasswordIpcErrorStatus>;
      /** Main writes the clipboard; never returns plaintext. */
      copyPassword?: (credentialId: string) => Promise<CopyResult | PasswordIpcErrorStatus>;
      confirmPasswordAction?: (
        confirmId: string,
        accepted: boolean,
      ) => Promise<RevealResult | CopyResult | PasswordIpcErrorStatus>;
      getPasswordNeverList?: () => Promise<string[]>;
      removeFromPasswordNeverList?: (origin: string) => Promise<string[]>;
      importPasswordsCsv?: () => Promise<PasswordImportStartResult>;
      importPasswordsFromBrowser?: () => Promise<PasswordImportStartResult>;
      commitPasswordImport?: (
        importId: string,
        choices?: Record<number, ConflictChoice>,
      ) => Promise<CommitImportResult | PasswordIpcError>;
      cancelPasswordImport?: (importId: string) => Promise<PasswordSimpleResult>;
      // Password save/update prompts (R6.2, R6.3, R7.2). Events carry origin and username only.
      onPasswordPrompt?: (cb: (prompt: PromptInfo) => void) => () => void;
      onPasswordPromptClosed?: (cb: (payload: { promptId: string }) => void) => () => void;
      respondToPasswordPrompt?: (
        promptId: string,
        action: PromptAction,
      ) => Promise<PromptResult | PasswordIpcError>;
    };
  }
}

// --- Password manager IPC shapes (src/main/index.ts registerPasswordIpc) ---
export type {
  CredentialListItem,
  RevealResult,
  CopyResult,
  ConflictChoice,
  CommitImportResult,
  PrepareImportResult,
  ImportConflictInfo,
  NotPossibleCause,
  CsvExportSteps,
  DirectImportNotPossible,
};
import type { CredentialListItem } from './shared/ipcTypes';
import type {
  RevealResult,
  CopyResult,
  ConflictChoice,
  CommitImportResult,
  PrepareImportResult,
  ImportConflictInfo,
} from './shared/ipcTypes';
import type { NotPossibleCause, CsvExportSteps, DirectImportNotPossible } from './shared/ipcTypes';

/** Gate and parse failures from the IPC layer (`forbidden`, `invalid-input`) or the manager. */
export interface PasswordIpcError {
  ok: false;
  error: string;
  message: string;
}
export type PasswordIpcErrorStatus = { status: 'error' } & PasswordIpcError;
export type PasswordStatusResult = { ok: true; encryptionAvailable: boolean } | PasswordIpcError;
export type PasswordSimpleResult = { ok: true } | PasswordIpcError;
export type PasswordEditResult = { ok: true; credential: CredentialListItem } | PasswordIpcError;
/** CSV or direct import: prepared (counts, conflicts, no passwords), not possible, or an error. */
export type PasswordImportStartResult =
  | (PrepareImportResult & { browsers?: Array<'chrome' | 'edge'> })
  | DirectImportNotPossible
  | PasswordIpcError;

/** An archived tab in the archive list, with the name of its space (null when the space was deleted). */
export type { ArchiveListEntry } from './shared/ipcTypes';
import type { ArchiveListEntry } from './shared/ipcTypes';

/** Password save/update prompt shapes (src/main/passwords/manager.ts). No password field. */
export type { PromptInfo, PromptAction, PromptResult } from './shared/ipcTypes';
import type { PromptInfo, PromptAction, PromptResult } from './shared/ipcTypes';
