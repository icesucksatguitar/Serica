import React, { Suspense, lazy, useEffect, useLayoutEffect, useMemo, useState, useRef, useCallback } from 'react';
import { useBrowser } from './hooks/useBrowser';
import { useProfiles } from './hooks/useProfiles';
import { Sidebar } from './components/Sidebar';
import { buildSidebarView } from './utils/sidebarModel';
import { selectSpaceByIndex } from './shared/layout';
import { FloatingUrlPill } from './components/FloatingUrlPill';
import { BrowserViewport } from './components/BrowserViewport';
import { RightPinsDock, type PinnedSite } from './components/RightPinsDock';
import { TopTabStrip } from './components/TopTabStrip';
import { ToolsMenu } from './components/ToolsMenu';
import { AuthHandoffNotice } from './components/AuthHandoffNotice';
import { UpdateNotice } from './components/UpdateNotice';
import { WhatsNew } from './components/WhatsNew';
import { PasswordPrompt } from './components/PasswordPrompt';
import { PageDialog } from './components/PageDialog';
import { PermissionPrompt } from './components/PermissionPrompt';
import { DropZones } from './components/DropZones';
import type { UserProfile, WelcomePrefs } from './components/WelcomeModal';
import { useSettings } from './hooks/useSettings';
import { prewarmPageUnderlay, useViewportOverlay, useViewportOverlayCount } from './hooks/useViewportOverlay';
import { TOP_BAR_HEIGHT, TOP_BAR_REVEAL_STRIP, useTopBarAutoHide } from './hooks/useTopBarAutoHide';
import { resolveSpaceTheme, resolveReducedEffectsActive } from './shared/settings';
import { type SpaceTheme, getDefaultSpaceTheme } from './shared/spaceTheme';
import { applySpaceThemeToRoot, getOsPrefersDark } from './utils/applySpaceTheme';
import { computeTopBarTint, type PageColorReport, type TabPageColor } from './utils/topBarTint';
import { ArrowLeft, ArrowRight, RotateCw, Home, X, Shield, Lock, Download, Plus, PanelLeft } from 'lucide-react';
import { useDownloads, aggregateProgress, isActiveDownload } from './hooks/useDownloads';
import {
  TabState,
  ActiveMediaState,
  HistoryEntry,
  HistoryClearRange,
  VpnConfig,
  VpnStatus,
  SiteAction,
  PerformanceStatus,
} from './types';
import { AppContextMenu } from './components/AppContextMenu';
import { PageContextMenu } from './components/PageContextMenu';
import { normalizePinnedTools, type ToolActions } from './shared/tools';
import './styles/theme.css';
import './styles/system.css';
import './styles/settings.css';
import './styles/panels.css';
import './styles/sidebar-v2.css';
import './styles/foundation.css';
import './styles/features.css';
import './styles/chrome.css';

const ArchiveList = lazy(() =>
  import('./components/ArchiveList').then((m) => ({ default: m.ArchiveList }))
);
const CommandBar = lazy(() =>
  import('./components/CommandBar').then((m) => ({ default: m.CommandBar }))
);
const ShieldModal = lazy(() =>
  import('./components/ShieldModal').then((m) => ({ default: m.ShieldModal }))
);
const McpModal = lazy(() =>
  import('./components/McpModal').then((m) => ({ default: m.McpModal }))
);
const ThemeCustomizerModal = lazy(() =>
  import('./components/ThemeCustomizerModal').then((m) => ({ default: m.ThemeCustomizerModal }))
);
const MediaPlayer = lazy(() =>
  import('./components/MediaPlayer').then((m) => ({ default: m.MediaPlayer }))
);
const WelcomeModal = lazy(() =>
  import('./components/WelcomeModal').then((m) => ({ default: m.WelcomeModal }))
);
const SettingsPanel = lazy(() =>
  import('./components/SettingsPanel').then((m) => ({ default: m.SettingsPanel }))
);
const DownloadsPanel = lazy(() =>
  import('./components/DownloadsPanel').then((m) => ({ default: m.DownloadsPanel }))
);
const ExtensionsModal = lazy(() =>
  import('./components/ExtensionsModal').then((m) => ({ default: m.ExtensionsModal }))
);
const HistoryModal = lazy(() =>
  import('./components/HistoryModal').then((m) => ({ default: m.HistoryModal }))
);
const VpnModal = lazy(() =>
  import('./components/VpnModal').then((m) => ({ default: m.VpnModal }))
);

export function App() {
  const browser = useBrowser();
  const {
    state,
    activeTab,
    currentSpaceTabs,
    blockedTrackers,
    isCommandBarOpen,
    setIsCommandBarOpen,
    isShieldModalOpen,
    setIsShieldModalOpen,
    isMcpModalOpen,
    setIsMcpModalOpen,
    navigate,
    createTab,
    closeTab,
    reopenClosedTab,
    selectTab,
    toggleSleepTab,
    selectSpace,
    createSpace,
    deleteSpace,
    reorderTabs,
    toggleShields,
    resetShieldStats,
  } = browser;
  const { activeProfile } = useProfiles();

  const { settings, updateSettings, isLoaded: settingsLoaded } = useSettings();
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const openSettings = useCallback(() => setIsSettingsOpen(true), []);

  const [isHistoryModalOpen, setIsHistoryModalOpen] = useState(false);
  const [isExtensionsOpen, setIsExtensionsOpen] = useState(false);
  const [isDownloadsOpen, setIsDownloadsOpen] = useState(false);
  // Like Chrome's download bubble: a new download opens the panel.
  const downloads = useDownloads(() => setIsDownloadsOpen(true));
  const activeDownloads = downloads.filter(isActiveDownload);
  const downloadProgress = aggregateProgress(downloads);
  const [historyEntries, setHistoryEntries] = useState<HistoryEntry[]>([]);
  const [isVpnModalOpen, setIsVpnModalOpen] = useState(false);
  const [vpnStatus, setVpnStatus] = useState<VpnStatus | null>(null);
  const [performanceStatus, setPerformanceStatus] = useState<PerformanceStatus | null>(null);
  const [isCheckingVpnIp, setIsCheckingVpnIp] = useState(false);
  const [vpnIpError, setVpnIpError] = useState<string | null>(null);

  const [isThemeModalOpen, setIsThemeModalOpen] = useState(false);
  const [isArchiveOpen, setIsArchiveOpen] = useState(false);

  // Sidebar: a full-height panel that floats over the page. It starts hidden and
  // opens from the toolbar button, Ctrl+S / Ctrl+E or the left edge of the window.
  const [sidebarOpen, setSidebarOpen] = useState(false);
  // Whether the sidebar is on screen (opened from the toggle, a shortcut or
  // the left edge): its toolbar row then holds the top bar's left controls.
  const [sidebarShown, setSidebarShown] = useState(false);
  const [toolsOpen, setToolsOpen] = useState(false);
  const setSidebarExpanded = useCallback((next: boolean) => setSidebarOpen(next), []);
  const toggleSidebar = useCallback(() => setSidebarOpen((o) => !o), []);
  const closeSidebar = useCallback(() => setSidebarOpen(false), []);

  // New tab control / Ctrl+T (R12.8): the Command_Bar opens in "new tab" mode and its
  // submission creates a Today tab in the Active_Space instead of navigating the current tab.
  const [isCommandBarNewTab, setIsCommandBarNewTab] = useState(false);
  const openCommandBar = useCallback(() => {
    setIsCommandBarNewTab(false);
    setIsCommandBarOpen(true);
  }, [setIsCommandBarOpen]);
  const openNewTabCommandBar = useCallback(() => {
    setIsCommandBarNewTab(true);
    setIsCommandBarOpen(true);
  }, [setIsCommandBarOpen]);
  useEffect(() => {
    if (!isCommandBarOpen) setIsCommandBarNewTab(false);
  }, [isCommandBarOpen]);
  // User Profile & First Launch Welcome Screen State
  const [userProfile, setUserProfile] = useState<UserProfile | null>(() => {
    try {
      const saved = (localStorage.getItem('serica_user_profile') || localStorage.getItem('neutrino_user_profile'));
      return saved ? JSON.parse(saved) : null;
    } catch {
      return null;
    }
  });

  const [isWelcomeOpen, setIsWelcomeOpen] = useState<boolean>(() => {
    try {
      return (localStorage.getItem('serica_onboarding_completed') || localStorage.getItem('neutrino_onboarding_completed')) === null;
    } catch {
      return false;
    }
  });

  const handleWelcomeComplete = (profile: UserProfile, prefs: WelcomePrefs) => {
    setUserProfile(profile);
    setIsWelcomeOpen(false);
    const dockPins: PinnedSite[] = prefs.dockSites.map((s) => ({ id: `pin-${s.id}`, title: s.title, url: s.url }));
    // The chosen theme goes to every space; Theme Studio can change each one later.
    const spaceThemes = Object.fromEntries(
      state.spaces.map((s) => [s.id, { ...prefs.spaceTheme, points: prefs.spaceTheme.points.map((p) => ({ ...p })) }]),
    );
    const palette = applySpaceThemeToRoot(prefs.spaceTheme, osPrefersDark);
    triggerThemeCrossFade();
    try {
      localStorage.setItem('serica_user_profile', JSON.stringify(profile));
      localStorage.setItem('serica_onboarding_completed', 'true');
      localStorage.setItem('serica_pinned_sites', JSON.stringify(dockPins));
    } catch { }
    void updateSettings({
      userProfile: profile,
      onboardingCompleted: true,
      theme: palette.effectiveMode,
      tabLayout: prefs.tabLayout,
      textScale: prefs.textScale,
      rightPinsDockVisible: prefs.dockVisible,
      pinnedSites: dockPins,
      spaceThemes: { ...(settings.spaceThemes || {}), ...spaceThemes },
    });
    if (prefs.sidebarSites.length > 0) {
      void window.serica?.addPinnedSites?.(
        prefs.sidebarSites.map((s) => ({ url: s.url, title: s.title })),
        state.activeSpaceId,
      );
    }
  };

  // Display name edited in the Settings panel; same key and shape WelcomeModal writes.
  const handleUserNameChange = useCallback(
    (name: string) => {
      const trimmed = name.trim();
      if (!trimmed) return;
      setUserProfile((prev) => {
        const base: UserProfile = prev ?? {
          name: trimmed,
          email: null,
          isGuest: true,
          theme: 'obsidian',
          mode: 'minimal',
        };
        const next: UserProfile = { ...base, name: trimmed };
        try {
          localStorage.setItem('serica_user_profile', JSON.stringify(next));
        } catch { }
        void updateSettings({ userProfile: next });
        return next;
      });
    },
    [updateSettings]
  );

  // Zen auto-sleep runs in the Rust backend (tabs idle for 10+ minutes); nothing to schedule here.

  useEffect(() => {
    void window.serica?.getPerformanceStatus?.().then((status) => {
      if (status) setPerformanceStatus(status);
    });
    const unsubPerf = window.serica?.onPerformanceStatusUpdated?.((status) => {
      if (status) setPerformanceStatus(status);
    });
    const syncHidden = () => {
      document.documentElement.setAttribute('data-window-hidden', document.hidden ? 'true' : 'false');
    };
    syncHidden();
    document.addEventListener('visibilitychange', syncHidden);
    return () => {
      unsubPerf?.();
      document.removeEventListener('visibilitychange', syncHidden);
    };
  }, []);

  const isReducedEffects = resolveReducedEffectsActive(
    settings.reducedEffectsMode,
    Boolean(performanceStatus?.gpuDisabledFlag),
    performanceStatus?.gpuFeatureStatus ?? null
  );

  useEffect(() => {
    document.documentElement.setAttribute('data-reduced-effects', isReducedEffects ? 'on' : 'off');
  }, [isReducedEffects]);

  // Active webpage detection: on web pages, the viewport is pulled down below the top bar
  // so window controls, tabs, and site navigation controls never collide.
  const isWebpage = Boolean(
    activeTab?.url &&
    activeTab.url !== 'serica://newtab' && activeTab.url !== 'neutrino://newtab' &&
    activeTab.url !== 'about:blank'
  );
  const [isHtmlFullscreen, setIsHtmlFullscreen] = useState(false);

  const shouldShowTopBar = !isHtmlFullscreen;

  // Dia-style top bar tint (R20). BrowserViewport reports the active page's theme-color and
  // sampled top color; reports for a tab that is no longer active are ignored. Internal pages,
  // HTML fullscreen and undeterminable colors keep the default theme (R20.5, R20.6, R21.12).
  const [pageColor, setPageColor] = useState<TabPageColor | null>(null);
  const activeTabIdRef = useRef<string | undefined>(activeTab?.id);
  activeTabIdRef.current = activeTab?.id;
  const handlePageColor = useCallback((tabId: string, report: PageColorReport) => {
    if (tabId !== activeTabIdRef.current) return;
    setPageColor({ tabId, ...report });
  }, []);
  const topBarTint = computeTopBarTint({
    activeTabId: activeTab?.id,
    activeTabUrl: activeTab?.url,
    activeTabIsSleeping: activeTab?.isSleeping,
    report: pageColor,
    suppressed: isHtmlFullscreen,
  });
  const isTopBarTinted = Boolean(topBarTint) && isWebpage;

  // Track OS dark/light preference for Spaces configured in 'auto' mode
  const [osPrefersDark, setOsPrefersDark] = useState<boolean>(() => getOsPrefersDark());
  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return;
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const onChange = (e: MediaQueryListEvent) => setOsPrefersDark(e.matches);
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  }, []);

  const activeSpace = state.spaces.find((s) => s.id === state.activeSpaceId) || state.spaces[0];
  const activeSpaceTheme = resolveSpaceTheme(settings, state.activeSpaceId);

  const themeTransitionTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const triggerThemeCrossFade = useCallback(() => {
    if (themeTransitionTimerRef.current !== null) {
      clearTimeout(themeTransitionTimerRef.current);
    }
    document.documentElement.classList.add('space-theme-transitioning');
    themeTransitionTimerRef.current = setTimeout(() => {
      themeTransitionTimerRef.current = null;
      document.documentElement.classList.remove('space-theme-transitioning');
    }, 520);
  }, []);

  useEffect(() => {
    return () => {
      if (themeTransitionTimerRef.current !== null) {
        clearTimeout(themeTransitionTimerRef.current);
      }
    };
  }, []);

  // Persistent Dark Theme state (kept in sync with active Space theme effective mode)
  const [isDarkMode, setIsDarkMode] = useState<boolean>(() => {
    try {
      const saved = (localStorage.getItem('serica_dark_theme') || localStorage.getItem('neutrino_dark_theme'));
      return saved === null ? true : saved === 'true';
    } catch {
      return true;
    }
  });

  const lastIpcThemeModeRef = useRef<string | null>(null);
  useEffect(() => {
    // Onboarding previews its theme choice on the root; leave it alone until it finishes.
    if (isWelcomeOpen) return;
    const palette = applySpaceThemeToRoot(activeSpaceTheme, osPrefersDark);
    const nextDark = palette.effectiveMode === 'dark';
    setIsDarkMode((prev) => (prev === nextDark ? prev : nextDark));
    try {
      localStorage.setItem('serica_dark_theme', nextDark ? 'true' : 'false');
    } catch { }

    const desiredIpcMode: 'dark' | 'light' | 'system' =
      activeSpaceTheme.mode === 'auto' ? 'system' : nextDark ? 'dark' : 'light';
    if (lastIpcThemeModeRef.current !== desiredIpcMode) {
      lastIpcThemeModeRef.current = desiredIpcMode;
      void window.serica?.setThemeMode?.(desiredIpcMode);
    }
  }, [activeSpaceTheme, osPrefersDark, isWelcomeOpen]);

  const handleChangeSpaceTheme = useCallback(
    (nextTheme: SpaceTheme, options?: { immediateSave?: boolean; animateTransition?: boolean }) => {
      if (options?.animateTransition) {
        triggerThemeCrossFade();
      }
      const palette = applySpaceThemeToRoot(nextTheme, osPrefersDark);
      if (options?.immediateSave !== false) {
        void updateSettings({
          theme: palette.effectiveMode,
          spaceThemes: {
            ...(settings.spaceThemes || {}),
            [state.activeSpaceId]: nextTheme,
          },
        });
      }
    },
    [osPrefersDark, settings.spaceThemes, state.activeSpaceId, triggerThemeCrossFade, updateSettings]
  );

  const handleResetSpaceTheme = useCallback(() => {
    triggerThemeCrossFade();
    const fallbackTheme = getDefaultSpaceTheme(state.activeSpaceId);
    const palette = applySpaceThemeToRoot(fallbackTheme, osPrefersDark);
    void updateSettings({
      theme: palette.effectiveMode,
      spaceThemes: {
        ...(settings.spaceThemes || {}),
        [state.activeSpaceId]: fallbackTheme,
      },
    });
  }, [osPrefersDark, settings.spaceThemes, state.activeSpaceId, triggerThemeCrossFade, updateSettings]);

  // Persist and apply a theme mode for the active Space; shared by top-bar toggle & Settings panel.
  const setDarkMode = useCallback(
    (next: boolean) => {
      triggerThemeCrossFade();
      const updatedTheme: SpaceTheme = {
        ...activeSpaceTheme,
        mode: next ? 'dark' : 'light',
      };
      applySpaceThemeToRoot(updatedTheme, osPrefersDark);
      setIsDarkMode(next);
      void window.serica?.setThemeMode?.(next ? 'dark' : 'light');
      void updateSettings({
        theme: next ? 'dark' : 'light',
        spaceThemes: {
          ...(settings.spaceThemes || {}),
          [state.activeSpaceId]: updatedTheme,
        },
      });
    },
    [activeSpaceTheme, osPrefersDark, settings.spaceThemes, state.activeSpaceId, triggerThemeCrossFade, updateSettings]
  );

  const toggleDarkMode = () => {
    setDarkMode(!isDarkMode);
  };

  // Reconcile cached main-process settings (`browser-state.json`) once loaded on startup
  const hydratedFromMainRef = useRef(false);
  useEffect(() => {
    if (!settingsLoaded || hydratedFromMainRef.current) return;
    hydratedFromMainRef.current = true;

    const migrationPatch: Record<string, unknown> = {};

    if (settings.theme !== 'dark' && settings.theme !== 'light') {
      migrationPatch.theme = isDarkMode ? 'dark' : 'light';
    }

    const persistedProfile = settings.userProfile as UserProfile | undefined;
    if (persistedProfile && typeof persistedProfile === 'object' && typeof persistedProfile.name === 'string') {
      setUserProfile(persistedProfile);
      setIsWelcomeOpen(false);
      try {
        localStorage.setItem('serica_user_profile', JSON.stringify(persistedProfile));
        localStorage.setItem('serica_onboarding_completed', 'true');
      } catch { }
    } else if (settings.onboardingCompleted === true) {
      setIsWelcomeOpen(false);
      try {
        localStorage.setItem('serica_onboarding_completed', 'true');
      } catch { }
    } else if (userProfile) {
      migrationPatch.userProfile = userProfile;
      migrationPatch.onboardingCompleted = true;
    }

    if (!settings.customTheme) {
      try {
        const savedTheme = (localStorage.getItem('serica_simple_theme_v5') || localStorage.getItem('neutrino_simple_theme_v5'));
        if (savedTheme) migrationPatch.customTheme = JSON.parse(savedTheme);
      } catch { }
    }

    if (!Array.isArray(settings.pinnedSites)) {
      try {
        const savedPins = (localStorage.getItem('serica_pinned_sites') || localStorage.getItem('neutrino_pinned_sites'));
        if (savedPins) migrationPatch.pinnedSites = JSON.parse(savedPins);
      } catch { }
    }

    if (Object.keys(migrationPatch).length > 0) {
      void updateSettings(migrationPatch);
    }
  }, [settingsLoaded, settings, isDarkMode, userProfile, updateSettings]);

  // Clean up any legacy zoom locks or invalid custom themes
  useEffect(() => {
    try {
      localStorage.removeItem('serica_browser_zoom'); localStorage.removeItem('neutrino_browser_zoom');
      localStorage.removeItem('serica_homescreen_scale'); localStorage.removeItem('neutrino_homescreen_scale');
      const validAccents = new Set(['#FFFFFF', '#1C1917', '#E4E4E7', '#71717A', '#38BDF8', '#10B981']);
      const savedTheme = (localStorage.getItem('serica_simple_theme_v5') || localStorage.getItem('neutrino_simple_theme_v5'));
      if (savedTheme) {
        try {
          const parsed = JSON.parse(savedTheme);
          if (parsed.accent && !validAccents.has(String(parsed.accent).toUpperCase())) {
            localStorage.removeItem('serica_simple_theme_v5'); localStorage.removeItem('neutrino_simple_theme_v5');
          }
        } catch {
          localStorage.removeItem('serica_simple_theme_v5'); localStorage.removeItem('neutrino_simple_theme_v5');
        }
      }
      document.documentElement.style.zoom = '';
    } catch { }
  }, []);

  // Persistent Media Player state (PiP when backgrounded)
  const [activeMediaState, setActiveMediaState] = useState<ActiveMediaState | null>(null);
  const [mediaCommand, setMediaCommand] = useState<{
    action: string;
    tabId?: string;
    value?: number;
    timestamp: number;
  } | null>(null);
  const dismissedMediaTabsRef = useRef<Set<string>>(new Set());

  // MCP Initial Tab state
  const [mcpInitialTab, setMcpInitialTab] = useState<'assistants' | 'local' | 'tools'>('assistants');

  // Media state change with dismissal memory to prevent unwanted popups
  const handleMediaStateChange = useCallback((newState: ActiveMediaState | null) => {
    if (newState && dismissedMediaTabsRef.current.has(newState.tabId)) {
      return;
    }
    setActiveMediaState(newState);
  }, []);

  // Space_Switcher order (profiles in creation order, then each profile's spaces), used by Ctrl+1..9.
  const spaceSwitcherOrder = useMemo(() => buildSidebarView(state).spaceOrder, [state]);
  const pageDialogTabIds = useMemo(() => state.tabs.map((t) => t.id), [state.tabs]);

  // Space switching is immediate: the UI state and the backend switch in the
  // same tick, so the sidebar, the top bar and the page never disagree about
  // which space is active. Only the theme colors cross-fade.
  const activeSpaceIdRef = useRef(state.activeSpaceId);
  activeSpaceIdRef.current = state.activeSpaceId;
  const handleSelectSpace = useCallback(
    (spaceId: string) => {
      if (spaceId === activeSpaceIdRef.current) return;
      activeSpaceIdRef.current = spaceId;
      triggerThemeCrossFade();
      void selectSpace(spaceId);
    },
    [selectSpace, triggerThemeCrossFade],
  );

  const handleCreateSpace = useCallback(
    async (name: string, color?: string) => {
      const created = await createSpace(name, color);
      if (created) triggerThemeCrossFade();
      return created;
    },
    [createSpace, triggerThemeCrossFade],
  );

  // Keyboard shortcuts & events
  useEffect(() => {
    const handleOpenCommandBar = () => openCommandBar();
    const handleCloseCommandBar = () => setIsCommandBarOpen(false);
    const handleToggleTabs = (e: any) => {
      const open = e?.detail?.open;
      if (typeof open === 'boolean') setSidebarExpanded(open);
      else toggleSidebar();
    };
    const handleCloseActiveTab = () => {
      if (state.activeTabId) {
        closeTab(state.activeTabId);
      }
    };

    // Native Electron IPC listeners (authoritative single-dispatch from main process)
    const unsubIpcToggle = window.serica?.onToggleTabs?.(() => {
      toggleSidebar();
    });
    // Ctrl+T arrives as `{ newTab: true }` (R12.8); Ctrl+K/L/F6 open the bar normally.
    const unsubIpcCmd = window.serica?.onOpenCommandBar?.((opts) => {
      if (opts?.newTab) openNewTabCommandBar();
      else openCommandBar();
    });
    const unsubIpcEsc = window.serica?.onEscapePressed?.(() => {
      setIsCommandBarOpen(false);
      setIsArchiveOpen(false);
      setIsSettingsOpen(false);
      setIsThemeModalOpen(false);
      setIsHistoryModalOpen(false);
      setIsExtensionsOpen(false);
      setIsDownloadsOpen(false);
      setIsVpnModalOpen(false);
    });
    // Ctrl+Comma while a webview has focus (main process before-input-event)
    const unsubIpcSettings = window.serica?.onOpenSettings?.(() => {
      setIsSettingsOpen(true);
    });
    const unsubIpcHistory = window.serica?.onOpenHistory?.(() => {
      setIsHistoryModalOpen(true);
    });
    const unsubIpcSpaceSwitched = window.serica?.onSpaceSwitched?.(() => {
      triggerThemeCrossFade();
    });
    // neutrino://settings, neutrino://history, neutrino://vpn from useBrowser.navigate
    const handleOpenSettingsEvent = () => setIsSettingsOpen(true);
    const handleOpenHistoryEvent = () => setIsHistoryModalOpen(true);
    const handleOpenVpnEvent = () => setIsVpnModalOpen(true);
    const handleOpenExtensionsEvent = () => setIsExtensionsOpen(true);
    window.addEventListener('serica:open-extensions', handleOpenExtensionsEvent);
    const handleOpenDownloadsEvent = () => setIsDownloadsOpen(true);
    // The sidebar guide needs the sidebar on screen.
    const handleGuideEvent = (e: Event) => {
      if ((e as CustomEvent).detail === 'sidebar') setSidebarExpanded(true);
    };
    window.addEventListener('serica:guide', handleGuideEvent);
    window.addEventListener('serica:open-downloads', handleOpenDownloadsEvent);
    const unsubIpcDownloads = window.serica?.onOpenDownloads?.(() => setIsDownloadsOpen(true));

    window.addEventListener('serica:open-settings', handleOpenSettingsEvent);
    window.addEventListener('serica:open-history', handleOpenHistoryEvent);
    window.addEventListener('serica:open-vpn', handleOpenVpnEvent);
    window.addEventListener('serica:open-command-bar', handleOpenCommandBar);
    window.addEventListener('serica:close-command-bar', handleCloseCommandBar);
    window.addEventListener('serica:toggle-tabs', handleToggleTabs);
    window.addEventListener('serica:close-active-tab', handleCloseActiveTab);

    // Fallback when running renderer standalone outside Electron (main process owns shortcuts in Electron)
    const handleKeyDown = (e: KeyboardEvent) => {
      if (window.serica?.isElectron) return;
      // Ctrl+1..9: the Nth space in Space_Switcher order; fewer than N keeps the current one (R13.4/13.5).
      if ((e.ctrlKey || e.metaKey) && e.key.length === 1 && e.key >= '1' && e.key <= '9') {
        e.preventDefault();
        const nextId = selectSpaceByIndex(spaceSwitcherOrder, Number(e.key), state.activeSpaceId);
        if (nextId !== state.activeSpaceId) {
          handleSelectSpace(nextId);
        }
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => {
      unsubIpcToggle?.();
      unsubIpcCmd?.();
      unsubIpcEsc?.();
      unsubIpcSettings?.();
      unsubIpcHistory?.();
      unsubIpcSpaceSwitched?.();
      window.removeEventListener('serica:open-settings', handleOpenSettingsEvent);
      window.removeEventListener('serica:open-history', handleOpenHistoryEvent);
      window.removeEventListener('serica:open-vpn', handleOpenVpnEvent);
      window.removeEventListener('serica:open-extensions', handleOpenExtensionsEvent);
      window.removeEventListener('serica:open-downloads', handleOpenDownloadsEvent);
      window.removeEventListener('serica:guide', handleGuideEvent);
      unsubIpcDownloads?.();
      window.removeEventListener('keydown', handleKeyDown);
      window.removeEventListener('serica:open-command-bar', handleOpenCommandBar);
      window.removeEventListener('serica:close-command-bar', handleCloseCommandBar);
      window.removeEventListener('serica:toggle-tabs', handleToggleTabs);
      window.removeEventListener('serica:close-active-tab', handleCloseActiveTab);
    };
}, [state.activeTabId, state.activeSpaceId, spaceSwitcherOrder, activeTab, closeTab, handleSelectSpace, setIsCommandBarOpen, triggerThemeCrossFade, toggleSidebar, setSidebarExpanded, openCommandBar, openNewTabCommandBar]);

// Built-in VPN state hydration & lazy History hydration (only fetch 500 history entries when open)
useEffect(() => {
  void window.serica?.getVpnStatus?.().then((status) => {
    if (status) setVpnStatus(status);
  });
  const unsubVpn = window.serica?.onVpnStatusUpdated?.((status) => {
    if (status) setVpnStatus(status);
  });

  return () => {
    unsubVpn?.();
  };
}, []);

useEffect(() => {
  if (!isHistoryModalOpen) {
    setHistoryEntries((prev) => (prev.length > 0 ? [] : prev));
    return;
  }
  void window.serica?.getHistory?.({ limit: 500 }).then((list) => {
    if (Array.isArray(list)) setHistoryEntries(list);
  });
  // The backend pings on changes; refetch, at most a few times a second.
  let timer: ReturnType<typeof setTimeout> | null = null;
  const unsubHistory = window.serica?.onHistoryUpdated?.(() => {
    if (timer) return;
    timer = setTimeout(() => {
      timer = null;
      void window.serica?.getHistory?.({ limit: 500 }).then((list) => {
        if (Array.isArray(list)) setHistoryEntries(list);
      });
    }, 250);
  });
  return () => {
    if (timer) clearTimeout(timer);
    unsubHistory?.();
  };
}, [isHistoryModalOpen]);

useEffect(() => {
  if (!isVpnModalOpen) return;
  void window.serica?.getVpnStatus?.().then((status) => {
    if (status) setVpnStatus(status);
  });
}, [isVpnModalOpen]);

const handleUpdateTab = (tabId: string, partial: Partial<TabState>) => {
  window.serica?.updateTab?.(tabId, partial);
};

// Media Controls
const handleMediaPlayPause = () => {
  if (!activeMediaState) return;
  const action = activeMediaState.isPlaying ? 'pause' : 'play';
  setMediaCommand({ action, tabId: activeMediaState.tabId, timestamp: Date.now() });
  setActiveMediaState((prev) => (prev ? { ...prev, isPlaying: !prev.isPlaying } : null));
};

const handleMediaSeek = (time: number) => {
  if (!activeMediaState) return;
  setMediaCommand({ action: 'seek', tabId: activeMediaState.tabId, value: time, timestamp: Date.now() });
  setActiveMediaState((prev) => (prev ? { ...prev, currentTime: time } : null));
};

const handleMediaVolume = (vol: number) => {
  if (!activeMediaState) return;
  setMediaCommand({ action: 'volume', tabId: activeMediaState.tabId, value: vol, timestamp: Date.now() });
  setActiveMediaState((prev) => (prev ? { ...prev, volume: vol } : null));
};

const handleGoToMediaTab = (tabId: string) => {
  selectTab(tabId);
};

const handleCloseMediaPlayer = () => {
  if (activeMediaState) {
    dismissedMediaTabsRef.current.add(activeMediaState.tabId);
    try {
      setMediaCommand({ action: 'pause', tabId: activeMediaState.tabId, timestamp: Date.now() });
    } catch { }
  }
  setActiveMediaState(null);
};

const dispatchSiteAction = useCallback((action: SiteAction) => {
  window.dispatchEvent(new CustomEvent('neutrino:site-action', { detail: { action, source: 'ui' } }));
}, []);

// Fallback Capturing Keyboard Shortcuts (only outside Electron where before-input-event is unavailable)
useEffect(() => {
  const handleGlobalKeyDown = (e: KeyboardEvent) => {
    if (window.serica?.isElectron) return;
    if (e.key === 'Escape' && !e.defaultPrevented) {
      // Close the topmost panel; the command bar handles its own Escape.
      setIsArchiveOpen(false);
      setIsSettingsOpen(false);
      setIsThemeModalOpen(false);
      setIsHistoryModalOpen(false);
      setIsExtensionsOpen(false);
      setIsDownloadsOpen(false);
      setIsVpnModalOpen(false);
      setIsShieldModalOpen(false);
      setIsMcpModalOpen(false);
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      const key = e.key.toLowerCase();
      if (key === 'w') {
        e.preventDefault();
        e.stopPropagation();
        e.stopImmediatePropagation();
        if (state.activeTabId) {
          closeTab(state.activeTabId);
        }
      } else if (key === 't' && e.shiftKey) {
        e.preventDefault();
        e.stopPropagation();
        void reopenClosedTab();
      } else if (key === 't') {
        e.preventDefault();
        e.stopPropagation();
        e.stopImmediatePropagation();
        openNewTabCommandBar();
      } else if (key === 'k' || key === 'l') {
        e.preventDefault();
        e.stopPropagation();
        e.stopImmediatePropagation();
        setIsCommandBarOpen((prev) => !prev);
      } else if (key === 'e' || key === 's') {
        e.preventDefault();
        e.stopPropagation();
        e.stopImmediatePropagation();
        toggleSidebar();
      } else if (key === 'r') {
        e.preventDefault();
        e.stopPropagation();
        e.stopImmediatePropagation();
        dispatchSiteAction(e.shiftKey ? 'hard-reload' : 'reload');
      } else if (key === ',') {
        e.preventDefault();
        e.stopPropagation();
        e.stopImmediatePropagation();
        setIsCommandBarOpen(false);
        setIsSettingsOpen(true);
      } else if (key === 'o' && !e.shiftKey) {
        e.preventDefault();
        e.stopPropagation();
        e.stopImmediatePropagation();
        setIsCommandBarOpen(false);
        void window.serica?.openFile?.().catch(() => {});
      } else if (key === 'j') {
        e.preventDefault();
        e.stopPropagation();
        e.stopImmediatePropagation();
        setIsCommandBarOpen(false);
        setIsDownloadsOpen((prev) => !prev);
      } else if (key === 'h') {
        e.preventDefault();
        e.stopPropagation();
        e.stopImmediatePropagation();
        setIsCommandBarOpen(false);
        setIsHistoryModalOpen((prev) => !prev);
      } else if (key === 'f') {
        e.preventDefault();
        dispatchSiteAction('open-find');
      } else if (key === 'tab' || key === 'pagedown' || key === 'pageup') {
        e.preventDefault();
        const back = (key === 'tab' && e.shiftKey) || key === 'pageup';
        void window.serica?.cycleTab?.(back ? -1 : 1);
      } else if (key === '=' || key === '+') {
        e.preventDefault();
        window.dispatchEvent(new CustomEvent('neutrino:zoom-action', { detail: 'in' }));
      } else if (key === '-') {
        e.preventDefault();
        window.dispatchEvent(new CustomEvent('neutrino:zoom-action', { detail: 'out' }));
      } else if (key === '0') {
        e.preventDefault();
        window.dispatchEvent(new CustomEvent('neutrino:zoom-action', { detail: 'reset' }));
      }
    } else if (e.altKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
      const target = e.target as HTMLElement | null;
      if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable)) return;
      e.preventDefault();
      dispatchSiteAction(e.key === 'ArrowLeft' ? 'back' : 'forward');
    } else if (e.key === 'F5') {
      e.preventDefault();
      dispatchSiteAction(e.ctrlKey || e.shiftKey ? 'hard-reload' : 'reload');
    } else if (e.key === 'F12') {
      e.preventDefault();
      void window.serica?.openDevtools?.();
    }
  };

  window.addEventListener('keydown', handleGlobalKeyDown, true);
  return () => window.removeEventListener('keydown', handleGlobalKeyDown, true);
}, [state.activeTabId, closeTab, openNewTabCommandBar, toggleSidebar, reopenClosedTab, dispatchSiteAction]);


const shieldsBlockedCount =
  state.shieldStats.totalBlocked || (state.shieldStats.trackersBlocked + state.shieldStats.adsBlocked);

const handleOpenHistoryUrl = useCallback(
  (url: string, options?: { newTab?: boolean }) => {
    if (options?.newTab || !activeTab) {
      void createTab(url, state.activeSpaceId);
    } else {
      navigate(url);
    }
  },
  [activeTab, createTab, navigate, state.activeSpaceId],
);

const handleDeleteHistoryEntry = useCallback(async (entry: HistoryEntry) => {
  await window.serica?.deleteHistoryEntry?.({
    id: entry.id,
    url: entry.url,
    timestamp: entry.timestamp,
  });
}, []);

const handleClearHistory = useCallback(async (range: HistoryClearRange) => {
  await window.serica?.clearHistory?.(range);
}, []);

const handleVpnConnect = useCallback(async (patch?: Partial<VpnConfig>) => {
  const next = await window.serica?.connectVpn?.(patch);
  if (next) setVpnStatus(next);
}, []);

const handleVpnDisconnect = useCallback(async () => {
  const next = await window.serica?.disconnectVpn?.();
  if (next) setVpnStatus(next);
}, []);

const handleVpnUpdateConfig = useCallback(async (patch: Partial<VpnConfig>) => {
  const next = await window.serica?.updateVpnConfig?.(patch);
  if (next) setVpnStatus(next);
}, []);

const handleVpnCheckIp = useCallback(async () => {
  setIsCheckingVpnIp(true);
  setVpnIpError(null);
  try {
    const res = await window.serica?.checkVpnIp?.();
    if (res && 'error' in res) {
      setVpnIpError(res.error);
    }
  } finally {
    setIsCheckingVpnIp(false);
  }
}, []);

// The controls panel stands in for the video; hide it while the video has its own PiP window.
const shouldShowMiniPlayer =
  activeMediaState &&
  activeMediaState.isPlaying &&
  !activeMediaState.inPip &&
  activeMediaState.tabId !== state.activeTabId;

// Command_Bar submission: in new-tab mode (Ctrl+T / sidebar new tab control) a URL or search
// opens a Today tab in the Active_Space (R12.8); serica:// panel URLs still open their panel.
const handleCommandBarNavigate = (url: string) => {
  if (isCommandBarNewTab && !/^(?:serica|neutrino):\/\/(settings|history|vpn)$/i.test(url.trim())) {
    void createTab(url, state.activeSpaceId);
    return;
  }
  navigate(url);
};

// Assistant drawer webviews follow the Active_Profile's session (R3.3, A3). The active tab's
// partition comes with every state-update; listProfiles covers a space with no tab yet.
const activeTabInSpace = activeTab && activeTab.spaceId === state.activeSpaceId ? activeTab : undefined;
const activeProfilePartition = activeTabInSpace?.partition ?? activeProfile?.partition;

// Tab layout: Arc-style sidebar (default) or a horizontal strip under the top bar.
const topTabs = settings.tabLayout === 'top';
const showTopTabs = topTabs && !isHtmlFullscreen;
// Sidebar placement: docked (not auto-hide) pushes the web canvas right; hidden in HTML fullscreen.
const showSidebar = !isHtmlFullscreen && !topTabs;
// The top bar on web pages: 30px, and (by default) hidden until the pointer
// reaches the top edge. The tab strip of top-tabs mode keeps it shown.
const TOP_TAB_STRIP_HEIGHT = 38;
const topBarAutoHideOn = isWebpage && !showTopTabs && !isHtmlFullscreen && settings.topBarAutoHide === true;
const overlayCount = useViewportOverlayCount();
const topBar = useTopBarAutoHide(topBarAutoHideOn, toolsOpen || overlayCount > 0 || sidebarShown);
// Where web pages start: below the top bar, plus the tab strip in top-tabs mode.
const contentTop = isHtmlFullscreen
  ? 0
  : showTopTabs
    ? TOP_BAR_HEIGHT + TOP_TAB_STRIP_HEIGHT
    : topBarAutoHideOn
      ? TOP_BAR_REVEAL_STRIP
      : TOP_BAR_HEIGHT;

const [sidebarWidth, setSidebarWidth] = useState<number>(() => {
  const saved = (settings as any).sidebarWidth;
  return typeof saved === 'number' && saved >= 160 && saved <= 520 ? saved : 240;
});

useEffect(() => {
  const saved = (settings as any).sidebarWidth;
  if (typeof saved === 'number' && saved >= 160 && saved <= 520 && saved !== sidebarWidth) {
    setSidebarWidth(saved);
  }
}, [(settings as any).sidebarWidth]);

// Sidebar control absorption (chrome.css): how far the site buttons glide
// left when the space pill folds to its icon, and whether the new-tab button
// still fits on a narrow sidebar. Measured only when the pill's size changes.
const clusterRef = useRef<HTMLElement>(null);
const [absorbCompact, setAbsorbCompact] = useState(false);
useLayoutEffect(() => {
  const cluster = clusterRef.current;
  const pill = cluster?.querySelector<HTMLElement>('.macos-tabs-toggle-pill');
  if (!cluster || !pill) return;
  const measure = () => {
    const shift = Math.max(0, pill.offsetWidth - 28);
    cluster.style.setProperty('--pill-shift', `${shift}px`);
    const left = cluster.offsetLeft + (cluster.offsetParent as HTMLElement | null ?? cluster).offsetLeft;
    setAbsorbCompact(left + cluster.scrollWidth - shift + 6 > sidebarWidth);
  };
  measure();
  const ro = new ResizeObserver(measure);
  ro.observe(pill);
  return () => ro.disconnect();
}, [sidebarWidth, topTabs]);

// Native page views paint above HTML; freeze the page while any of these cover it.
useViewportOverlay(
  isCommandBarOpen ||
    isShieldModalOpen ||
    isMcpModalOpen ||
    isThemeModalOpen ||
    isSettingsOpen ||
    isHistoryModalOpen ||
    isExtensionsOpen ||
    isDownloadsOpen ||
    isVpnModalOpen ||
    isArchiveOpen ||
    isWelcomeOpen,
);

// Browser tools, run from the top bar's menu (when pinned) or from Settings.
const toolActions: ToolActions = {
  history: () => setIsHistoryModalOpen(true),
  downloads: () => setIsDownloadsOpen(true),
  'open-file': () => void window.serica?.openFile?.().catch(() => {}),
  ai: () => {
    setMcpInitialTab('assistants');
    setIsMcpModalOpen(true);
  },
  vpn: () => setIsVpnModalOpen(true),
  theme: () => setIsThemeModalOpen(true),
};

const shellStyle = {
  '--topbar-h': `${TOP_BAR_HEIGHT}px`,
  // Where the page starts on web pages: below the top bar, plus the tab strip in top-tabs mode.
  '--app-content-top': `${contentTop}px`,
} as React.CSSProperties;

return (
  <div className={`arc-app-shell radical-minimalism-shell${showTopTabs ? ' top-tabs-mode' : ''}`} style={shellStyle}>
    <div className="space-grain-overlay" aria-hidden="true" />
    {/* Frameless Top Drag Bar (only where the top bar is on screen) */}
    {!isHtmlFullscreen && topBar.shown && (
      <div className={`macos-window-drag-bar ${isWebpage ? 'on-webpage' : ''}`} />
    )}
    {/* Pointer at the top edge reveals the auto-hidden top bar. */}
    {topBarAutoHideOn && !topBar.shown && (
      <div className="topbar-reveal-strip" onPointerEnter={topBar.revealNow} aria-hidden="true" />
    )}

    {showTopTabs && (
      <TopTabStrip
        state={state}
        onWebpage={isWebpage}
        tint={isWebpage && isTopBarTinted && topBarTint ? (topBarTint.vars as React.CSSProperties) : undefined}
        onSelectTab={(id) => browser.selectTab(id)}
        onCloseTab={(id) => closeTab(id)}
        onSelectSpace={handleSelectSpace}
        onNewTab={openNewTabCommandBar}
        actions={browser}
      />
    )}

    {/* Arc-style Sidebar: favorites, pinned, Today and Space_Switcher (R12.1) */}
    {showSidebar && (
      <Sidebar
        className="app-sidebar"
        state={state}
        actions={{ ...browser, selectSpace: handleSelectSpace, createSpace: handleCreateSpace, openCommandBar: openNewTabCommandBar }}
        expanded
        autoHide
        pinnedOpen={sidebarOpen}
        onRequestClose={closeSidebar}
        livePage={isWebpage && !activeTab?.unloaded}
        sidebarWidth={sidebarWidth}
        onUpdateSidebarWidth={setSidebarWidth}
        onUpdateSidebarState={(patch) => void updateSettings(patch)}
        onShownChange={setSidebarShown}
        scale={settings.uiScale?.sidebar ?? 1}
      />
    )}

    {/* Top Chrome Header Bar (Solid dedicated header on web pages, floating on newtab) */}
    <div
      className={`floating-top-chrome-capsule ${isWebpage ? 'on-webpage' : ''} ${isTopBarTinted ? 'topbar-tinted' : ''} ${shouldShowTopBar ? 'visible' : 'autohidden'}${toolsOpen ? ' tools-open' : ''} topbar-style-${settings.topBarStyle === 'pill' ? 'pill' : 'flat'}${topBarAutoHideOn ? ` topbar-autohide${topBar.shown ? ' topbar-shown' : ''}` : ''}`}
      style={isTopBarTinted && topBarTint ? (topBarTint.vars as React.CSSProperties) : undefined}
      data-topbar-tint={isTopBarTinted && topBarTint ? topBarTint.source : 'default'}
    >
      {/* Top left: sidebar toggle, site navigation and new tab */}
      <aside ref={clusterRef} className={`macos-window-controls-cluster${absorbCompact ? ' absorb-compact' : ''}`} aria-label="Navigation">
        {!topTabs && (
        <button
          className={`macos-tabs-toggle-pill ${sidebarOpen ? 'active' : ''}`}
          data-sidebar-toggle
          onPointerEnter={isWebpage ? prewarmPageUnderlay : undefined}
          onClick={toggleSidebar}
          title={sidebarOpen ? 'Hide sidebar (Ctrl+S)' : 'Show sidebar (Ctrl+S)'}
          aria-label="Toggle sidebar"
          aria-pressed={sidebarOpen}
        >
          <PanelLeft size={14} strokeWidth={2} aria-hidden="true" />
          <span
            className="macos-tabs-space-dot"
            style={{ backgroundColor: activeSpace?.color || '#E4E4E7' }}
          />
          <span className="macos-tabs-space-name" data-testid="active-space-indicator">
            {activeSpace?.name || 'Study'}
          </span>
          <span className="macos-tabs-count" data-testid="active-space-tab-count">
            {currentSpaceTabs.length}
          </span>
        </button>
        )}

        {/* Compact Site Navigation Controls */}
        <div className="site-nav-controls" role="group" aria-label="Site controls">
          <button
            type="button"
            className="site-nav-btn"
            onClick={() => dispatchSiteAction('back')}
            disabled={!isWebpage || !activeTab?.canGoBack}
            title="Back (Alt+Left / Two-finger Swipe Left)"
            aria-label="Go Back"
          >
            <ArrowLeft size={14} strokeWidth={2.1} />
          </button>
          <button
            type="button"
            className="site-nav-btn"
            onClick={() => dispatchSiteAction('forward')}
            disabled={!isWebpage || !activeTab?.canGoForward}
            title="Forward (Alt+Right / Two-finger Swipe Right)"
            aria-label="Go Forward"
          >
            <ArrowRight size={14} strokeWidth={2.1} />
          </button>
          <button
            type="button"
            className="site-nav-btn"
            onClick={() => dispatchSiteAction(activeTab?.isLoading ? 'stop' : 'reload')}
            disabled={!isWebpage}
            title={activeTab?.isLoading ? 'Stop Loading (Esc)' : 'Reload Page (Ctrl+R)'}
            aria-label={activeTab?.isLoading ? 'Stop Loading' : 'Reload Page'}
          >
            {activeTab?.isLoading ? (
              <X size={14} strokeWidth={2.2} />
            ) : (
              <RotateCw size={13.5} strokeWidth={2.1} />
            )}
          </button>
          <button
            type="button"
            className="site-nav-btn"
            onClick={() => dispatchSiteAction('home')}
            disabled={!isWebpage}
            title="Homescreen (Alt+Home)"
            aria-label="Home"
          >
            <Home size={13.5} strokeWidth={2.1} />
          </button>
        </div>
        <button
          type="button"
          className="minimal-icon-btn topbar-new-tab-btn"
          onClick={openNewTabCommandBar}
          title="New tab (Ctrl+T)"
          aria-label="New tab"
        >
          <Plus size={16} />
        </button>
      </aside>

      {/* Top Center Floating URL Pill */}
      <FloatingUrlPill
        url={activeTab?.url}
        isLoading={activeTab?.isLoading}
        zoomFactor={activeTab?.zoomFactor}
        defaultZoom={settings.textScale}
        onClick={() => setIsCommandBarOpen(true)}
        onResetZoom={() => window.dispatchEvent(new CustomEvent('neutrino:zoom-action', { detail: 'reset' }))}
        onOpenFind={() => dispatchSiteAction('open-find')}
        tabId={activeTab?.id}
        pdfDarkKey={`${isDarkMode}-${settings.pdfDarkMode}`}
      />

      {/* Top right: status pills and the browser menu, then the window controls */}
      <div className="minimal-top-right-cluster">
        {vpnStatus?.state === 'connected' && (
          <button
            className="minimal-pill-btn vpn-top-pill vpn-connected"
            onClick={() => setIsVpnModalOpen(!isVpnModalOpen)}
            title={`VPN Tunnel Active (${vpnStatus.activeProxyRule || vpnStatus.config.mode})`}
            aria-label="Built-in VPN"
          >
            <Lock size={13} className="modern-btn-icon" />
            <span>VPN ON</span>
          </button>
        )}

        {(() => {
          const activeHost = (() => {
            try {
              const parsed = activeTab?.url ? new URL(activeTab.url) : null;
              return parsed && (parsed.protocol === 'http:' || parsed.protocol === 'https:')
                ? parsed.hostname.toLowerCase()
                : '';
            } catch {
              return '';
            }
          })();
          const isSiteWhitelisted =
            !!activeHost &&
            (settings.shieldWhitelist ?? []).some(
              (allowed) => activeHost === allowed || activeHost.endsWith(`.${allowed}`),
            );
          return (
            <button
              className="minimal-pill-btn"
              onClick={() => setIsShieldModalOpen(!isShieldModalOpen)}
              title={
                !settings.adblockEnabled
                  ? `Shields off: ${shieldsBlockedCount} blocked`
                  : isSiteWhitelisted
                    ? `Shields paused on ${activeHost}`
                    : `Shields active: ${shieldsBlockedCount} blocked`
              }
              aria-label="Shields"
              style={
                !settings.adblockEnabled
                  ? { opacity: 0.55 }
                  : isSiteWhitelisted
                    ? { color: '#f59e0b' }
                    : undefined
              }
            >
              <Shield size={14} className="modern-btn-icon" />
              <span>{shieldsBlockedCount}</span>
            </button>
          );
        })()}

        {downloads.length > 0 && (
          <button
            className={`minimal-icon-btn downloads-top-btn${activeDownloads.length > 0 ? ' active' : ''}`}
            onClick={() => setIsDownloadsOpen((prev) => !prev)}
            title={
              activeDownloads.length > 0
                ? `Downloads: ${activeDownloads.length} in progress${downloadProgress !== null ? ` (${Math.round(downloadProgress * 100)}%)` : ''} (Ctrl+J)`
                : 'Downloads (Ctrl+J)'
            }
            aria-label="Downloads"
          >
            {activeDownloads.length > 0 && (
              <svg className="downloads-ring" viewBox="0 0 24 24" aria-hidden="true">
                <circle className="downloads-ring-track" cx="12" cy="12" r="10.5" />
                <circle
                  className={`downloads-ring-fill${downloadProgress === null ? ' indeterminate' : ''}`}
                  cx="12"
                  cy="12"
                  r="10.5"
                  pathLength={100}
                  strokeDasharray={downloadProgress === null ? '25 75' : `${Math.max(2, downloadProgress * 100)} 100`}
                />
              </svg>
            )}
            <Download size={14} />
          </button>
        )}

        <ToolsMenu
          isDarkMode={isDarkMode}
          vpnOn={vpnStatus?.state === 'connected'}
          pinned={normalizePinnedTools(settings.pinnedTools)}
          actions={toolActions}
          onToggleDarkMode={toggleDarkMode}
          onSettings={openSettings}
          onOpenChange={setToolsOpen}
        />

        <div className="window-controls-divider" aria-hidden="true" />
        <div className="macos-traffic-lights window-controls-right" aria-label="Window controls">
          <button
            className="traffic-light minimize"
            onClick={() => window.serica?.windowControl('minimize')}
            title="Minimize"
            aria-label="Minimize"
          >
            <span className="traffic-glyph">−</span>
          </button>
          <button
            className="traffic-light maximize"
            onClick={() => window.serica?.windowControl('maximize')}
            title="Maximize"
            aria-label="Maximize"
          >
            <span className="traffic-glyph">+</span>
          </button>
          <button
            className="traffic-light close"
            onClick={() => window.serica?.windowControl('close')}
            title="Close"
            aria-label="Close window"
          >
            <span className="traffic-glyph">✕</span>
          </button>
        </div>
      </div>
    </div>

    {/* Edge-to-Edge Web Canvas Viewport (pulled down on web pages so site controls never collide;
        pushed right of a docked sidebar) */}
    <main className={`arc-viewport-container edge-to-edge ${isWebpage && !isHtmlFullscreen ? 'with-site-top-offset' : ''}`}>
      <div className="arc-web-canvas-frame seamless">
        <BrowserViewport
          activeTab={activeTab}
          tabs={state.tabs}
          spaces={state.spaces}
          activeSpaceId={state.activeSpaceId}
          isDarkMode={isDarkMode}
          isSidebarOpen={false}
          onSelectSpace={handleSelectSpace}
          onUpdateTab={handleUpdateTab}
          onNavigate={navigate}
          onOpenCommandBar={() => setIsCommandBarOpen(true)}
          onMediaStateChange={handleMediaStateChange}
          mediaCommand={mediaCommand}
          userName={userProfile?.name}
          onFullscreenChange={setIsHtmlFullscreen}
          onPageColor={handlePageColor}
          defaultZoom={settings.textScale}
          contentTop={contentTop}
          insetLeft={showSidebar ? 8 : 0}
          insetRight={settings.rightPinsDockVisible && isWebpage && !isHtmlFullscreen ? 6 : 0}
          homeWidgets={settings.homeWidgets}
          uiScale={settings.uiScale}
          onUpdateHomeWidgets={(homeWidgets) => void updateSettings({ homeWidgets })}
        />
      </div>
    </main>

    {/* Google sign-in refused in-app: offer to continue in the default browser */}
    <AuthHandoffNotice />

    {/* A downloaded update is ready: offer to restart into it */}
    <UpdateNotice />
    <AppContextMenu />
    <PageContextMenu />
    <WhatsNew onboarded={!isWelcomeOpen} />

    {/* Save/update password prompts from login form submissions (R6.2, R6.3, R7.2) */}
    <PasswordPrompt />

    {/* Page alert/confirm/prompt, "Leave site?" and "Open in app?" in Serica's style */}
    <PageDialog activeTabId={state.activeTabId} tabIds={pageDialogTabIds} />

    {/* Camera, microphone, location... asked in Serica's own prompt */}
    <PermissionPrompt activeTabId={state.activeTabId} tabIds={pageDialogTabIds} />

    {/* Files, links and text dropped on Serica's own interface */}
    <DropZones
      onNewTabPage={!isWebpage}
      spaceName={activeSpace?.name}
      onOpenInNewTab={(url) => void createTab(url, state.activeSpaceId)}
      onOpenHere={(url) => navigate(url)}
    />

    {/* Vertical Pinned Websites Dock (Right Side with macOS Magnification) */}
    {/* Hidden by the Settings toggle; stored pinnedSites stay untouched (R19.2, R19.3) */}
    {settings.rightPinsDockVisible && (
      <RightPinsDock
        onNavigate={(url) => void createTab(url, state.activeSpaceId)}
        isOnWebpage={isWebpage}
        persistedPins={Array.isArray(settings.pinnedSites) ? (settings.pinnedSites as PinnedSite[]) : undefined}
        onPersistPins={(pins) => void updateSettings({ pinnedSites: pins })}
        onHideDock={() => void updateSettings({ rightPinsDockVisible: false })}
        dockScale={settings.uiScale?.dock ?? 1}
      />
    )}

    {/* Floating Media Player (PiP) */}
    {shouldShowMiniPlayer && (
      <Suspense fallback={null}>
        <MediaPlayer
          mediaState={activeMediaState}
          onPlayPause={handleMediaPlayPause}
          onSeek={handleMediaSeek}
          onVolume={handleMediaVolume}
          onGoToTab={handleGoToMediaTab}
          onClose={handleCloseMediaPlayer}
          onPictureInPicture={() =>
            setMediaCommand({ action: 'pip', tabId: activeMediaState!.tabId, timestamp: Date.now() })
          }
          dockOverPage={isWebpage}
        />
      </Suspense>
    )}

    {/* Centered Floating Command Bar (Ctrl+T / Ctrl+K / Ctrl+L) */}
    {isCommandBarOpen && (
      <Suspense fallback={null}>
        <CommandBar
          isOpen={isCommandBarOpen}
          tabs={state.tabs}
          spaces={state.spaces}
          onClose={() => setIsCommandBarOpen(false)}
          onNavigate={handleCommandBarNavigate}
          onSelectTab={selectTab}
          onCreateTab={(url) => void createTab(url, state.activeSpaceId)}
          onToggleShields={toggleShields}
          onSelectSpace={handleSelectSpace}
          onToggleTheme={toggleDarkMode}
          onOpenMcp={() => setIsMcpModalOpen(true)}
          onOpenTheme={() => setIsThemeModalOpen(true)}
          onOpenShields={() => setIsShieldModalOpen(true)}
          searchEngine={settings.searchEngine}
          onOpenSettings={openSettings}
          onOpenHistory={() => setIsHistoryModalOpen(true)}
          onOpenVpn={() => setIsVpnModalOpen(true)}
          onGoBack={() => dispatchSiteAction('back')}
          onGoForward={() => dispatchSiteAction('forward')}
          onReload={() => dispatchSiteAction('reload')}
          onCloseActiveTab={() => {
            if (activeTab?.id) closeTab(activeTab.id);
          }}
          onReopenClosedTab={() => void reopenClosedTab()}
          onOpenFind={() => dispatchSiteAction('open-find')}
          onOpenArchive={() => setIsArchiveOpen(true)}
        />
      </Suspense>
    )}

    {/* Archived Today tabs of the Active_Profile (R16.7, R16.8); opened from the Command_Bar */}
    {isArchiveOpen && (
      <Suspense fallback={null}>
        <ArchiveList isOpen={isArchiveOpen} onClose={() => setIsArchiveOpen(false)} />
      </Suspense>
    )}

    {/* Built-in VPN / Privacy Tunnel Modal */}
    {isVpnModalOpen && (
      <Suspense fallback={null}>
        <VpnModal
          isOpen={isVpnModalOpen}
          onClose={() => setIsVpnModalOpen(false)}
          status={vpnStatus}
          onConnect={handleVpnConnect}
          onDisconnect={handleVpnDisconnect}
          onUpdateConfig={handleVpnUpdateConfig}
          onCheckIp={handleVpnCheckIp}
          isCheckingIp={isCheckingVpnIp}
          ipCheckError={vpnIpError}
        />
      </Suspense>
    )}

    {/* Browsing History Modal */}
    {isHistoryModalOpen && (
      <Suspense fallback={null}>
        <HistoryModal
          isOpen={isHistoryModalOpen}
          onClose={() => setIsHistoryModalOpen(false)}
          entries={historyEntries}
          onOpenUrl={handleOpenHistoryUrl}
          onDeleteEntry={handleDeleteHistoryEntry}
          onClearHistory={handleClearHistory}
        />
      </Suspense>
    )}

    {/* Downloads (top bar / Ctrl+J / neutrino://downloads) */}
    {isDownloadsOpen && (
      <Suspense fallback={null}>
        <DownloadsPanel isOpen={isDownloadsOpen} onClose={() => setIsDownloadsOpen(false)} downloads={downloads} />
      </Suspense>
    )}

    {/* Extensions (neutrino://extensions / Command Bar) */}
    {isExtensionsOpen && (
      <Suspense fallback={null}>
        <ExtensionsModal
          isOpen={isExtensionsOpen}
          onClose={() => setIsExtensionsOpen(false)}
          onOpenUrl={(url) => {
            setIsExtensionsOpen(false);
            void createTab(url, state.activeSpaceId);
          }}
        />
      </Suspense>
    )}

    {/* Shield Modal Flyout */}
    {isShieldModalOpen && (
      <Suspense fallback={null}>
        <ShieldModal
          isOpen={isShieldModalOpen}
          onClose={() => setIsShieldModalOpen(false)}
          stats={state.shieldStats}
          blockedItems={blockedTrackers}
          onToggleShields={toggleShields}
          enabled={settings.adblockEnabled}
          activeUrl={activeTab?.url}
          whitelist={settings.shieldWhitelist ?? []}
          onToggleSiteWhitelist={(domainOrUrl) => {
            void window.serica?.toggleSiteWhitelist?.(domainOrUrl);
          }}
          onResetStats={resetShieldStats}
        />
      </Suspense>
    )}

    {/* Browser Settings Panel (gear / Ctrl+, / Command Bar / neutrino://settings) */}
    {isSettingsOpen && (
      <Suspense fallback={null}>
        <SettingsPanel
          isOpen={isSettingsOpen}
          onClose={() => setIsSettingsOpen(false)}
          settings={settings}
          onUpdateSettings={updateSettings}
          isDarkMode={isDarkMode}
          onSetDarkMode={setDarkMode}
          onOpenThemeStudio={() => setIsThemeModalOpen(true)}
          userName={userProfile?.name ?? ''}
          onUserNameChange={handleUserNameChange}
          tabs={state.tabs}
          toolActions={toolActions}
        />
      </Suspense>
    )}

    {/* MCP Drawer (Right-Side) */}
    {isMcpModalOpen && (
      <Suspense fallback={null}>
        <McpModal
          isOpen={isMcpModalOpen}
          onClose={() => setIsMcpModalOpen(false)}
          activeTab={activeTab}
          onNavigate={navigate}
          initialTab={mcpInitialTab}
          partition={activeProfilePartition}
        />
      </Suspense>
    )}

    {/* Theme Studio Popover */}
    {isThemeModalOpen && (
      <Suspense fallback={null}>
        <ThemeCustomizerModal
          isOpen={isThemeModalOpen}
          onClose={() => setIsThemeModalOpen(false)}
          activeSpace={activeSpace}
          spaceTheme={activeSpaceTheme}
          customPresets={settings.customThemePresets || []}
          onChangeCustomPresets={(nextPresets) => void updateSettings({ customThemePresets: nextPresets })}
          onChangeSpaceTheme={handleChangeSpaceTheme}
          onResetSpaceTheme={handleResetSpaceTheme}
          isDarkMode={isDarkMode}
          onSetDarkMode={setDarkMode}
          persistedCustomTheme={
            settings.customTheme && typeof settings.customTheme === 'object'
              ? (settings.customTheme as { themeId: string; accent: string })
              : undefined
          }
          onPersistCustomTheme={(customTheme) => void updateSettings({ customTheme })}
        />
      </Suspense>
    )}

    {/* Welcome / Onboarding Flow for First Launch */}
    {isWelcomeOpen && (
      <Suspense fallback={null}>
        <WelcomeModal
          isOpen={isWelcomeOpen}
          onPreviewTextScale={(textScale) => void updateSettings({ textScale })}
          onComplete={handleWelcomeComplete}
        />
      </Suspense>
    )}
  </div>
);
}

export default App;
