import { useState, useEffect, useCallback, useRef } from 'react';
import type { BrowserAppState, BlockedTrackerItem, MoveOp, SidebarState, SpacePatch, TabState } from '../types';

const INITIAL_STATE: BrowserAppState = {
  spaces: [
    { id: 'space-study', name: 'Study', icon: 'BookOpen', color: '#E4E4E7' },
    { id: 'space-work', name: 'Work', icon: 'Briefcase', color: '#A1A1AA' },
    { id: 'space-leisure', name: 'Leisure', icon: 'Coffee', color: '#71717A' },
  ],
  activeSpaceId: 'space-study',
  tabs: [
    {
      id: 'tab-init',
      spaceId: 'space-study',
      title: 'New Tab',
      url: 'serica://newtab',
      isLoading: false,
      canGoBack: false,
      canGoForward: false,
      isPinned: false,
      isSleeping: false,
      blockedCount: 0,
    },
  ],
  activeTabId: 'tab-init',
  shieldStats: {
    totalBlocked: 0,
    adsBlocked: 0,
    trackersBlocked: 0,
    analyticsBlocked: 0,
    fingerprintersBlocked: 0,
    dataSavedKb: 0,
  },
  resourceStats: {
    memorySavedMb: 0,
    sleepingCount: 0,
  },
};

export function useBrowser() {
  const [state, setState] = useState<BrowserAppState>(INITIAL_STATE);
  const [blockedTrackers, setBlockedTrackers] = useState<BlockedTrackerItem[]>([]);
  const [isSidebarCollapsed, setIsSidebarCollapsed] = useState(false);
  const [isCommandBarOpen, setIsCommandBarOpen] = useState(false);
  const [isShieldModalOpen, setIsShieldModalOpen] = useState(false);
  const [isMcpModalOpen, setIsMcpModalOpen] = useState(false);
  // A space switch the backend has not confirmed yet. State updates sent
  // before it (for an earlier switch, say) must not flip the UI back.
  const pendingSpaceRef = useRef<{ id: string; seq: number } | null>(null);
  const spaceSeqRef = useRef(0);

  useEffect(() => {
    if (!window.serica) return;

    const applyIncomingState = (newState: BrowserAppState) => {
      // Spaces come from the ProfileStore as-is: legacy ids (DEFAULT/PLAN/MAX) are already
      // migrated in main, and rewriting them here would undo user renames and drop `profileId`.
      const spaces = newState.spaces || [];
      const incomingSpace = newState.activeSpaceId || spaces[0]?.id || INITIAL_STATE.activeSpaceId;
      const pending = pendingSpaceRef.current;
      if (pending && pending.id === incomingSpace) pendingSpaceRef.current = null;
      setState((prev) => {
        const keepOptimistic =
          pendingSpaceRef.current !== null && spaces.some((s) => s.id === pendingSpaceRef.current!.id);
        return {
          ...newState,
          spaces: spaces.length > 0 ? spaces : INITIAL_STATE.spaces,
          activeSpaceId: keepOptimistic ? prev.activeSpaceId : incomingSpace,
          activeTabId: keepOptimistic ? prev.activeTabId : newState.activeTabId,
        };
      });
    };

    const unregState = window.serica.onStateUpdate(applyIncomingState);

    window.serica.getState?.().then((initial) => {
      if (initial && Array.isArray(initial.spaces)) {
        applyIncomingState(initial);
      }
    }).catch(() => { });

    // A tab moved to another profile's space reloads in that profile's session (R1.12, R17.5).
    // The state-update that follows carries the same partition; applying it now lets the
    // webview remount without waiting for it.
    const unregPartition = window.serica.onTabPartitionChanged?.(({ tabId, partition, url }) => {
      setState((prev) => ({
        ...prev,
        tabs: prev.tabs.map((t) => (t.id === tabId ? { ...t, partition, url } : t)),
      }));
    });

    // High-performance batched tracker updates (prevents UI thread hitching on heavy pages)
    const trackerQueue: BlockedTrackerItem[] = [];
    let trackerFlushTimer: any = null;

    const flushTrackerQueue = () => {
      trackerFlushTimer = null;
      if (trackerQueue.length === 0) return;

      const itemsToFlush = trackerQueue.splice(0, trackerQueue.length);

      setBlockedTrackers((prev) => [
        ...itemsToFlush.map((it) => ({ ...it, timestamp: it.timestamp || Date.now() })),
        ...prev,
      ].slice(0, 50));

      // Pull authoritative stats from the main engine rather than blindly adding deltas on top of
      // state-update snapshots (which otherwise double-counted bursts until the next state update).
      if (window.serica?.getShieldStats) {
        window.serica.getShieldStats().then((stats: any) => {
          if (stats) {
            setState((prev) => ({
              ...prev,
              shieldStats: { ...prev.shieldStats, ...stats },
            }));
          }
        }).catch(() => { });
        return;
      }

      const totalCount = itemsToFlush.length;
      let adsCount = 0;
      let analyticsCount = 0;
      let fingerprintersCount = 0;
      let trackersCount = 0;
      for (const it of itemsToFlush) {
        if (it.category === 'Advertising') adsCount++;
        else if (it.category === 'Analytics' || it.category === 'Telemetry') analyticsCount++;
        else if (it.category === 'Fingerprinting' || it.category === 'Cryptomining') fingerprintersCount++;
        else trackersCount++;
      }

      setState((prev) => ({
        ...prev,
        shieldStats: {
          ...prev.shieldStats,
          totalBlocked: prev.shieldStats.totalBlocked + totalCount,
          trackersBlocked: prev.shieldStats.trackersBlocked + trackersCount,
          adsBlocked: prev.shieldStats.adsBlocked + adsCount,
          analyticsBlocked: prev.shieldStats.analyticsBlocked + analyticsCount,
          fingerprintersBlocked: prev.shieldStats.fingerprintersBlocked + fingerprintersCount,
          dataSavedKb: prev.shieldStats.dataSavedKb + totalCount * 54,
        },
      }));
    };

    const unregTracker = window.serica.onTrackerBlocked?.((item) => {
      trackerQueue.push(item);
      if (!trackerFlushTimer) {
        trackerFlushTimer = setTimeout(flushTrackerQueue, 250);
      }
    });

    // Initial shield stats fetch
    window.serica.getShieldStats?.().then((stats: any) => {
      if (stats) {
        setState((prev) => ({
          ...prev,
          shieldStats: { ...prev.shieldStats, ...stats },
        }));
      }
    }).catch(() => { });

    const handleMockTabs = (e: any) => {
      if (e.detail?.tabs) {
        setState((prev) => ({
          ...prev,
          tabs: e.detail.tabs,
          activeTabId: e.detail.activeTabId || e.detail.tabs[0]?.id || prev.activeTabId,
        }));
      }
    };
    window.addEventListener('serica:mock-tabs', handleMockTabs);

    return () => {
      if (trackerFlushTimer) clearTimeout(trackerFlushTimer);
      unregState?.();
      unregPartition?.();
      unregTracker?.();
      window.removeEventListener('serica:mock-tabs', handleMockTabs);
    };
  }, []);

  const navigate = useCallback((url: string) => {
    if (typeof url === 'string') {
      const normalized = url.trim().toLowerCase();
      // neutrino://settings, neutrino://history, neutrino://vpn open panels without changing Active_Tab URL.
      if (normalized === 'serica://settings' || normalized === 'neutrino://settings') {
        window.dispatchEvent(new CustomEvent('serica:open-settings'));
        return;
      }
      if (normalized === 'serica://history' || normalized === 'neutrino://history') {
        window.dispatchEvent(new CustomEvent('serica:open-history'));
        return;
      }
      if (normalized === 'serica://downloads' || normalized === 'neutrino://downloads') {
        window.dispatchEvent(new CustomEvent('serica:open-downloads'));
        return;
      }
      if (normalized === 'serica://extensions' || normalized === 'neutrino://extensions') {
        window.dispatchEvent(new CustomEvent('serica:open-extensions'));
        return;
      }
      if (normalized === 'serica://vpn' || normalized === 'neutrino://vpn') {
        window.dispatchEvent(new CustomEvent('serica:open-vpn'));
        return;
      }
    }
    window.serica?.navigate(url);
  }, []);

  const createTab = useCallback((url?: string, spaceId?: string) => {
    return window.serica?.createTab(url, spaceId);
  }, []);

  const closeTab = useCallback((tabId: string) => {
    window.serica?.closeTab(tabId);
  }, []);

  const reopenClosedTab = useCallback(() => {
    return window.serica?.reopenClosedTab?.();
  }, []);

  const cycleTab = useCallback((direction: 1 | -1) => {
    return window.serica?.cycleTab?.(direction);
  }, []);

  const selectTab = useCallback((tabId: string) => {
    window.serica?.selectTab(tabId);
    setState((prev) => ({
      ...prev,
      activeTabId: tabId,
    }));
  }, []);

  const togglePinTab = useCallback((tabId: string) => {
    window.serica?.togglePinTab(tabId);
  }, []);

  const toggleSleepTab = useCallback((tabId: string) => {
    window.serica?.toggleSleepTab(tabId);
  }, []);

  const selectSpace = useCallback((spaceId: string): Promise<void> => {
    const seq = ++spaceSeqRef.current;
    pendingSpaceRef.current = { id: spaceId, seq };
    const done = Promise.resolve(window.serica?.selectSpace(spaceId))
      .catch(() => undefined)
      .finally(() => {
        // Confirmed (or failed): backend updates are authoritative again.
        if (pendingSpaceRef.current?.seq === seq) pendingSpaceRef.current = null;
      });
    setState((prev) => {
      if (!prev.spaces.some((s) => s.id === spaceId)) return prev;
      // Same pick as the backend (`Store::activate_space`): the space's most
      // recently used loaded tab. A different guess shows the wrong tab (say
      // its New Tab page) for a moment until the backend's state arrives.
      let nextActiveTab: TabState | undefined;
      for (const t of prev.tabs) {
        if (t.spaceId !== spaceId || t.unloaded) continue;
        if (!nextActiveTab || (t.lastActiveAt ?? 0) >= (nextActiveTab.lastActiveAt ?? 0)) nextActiveTab = t;
      }
      const pickedId = nextActiveTab?.id;
      return {
        ...prev,
        activeSpaceId: spaceId,
        activeTabId: pickedId ?? prev.activeTabId,
        // The backend wakes the picked tab; so does this guess, so its page
        // (and its last capture, while it wakes) shows right away.
        tabs: nextActiveTab?.isSleeping
          ? prev.tabs.map((t) => (t.id === pickedId ? { ...t, isSleeping: false } : t))
          : prev.tabs,
      };
    });
    return done;
  }, []);

  const createSpace = useCallback(async (name: string, color?: string) => {
    const trimmed = typeof name === 'string' ? name.trim() : '';
    if (!trimmed) return null;

    if (window.serica?.createSpace) {
      const created = await window.serica.createSpace(trimmed, color);
      if (created) {
        setState((prev) => ({
          ...prev,
          spaces: prev.spaces.some((s) => s.id === created.id) ? prev.spaces : [...prev.spaces, created],
          activeSpaceId: created.id,
        }));
      }
      return created ?? null;
    }

    const newSpace = {
      id: `space-local-${Date.now().toString(36)}`,
      name: trimmed,
      icon: 'Sparkles',
      color: color || '#8B5CF6',
    };
    const newTabId = `tab-local-${Date.now().toString(36)}`;
    setState((prev) => ({
      ...prev,
      spaces: [...prev.spaces, newSpace],
      activeSpaceId: newSpace.id,
      tabs: [
        ...prev.tabs,
        {
          id: newTabId,
          spaceId: newSpace.id,
          title: 'New Tab',
          url: 'serica://newtab',
          isLoading: false,
          canGoBack: false,
          canGoForward: false,
          isPinned: false,
          isSleeping: false,
          blockedCount: 0,
        },
      ],
      activeTabId: newTabId,
    }));
    return newSpace;
  }, []);

  const deleteSpace = useCallback((spaceId: string) => {
    return window.serica?.deleteSpace?.(spaceId);
  }, []);

  const updateSpace = useCallback((spaceId: string, patch: SpacePatch) => {
    return window.serica?.updateSpace?.(spaceId, patch);
  }, []);

  const reorderSpace = useCallback((spaceId: string, toIndex: number) => {
    return window.serica?.reorderSpace?.(spaceId, toIndex);
  }, []);

  const moveTab = useCallback((op: MoveOp) => {
    return window.serica?.moveTab?.(op);
  }, []);

  const pinTab = useCallback((tabId: string) => {
    return window.serica?.pinTab?.(tabId);
  }, []);

  const unpinTab = useCallback((tabId: string) => {
    return window.serica?.unpinTab?.(tabId);
  }, []);

  const addFavorite = useCallback((tabId: string) => {
    return window.serica?.addFavorite?.(tabId);
  }, []);

  const removeFavorite = useCallback((favoriteId: string) => {
    return window.serica?.removeFavorite?.(favoriteId);
  }, []);

  const duplicateTab = useCallback((id: string, spaceId?: string) => {
    return window.serica?.duplicateTab?.(id, spaceId);
  }, []);

  const createFolder = useCallback((spaceId: string, name: string) => {
    return window.serica?.createFolder?.(spaceId, name);
  }, []);

  const updateFolder = useCallback((folderId: string, patch: any) => {
    return window.serica?.updateFolder?.(folderId, patch);
  }, []);

  const deleteFolder = useCallback((folderId: string) => {
    return window.serica?.deleteFolder?.(folderId);
  }, []);

  const moveTabToFolder = useCallback((tabId: string, folderId: string | null) => {
    return window.serica?.moveTabToFolder?.(tabId, folderId);
  }, []);

  const updateSidebarState = useCallback((patch: Partial<SidebarState>) => {
    return window.serica?.updateSidebarState?.(patch);
  }, []);

  const reorderTabs = useCallback((draggedTabId: string, targetTabId: string) => {
    return window.serica?.reorderTabs?.(draggedTabId, targetTabId);
  }, []);

  const toggleShields = useCallback(() => {
    return window.serica?.toggleShields();
  }, []);

  const resetShieldStats = useCallback(async () => {
    setBlockedTrackers([]);
    if (window.serica?.resetShieldStats) {
      const nextStats = await window.serica.resetShieldStats();
      setState((prev) => ({ ...prev, shieldStats: nextStats }));
    } else {
      setState((prev) => ({
        ...prev,
        shieldStats: {
          totalBlocked: 0,
          adsBlocked: 0,
          trackersBlocked: 0,
          analyticsBlocked: 0,
          fingerprintersBlocked: 0,
          dataSavedKb: 0,
        },
      }));
    }
  }, []);

  const zenOptimize = useCallback(() => {
    window.serica?.zenOptimize();
  }, []);

  const currentSpaceTabs = state.tabs.filter(
    (t) => t.spaceId === state.activeSpaceId
  );

  // Favorites are shared by every space of the active space's owning profile (R14.2).
  const activeSpaceProfileId =
    state.spaces.find((s) => s.id === state.activeSpaceId)?.profileId ?? state.activeProfileId;
  const currentFavorites = (state.favorites ?? [])
    .filter((f) => f.profileId === activeSpaceProfileId)
    .sort((a, b) => a.order - b.order);

  const activeTab: TabState | undefined =
    currentSpaceTabs.find((t) => t.id === state.activeTabId) ||
    currentSpaceTabs[0] ||
    state.tabs.find((t) => t.id === state.activeTabId) ||
    state.tabs[0];

  return {
    state,
    activeTab,
    currentSpaceTabs,
    currentFavorites,
    blockedTrackers,
    isSidebarCollapsed,
    setIsSidebarCollapsed,
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
    cycleTab,
    selectTab,
    togglePinTab,
    toggleSleepTab,
    selectSpace,
    createSpace,
    deleteSpace,
    updateSpace,
    reorderSpace,
    reorderTabs,
    moveTab,
    pinTab,
    unpinTab,
    addFavorite,
    removeFavorite,
    duplicateTab,
    createFolder,
    updateFolder,
    deleteFolder,
    moveTabToFolder,
    updateSidebarState,
    toggleShields,
    resetShieldStats,
    zenOptimize,
  };
}

