import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Space, TabState, ActiveMediaState, SiteAction } from '../types';
import { Homescreen } from './Homescreen';
import { FindBar } from './FindBar';
import { Moon, Sparkles, Globe } from 'lucide-react';
import type { PageColorReport } from '../utils/topBarTint';
import { useReservedPageBottom, useViewportOverlayCount } from '../hooks/useViewportOverlay';

interface BrowserViewportProps {
  activeTab?: TabState;
  tabs?: TabState[];
  spaces?: Space[];
  activeSpaceId?: string;
  isDarkMode?: boolean;
  isSidebarOpen?: boolean;
  onSelectSpace?: (spaceId: string) => void;
  onUpdateTab: (tabId: string, partial: Partial<TabState>) => void;
  onNavigate: (url: string) => void;
  onOpenCommandBar?: () => void;
  onMediaStateChange?: (state: ActiveMediaState | null) => void;
  mediaCommand?: { action: string; tabId?: string; value?: number; timestamp: number } | null;
  userName?: string;
  onTopHover?: (isHovering: boolean) => void;
  onFullscreenChange?: (isFullscreen: boolean) => void;
  /** Page color of the active tab for the top bar tint (theme-color meta). */
  onPageColor?: (tabId: string, report: PageColorReport) => void;
  /** Strips at the page edges kept free for hover-reveal zones (auto-hide sidebar, pins dock). */
  insetLeft?: number;
  insetRight?: number;
}

/** Height reserved above the page while the find bar is open (bar + margins). */
const FIND_BAR_INSET = 62;

function isWebUrl(url: string | undefined): url is string {
  return !!url && url !== 'about:blank' && !url.startsWith('neutrino://');
}

function hostnameOf(url: string): string {
  try {
    return new URL(url).hostname;
  } catch {
    return url;
  }
}

/**
 * The web page area. Pages are native WebView2 views owned by the Rust tab
 * host; this component reports where they go, renders the app's own pages
 * (new tab, sleeping tab) and shows a snapshot while UI overlays cover it.
 */
export const BrowserViewport: React.FC<BrowserViewportProps> = ({
  activeTab,
  tabs = [],
  isDarkMode = false,
  onNavigate,
  onOpenCommandBar,
  onMediaStateChange,
  mediaCommand,
  userName,
  onFullscreenChange,
  onPageColor,
  insetLeft = 0,
  insetRight = 0,
}) => {
  const pageAreaRef = useRef<HTMLDivElement>(null);
  const activeTabRef = useRef<TabState | undefined>(activeTab);
  activeTabRef.current = activeTab;

  const [isFindOpen, setIsFindOpen] = useState(false);
  const [findQuery, setFindQuery] = useState('');
  const [findState, setFindState] = useState({ activeMatch: 0, totalMatches: 0 });
  const [frozenImage, setFrozenImage] = useState<string | null>(null);

  const isWebPage = Boolean(activeTab && isWebUrl(activeTab.url) && !activeTab.isSleeping && !activeTab.unloaded);
  const findVisible = isFindOpen && isWebPage;
  const reservedBottom = useReservedPageBottom();

  // ---- Report the page rectangle to the tab host (CSS px = logical px).
  const lastBounds = useRef('');
  const reportBounds = useCallback(() => {
    const el = pageAreaRef.current;
    if (!el || !window.neutrino?.setViewport) return;
    const r = el.getBoundingClientRect();
    const b = [Math.round(r.left), Math.round(r.top), Math.round(r.width), Math.round(r.height)];
    const key = b.join(',');
    if (key === lastBounds.current) return;
    lastBounds.current = key;
    void window.neutrino.setViewport(b[0], b[1], b[2], b[3]);
  }, []);

  useLayoutEffect(() => {
    const el = pageAreaRef.current;
    if (!el) return;
    reportBounds();
    const ro = new ResizeObserver(() => reportBounds());
    ro.observe(el);
    window.addEventListener('resize', reportBounds);
    // Sidebar slide/resize animations move the page area without resizing it.
    const onTransitionEnd = () => reportBounds();
    document.addEventListener('transitionend', onTransitionEnd, true);
    const poll = window.setInterval(reportBounds, 400);
    return () => {
      ro.disconnect();
      window.removeEventListener('resize', reportBounds);
      document.removeEventListener('transitionend', onTransitionEnd, true);
      window.clearInterval(poll);
    };
  }, [reportBounds, findVisible, reservedBottom, activeTab?.id]);

  // A freshly created native view can come up at stale bounds (the first
  // WebView2 of a session is created while the layout is still settling), so
  // re-assert the rectangle when the active tab changes or finishes loading.
  useEffect(() => {
    lastBounds.current = '';
    reportBounds();
  }, [activeTab?.id, activeTab?.isLoading, isWebPage, reportBounds]);

  // ---- Freeze the page while UI overlays cover it.
  const overlayCount = useViewportOverlayCount();
  const wantFrozen = overlayCount > 0;
  // null = unknown backend state (first mount / UI reload): always sync once.
  const freezeApplied = useRef<boolean | null>(null);
  const freezeBusy = useRef(false);
  const wantFrozenRef = useRef(wantFrozen);
  wantFrozenRef.current = wantFrozen;

  const applyFreeze = useCallback(async () => {
    if (freezeBusy.current) return;
    freezeBusy.current = true;
    try {
      while (freezeApplied.current !== wantFrozenRef.current) {
        if (wantFrozenRef.current) {
          const img = (await window.neutrino?.freezeViewport?.().catch(() => null)) ?? null;
          freezeApplied.current = true;
          setFrozenImage(img);
        } else {
          await window.neutrino?.thawViewport?.().catch(() => undefined);
          freezeApplied.current = false;
          setFrozenImage(null);
        }
      }
    } finally {
      freezeBusy.current = false;
    }
  }, []);

  useEffect(() => {
    void applyFreeze();
  }, [wantFrozen, applyFreeze]);

  // A different tab or page makes the snapshot stale.
  useEffect(() => {
    if (freezeApplied.current) setFrozenImage(null);
  }, [activeTab?.id, activeTab?.url]);

  // ---- Find in page (native WebView2 find)
  const closeFindBar = useCallback(() => {
    setIsFindOpen(false);
    setFindQuery('');
    setFindState({ activeMatch: 0, totalMatches: 0 });
    void window.neutrino?.findInPage?.(null);
  }, []);

  useEffect(() => {
    closeFindBar();
  }, [activeTab?.id, closeFindBar]);

  useEffect(() => {
    return window.neutrino?.onFindResult?.((r) => {
      if (r.tabId === activeTabRef.current?.id) setFindState({ activeMatch: r.activeMatch, totalMatches: r.totalMatches });
    });
  }, []);

  const handleFindQueryChange = useCallback((q: string) => {
    setFindQuery(q);
    if (!q) setFindState({ activeMatch: 0, totalMatches: 0 });
    void window.neutrino?.findInPage?.(q || null);
  }, []);

  const handleFindNext = useCallback(() => {
    if (findQuery) void window.neutrino?.findInPage?.(findQuery, { forward: true, findNext: true });
  }, [findQuery]);

  const handleFindPrev = useCallback(() => {
    if (findQuery) void window.neutrino?.findInPage?.(findQuery, { forward: false, findNext: true });
  }, [findQuery]);

  // ---- Site actions (toolbar, command bar, shortcuts pressed inside pages)
  const executeSiteAction = useCallback(
    (action: SiteAction) => {
      if (action === 'open-find') {
        const tab = activeTabRef.current;
        if (tab && isWebUrl(tab.url) && !tab.isSleeping) setIsFindOpen(true);
        return;
      }
      if (action === 'home') closeFindBar();
      void window.neutrino?.siteAction?.(action);
    },
    [closeFindBar],
  );

  useEffect(() => {
    const unsubIpc = window.neutrino?.onSiteAction?.((p) => {
      if (p?.action) executeSiteAction(p.action as SiteAction);
    });
    const onCustom = (e: Event) => {
      const action = (e as CustomEvent<{ action?: SiteAction }>).detail?.action;
      if (action) executeSiteAction(action);
    };
    const onZoom = (e: Event) => {
      const dir = (e as CustomEvent<'in' | 'out' | 'reset'>).detail;
      const current = activeTabRef.current?.zoomFactor ?? 1;
      const next = dir === 'reset' ? 1 : dir === 'in' ? Math.min(3, current + 0.1) : dir === 'out' ? Math.max(0.3, current - 0.1) : current;
      void window.neutrino?.setZoomFactor?.(Math.round(next * 10) / 10);
    };
    const unsubEsc = window.neutrino?.onEscapePressed?.(() => closeFindBar());
    window.addEventListener('neutrino:site-action', onCustom);
    window.addEventListener('neutrino:zoom-action', onZoom);
    return () => {
      unsubIpc?.();
      unsubEsc?.();
      window.removeEventListener('neutrino:site-action', onCustom);
      window.removeEventListener('neutrino:zoom-action', onZoom);
    };
  }, [executeSiteAction, closeFindBar]);

  // ---- Page color and HTML fullscreen from the backend
  const onPageColorRef = useRef(onPageColor);
  onPageColorRef.current = onPageColor;
  const onFullscreenRef = useRef(onFullscreenChange);
  onFullscreenRef.current = onFullscreenChange;
  // Colors arrive once per page load; remember them so switching tabs re-tints.
  const pageColors = useRef(new Map<string, string | null>());
  useEffect(() => {
    const id = activeTab?.id;
    if (id && pageColors.current.has(id)) {
      onPageColorRef.current?.(id, { themeColor: pageColors.current.get(id) ?? null, sampled: null });
    }
  }, [activeTab?.id]);
  useEffect(() => {
    const unsubColor = window.neutrino?.onPageColor?.((r) => {
      pageColors.current.set(r.tabId, r.themeColor);
      onPageColorRef.current?.(r.tabId, { themeColor: r.themeColor, sampled: null });
    });
    const unsubFs = window.neutrino?.onHtmlFullscreen?.((fs) => onFullscreenRef.current?.(fs));
    return () => {
      unsubColor?.();
      unsubFs?.();
    };
  }, []);

  // ---- Media: poll the tab WebView2 reports as playing audio
  const mediaTab = tabs.find((t) => t.hasActiveMedia && !t.isSleeping);
  const mediaTabId = mediaTab?.id;
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;
  const onMediaRef = useRef(onMediaStateChange);
  onMediaRef.current = onMediaStateChange;
  useEffect(() => {
    if (!mediaTabId) {
      onMediaRef.current?.(null);
      return;
    }
    let cancelled = false;
    const poll = async () => {
      const res = await window.neutrino?.getMediaState?.(mediaTabId).catch(() => null);
      if (cancelled) return;
      const tab = tabsRef.current.find((t) => t.id === mediaTabId);
      onMediaRef.current?.(
        res
          ? {
              tabId: mediaTabId,
              url: tab?.url ?? '',
              tabTitle: tab?.title || res.tabTitle,
              isPlaying: res.isPlaying,
              currentTime: res.currentTime,
              duration: res.duration,
              volume: res.volume,
              hasVideo: res.hasVideo,
              inPip: Boolean((res as { inPip?: boolean }).inPip),
            }
          : null,
      );
    };
    void poll();
    const id = window.setInterval(poll, 2500);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [mediaTabId]);

  useEffect(() => {
    if (!mediaCommand) return;
    const target = mediaCommand.tabId || mediaTabId || activeTabRef.current?.id;
    if (!target) return;
    void window.neutrino?.mediaControl?.(mediaCommand.action, { tabId: target, value: mediaCommand.value });
  }, [mediaCommand, mediaTabId]);

  if (!activeTab) {
    return (
      <div className="webview-host empty-state">
        <p className="empty-message">No active tab</p>
      </div>
    );
  }

  const isNewTab = !isWebUrl(activeTab.url);
  const showSleeping = activeTab.isSleeping && !isNewTab;
  const showUnloaded = Boolean(activeTab.unloaded) && !isNewTab && !activeTab.isSleeping;

  return (
    <div className="webview-host">
      <FindBar
        isOpen={findVisible}
        query={findQuery}
        activeMatch={findState.activeMatch}
        totalMatches={findState.totalMatches}
        onQueryChange={handleFindQueryChange}
        onNext={handleFindNext}
        onPrev={handleFindPrev}
        onClose={closeFindBar}
      />

      {/* The native page is placed exactly over this element. */}
      <div
        ref={pageAreaRef}
        className="native-page-area"
        style={{
          position: 'absolute',
          left: insetLeft,
          right: insetRight,
          bottom: reservedBottom,
          top: findVisible ? FIND_BAR_INSET : 0,
          pointerEvents: isWebPage ? 'auto' : 'none',
          backgroundColor: isWebPage ? (isDarkMode ? '#1a1a1a' : '#ffffff') : 'transparent',
        }}
      >
        {isWebPage && frozenImage && (
          <img
            className="native-page-snapshot"
            src={frozenImage}
            alt=""
            draggable={false}
            style={{ position: 'absolute', inset: 0, width: '100%', height: '100%', objectFit: 'fill', pointerEvents: 'none' }}
          />
        )}
      </div>

      {isNewTab && !activeTab.isSleeping && (
        <Homescreen isDarkMode={isDarkMode} onNavigate={onNavigate} onOpenCommandBar={onOpenCommandBar} userName={userName} />
      )}

      {(showSleeping || showUnloaded) && (
        <div
          className="zen-sleeping-screen"
          onClick={() => window.neutrino?.selectTab(activeTab.id)}
          role="button"
          tabIndex={0}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') window.neutrino?.selectTab(activeTab.id);
          }}
          title="Click anywhere to wake tab"
        >
          <div className="zen-sleeping-ambient-glow" />
          <div className="zen-sleeping-card" onClick={(e) => e.stopPropagation()}>
            <div className="zen-sleeping-icon-ring">
              <Moon className="zen-sleeping-moon" size={26} strokeWidth={1.75} />
              <span className="zen-sleeping-orbit" />
            </div>
            <div className="zen-sleeping-badge">
              <span className="zen-sleeping-dot" />
              <span>{showUnloaded ? 'Pinned tab' : 'Zen Memory Saver'}</span>
            </div>
            <h2 className="zen-sleeping-title">
              {activeTab.title && !activeTab.title.startsWith('neutrino://') ? activeTab.title : 'Tab is Sleeping'}
            </h2>
            <div className="zen-sleeping-url-pill">
              <Globe size={12} className="zen-sleeping-globe" />
              <span className="zen-sleeping-url-text">{hostnameOf(activeTab.url)}</span>
            </div>
            <p className="zen-sleeping-desc">
              {showUnloaded
                ? 'This pinned tab is closed. Open it again to load the page.'
                : 'This tab was put to sleep to free memory. Waking it reloads the page.'}
            </p>
            <button
              type="button"
              className="zen-sleeping-wake-btn"
              onClick={(e) => {
                e.stopPropagation();
                window.neutrino?.selectTab(activeTab.id);
              }}
            >
              <Sparkles size={15} />
              <span>{showUnloaded ? 'Open Tab' : 'Wake Tab'}</span>
            </button>
            <div className="zen-sleeping-hint">Click anywhere to resume</div>
          </div>
        </div>
      )}
    </div>
  );
};
