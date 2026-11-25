import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Space, TabState, ActiveMediaState, SiteAction } from '../types';
import { Homescreen } from './Homescreen';
import { FindBar } from './FindBar';
import type { PageColorReport } from '../utils/topBarTint';
import type { HomeWidgets } from '../shared/settings';
import { acquireViewportOverlay, setViewportCovered, setViewportPrewarmHandler, usePageUnderlay, useReservedPageBottom, useViewportOverlayCount } from '../hooks/useViewportOverlay';

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
  /** Zoom of an untouched page (the text size setting); Ctrl+0 returns to it. */
  defaultZoom?: number;
  /** Where web pages start: below the top bar (and the tab strip in top-tabs mode); 0 in HTML fullscreen. */
  contentTop?: number;
  /** Strips at the page edges kept free for hover-reveal zones (auto-hide sidebar, pins dock). */
  insetLeft?: number;
  insetRight?: number;
  homeWidgets?: HomeWidgets;
  onUpdateHomeWidgets?: (next: HomeWidgets) => void;
}

/** Height reserved above the page while the find bar is open (bar + margins). */
const FIND_BAR_INSET = 62;

/** Resolves once a data-URL image is decoded, so painting it never shows a blank frame. */
function decodeImage(src: string): Promise<void> {
  const img = new Image();
  img.src = src;
  return img.decode().catch(() => undefined);
}

const SLOW = Symbol('slow');

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Resolves after the browser has painted the current update. */
function nextPaint(): Promise<void> {
  return new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
}

function isWebUrl(url: string | undefined): url is string {
  return !!url && url !== 'about:blank' && !url.startsWith('serica://') && !url.startsWith('neutrino://');
}

/**
 * The web page area. Pages are native WebView2 views owned by the Rust tab
 * host; this component reports where they go, renders the app's own pages
 * (new tab) and shows a snapshot while UI overlays cover it. A tab whose
 * memory was freed (sleeping, or a closed pinned tab) wakes as soon as it is
 * shown, with its last capture in place until the page is back.
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
  defaultZoom = 1,
  contentTop = 44,
  homeWidgets,
  onUpdateHomeWidgets,
}) => {
  const pageAreaRef = useRef<HTMLDivElement>(null);
  const activeTabRef = useRef<TabState | undefined>(activeTab);
  activeTabRef.current = activeTab;
  const defaultZoomRef = useRef(defaultZoom);
  defaultZoomRef.current = defaultZoom;

  const [isFindOpen, setIsFindOpen] = useState(false);
  const [findQuery, setFindQuery] = useState('');
  const [findState, setFindState] = useState({ activeMatch: 0, totalMatches: 0 });
  const [frozenImage, setFrozenImage] = useState<string | null>(null);
  const frozenImageRef = useRef<string | null>(null);
  frozenImageRef.current = frozenImage;
  // The page's snapshot beneath it while UI slides over it (see setPageUnderlay).
  const underlayImage = usePageUnderlay();

  const isWebPage = Boolean(activeTab && isWebUrl(activeTab.url) && !activeTab.isSleeping && !activeTab.unloaded);
  const findVisible = isFindOpen && isWebPage;
  const reservedBottom = useReservedPageBottom();

  // ---- Report the page rectangle to the tab host (CSS px; the host applies the text size).
  // Computed from the layout rather than measured, so it is the same on the
  // new-tab page as on a site (a page created from the new-tab page starts in
  // the right place) and space/overlay animations never move the page.
  const pageTop = contentTop + (findVisible ? FIND_BAR_INSET : 0);
  const geometryRef = useRef({ left: insetLeft, right: insetRight, top: pageTop, bottom: reservedBottom });
  geometryRef.current = { left: insetLeft, right: insetRight, top: pageTop, bottom: reservedBottom };
  const lastBounds = useRef('');
  const reportBounds = useCallback(() => {
    if (!window.serica?.setViewport) return;
    const g = geometryRef.current;
    const b = [
      Math.round(g.left),
      Math.round(g.top),
      Math.max(0, Math.round(window.innerWidth - g.left - g.right)),
      Math.max(0, Math.round(window.innerHeight - g.top - g.bottom)),
    ];
    const key = b.join(',');
    if (key === lastBounds.current) return;
    lastBounds.current = key;
    void window.serica.setViewport(b[0], b[1], b[2], b[3]);
  }, []);

  useLayoutEffect(() => {
    reportBounds();
  }, [reportBounds, insetLeft, insetRight, pageTop, reservedBottom]);

  useEffect(() => {
    window.addEventListener('resize', reportBounds);
    return () => window.removeEventListener('resize', reportBounds);
  }, [reportBounds]);

  // A freshly created native view can come up at stale bounds (the first
  // WebView2 of a session is created while the layout is still settling), so
  // re-assert the rectangle when the active tab changes or finishes loading.
  useEffect(() => {
    lastBounds.current = '';
    reportBounds();
  }, [activeTab?.id, activeTab?.isLoading, isWebPage, reportBounds]);

  // ---- Freeze the page while UI overlays cover it.
  // Order matters for a flicker-free swap: capture the live page, paint the
  // capture in its place, and only then hide the native view. Thawing shows
  // the native view first and drops the capture a frame later.
  const overlayCount = useViewportOverlayCount();
  const wantFrozen = overlayCount > 0;
  // null = unknown backend state (first mount / UI reload): always sync once.
  const freezeApplied = useRef<boolean | null>(null);
  const freezeBusy = useRef(false);
  const wantFrozenRef = useRef(wantFrozen);
  wantFrozenRef.current = wantFrozen;
  const isWebPageRef = useRef(isWebPage);
  isWebPageRef.current = isWebPage;

  // A capture taken moments ago (pointer reached an overlay's button) is reused.
  const prewarmed = useRef<{ img: string; key: string; at: number } | null>(null);
  // The capture in flight, and the page it is of.
  const prewarmBusy = useRef<{ key: string; done: Promise<boolean> } | null>(null);
  const pageKey = () => `${activeTabRef.current?.id ?? ''}|${activeTabRef.current?.url ?? ''}`;
  useEffect(() => {
    // Resolves true once `prewarmed` holds a capture of the current page.
    const prewarm = (): Promise<boolean> | void => {
      if (freezeApplied.current || !isWebPageRef.current || !window.serica?.captureViewport) return;
      const key = pageKey();
      const busy = prewarmBusy.current;
      if (busy) return busy.key === key ? busy.done : busy.done.then(() => prewarm() ?? true);
      const fresh = prewarmed.current;
      if (fresh && fresh.key === key && performance.now() - fresh.at < 400) return Promise.resolve(true);
      const done = window.serica
        .captureViewport()
        .then(async (img) => {
          if (!img) return false;
          await decodeImage(img);
          prewarmed.current = { img, key, at: performance.now() };
          return true;
        })
        .catch(() => false)
        .finally(() => {
          prewarmBusy.current = null;
        });
      prewarmBusy.current = { key, done };
      return done;
    };
    setViewportPrewarmHandler(prewarm);
    return () => setViewportPrewarmHandler(null);
  }, []);
  const takePrewarmed = (): string | null => {
    const p = prewarmed.current;
    prewarmed.current = null;
    return p && p.key === pageKey() && performance.now() - p.at < 1500 ? p.img : null;
  };

  const applyFreeze = useCallback(async () => {
    if (freezeBusy.current) return;
    freezeBusy.current = true;
    try {
      while (freezeApplied.current !== wantFrozenRef.current) {
        if (wantFrozenRef.current) {
          if (isWebPageRef.current && window.serica?.captureViewport) {
            const ready = takePrewarmed();
            let img: string | null = ready;
            if (!img) {
              // A busy page (still loading) can take a second to capture. Do not
              // wait for it: use the tab's last capture, then swap in the fresh one.
              const fresh = window.serica.captureViewport().catch(() => null);
              const quick = await Promise.race([fresh, sleep(140).then(() => SLOW)]);
              if (quick === SLOW) {
                const tabId = activeTabRef.current?.id;
                const cached = tabId ? await window.serica.getTabPreview?.(tabId).catch(() => null) : null;
                img = cached ?? (await fresh);
                if (cached) {
                  const key = pageKey();
                  void fresh.then(async (late) => {
                    if (late && freezeApplied.current && wantFrozenRef.current && pageKey() === key) {
                      await decodeImage(late);
                      setFrozenImage(late);
                    }
                  });
                }
              } else {
                img = quick as string | null;
              }
            }
            if (img && wantFrozenRef.current) {
              if (!ready) await decodeImage(img);
              setFrozenImage(img);
              await nextPaint();
            }
            await window.serica.freezeViewport?.(false).catch(() => null);
          } else {
            const img = (await window.serica?.freezeViewport?.().catch(() => null)) ?? null;
            setFrozenImage(img);
          }
          freezeApplied.current = true;
          setViewportCovered(true);
        } else {
          await window.serica?.thawViewport?.().catch(() => undefined);
          freezeApplied.current = false;
          setViewportCovered(!isWebPageRef.current);
          await nextPaint();
          if (!wantFrozenRef.current) setFrozenImage(null);
        }
      }
    } finally {
      freezeBusy.current = false;
    }
  }, []);

  useEffect(() => {
    void applyFreeze();
  }, [wantFrozen, applyFreeze]);

  // Without a live page there is nothing to cover.
  useEffect(() => {
    if (!freezeApplied.current) setViewportCovered(!isWebPage);
  }, [isWebPage]);

  // A tab being shown (or woken from sleep: its webview takes a moment to
  // build and paint) draws its last capture in its place meanwhile, instead
  // of an empty area. The live page covers it as soon as it is up; it is
  // dropped once the tab has loaded.
  const [wakeImage, setWakeImage] = useState<{ tabId: string; img: string } | null>(null);
  const activeTabLoading = Boolean(activeTab?.isLoading);
  const wakeSawLoad = useRef(false);
  const isWebTab = Boolean(activeTab && isWebUrl(activeTab.url));
  const isAsleep = Boolean(activeTab?.isSleeping || activeTab?.unloaded);
  useEffect(() => {
    const id = activeTab?.id;
    wakeSawLoad.current = false;
    if (!id || !isWebTab) {
      setWakeImage(null);
      return;
    }
    let cancelled = false;
    void window.serica?.getTabPreview?.(id).then((img) => {
      if (!cancelled && typeof img === 'string' && img) setWakeImage({ tabId: id, img });
    });
    return () => {
      cancelled = true;
    };
  }, [activeTab?.id, isWebTab]);
  // Memory freed in the background is never shown: wake the tab right away.
  useEffect(() => {
    if (activeTab && isWebTab && isAsleep) void window.serica?.selectTab(activeTab.id);
  }, [activeTab?.id, isWebTab, isAsleep]);
  useEffect(() => {
    if (!wakeImage) return;
    if (activeTabLoading) {
      wakeSawLoad.current = true;
      return;
    }
    // A woken tab has loaded: drop the image once its first frame is surely
    // painted. A tab that was awake covers it at once; drop it later anyway.
    const t = window.setTimeout(() => setWakeImage(null), wakeSawLoad.current ? 700 : 5000);
    return () => window.clearTimeout(t);
  }, [wakeImage, activeTabLoading]);
  const placeholder = wakeImage && wakeImage.tabId === activeTab?.id ? wakeImage.img : null;

  // A different tab or page makes the snapshot stale. Switching tabs while the
  // page is covered (from the sidebar, for example) shows the new tab's last
  // capture instead of an empty area until the page comes back.
  useEffect(() => {
    if (!freezeApplied.current) return;
    setFrozenImage(null);
    const id = activeTab?.id;
    if (!id || !isWebPageRef.current) return;
    let cancelled = false;
    void window.serica?.getTabPreview?.(id).then((img) => {
      if (!cancelled && freezeApplied.current && typeof img === 'string' && img) setFrozenImage(img);
    });
    return () => {
      cancelled = true;
    };
  }, [activeTab?.id, activeTab?.url]);

  // ---- Find in page (native WebView2 find)
  const closeFindBar = useCallback(() => {
    setIsFindOpen(false);
    setFindQuery('');
    setFindState({ activeMatch: 0, totalMatches: 0 });
    void window.serica?.findInPage?.(null);
  }, []);

  useEffect(() => {
    closeFindBar();
  }, [activeTab?.id, closeFindBar]);

  useEffect(() => {
    return window.serica?.onFindResult?.((r) => {
      if (r.tabId === activeTabRef.current?.id) setFindState({ activeMatch: r.activeMatch, totalMatches: r.totalMatches });
    });
  }, []);

  const handleFindQueryChange = useCallback((q: string) => {
    setFindQuery(q);
    if (!q) setFindState({ activeMatch: 0, totalMatches: 0 });
    void window.serica?.findInPage?.(q || null);
  }, []);

  const handleFindNext = useCallback(() => {
    if (findQuery) void window.serica?.findInPage?.(findQuery, { forward: true, findNext: true });
  }, [findQuery]);

  const handleFindPrev = useCallback(() => {
    if (findQuery) void window.serica?.findInPage?.(findQuery, { forward: false, findNext: true });
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
      void window.serica?.siteAction?.(action);
    },
    [closeFindBar],
  );

  useEffect(() => {
    const unsubIpc = window.serica?.onSiteAction?.((p) => {
      if (p?.action) executeSiteAction(p.action as SiteAction);
    });
    const onCustom = (e: Event) => {
      const action = (e as CustomEvent<{ action?: SiteAction }>).detail?.action;
      if (action) executeSiteAction(action);
    };
    const onZoom = (e: Event) => {
      const dir = (e as CustomEvent<'in' | 'out' | 'reset'>).detail;
      const current = activeTabRef.current?.zoomFactor ?? defaultZoomRef.current;
      const next = dir === 'reset' ? defaultZoomRef.current : dir === 'in' ? Math.min(3, current + 0.1) : dir === 'out' ? Math.max(0.3, current - 0.1) : current;
      void window.serica?.setZoomFactor?.(Math.round(next * 10) / 10);
    };
    const unsubEsc = window.serica?.onEscapePressed?.(() => closeFindBar());
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
    const unsubColor = window.serica?.onPageColor?.((r) => {
      pageColors.current.set(r.tabId, r.themeColor);
      onPageColorRef.current?.(r.tabId, { themeColor: r.themeColor, sampled: null });
    });
    const unsubFs = window.serica?.onHtmlFullscreen?.((fs) => onFullscreenRef.current?.(fs));
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
      const res = await window.serica?.getMediaState?.(mediaTabId).catch(() => null);
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
    void window.serica?.mediaControl?.(mediaCommand.action, { tabId: target, value: mediaCommand.value });
  }, [mediaCommand, mediaTabId]);

  if (!activeTab) {
    return (
      <div className="webview-host empty-state">
        <p className="empty-message">No active tab</p>
      </div>
    );
  }

  const isNewTab = !isWebUrl(activeTab.url);

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
          // Seen only at the moving edge of UI cut out of the page: match the sidebar.
          backgroundColor: isWebPage ? 'var(--theme-bg-canvas)' : 'transparent',
        }}
      >
        {(isWebPage || (isAsleep && !isNewTab)) && (frozenImage || underlayImage || placeholder) && (
          <img
            className="native-page-snapshot"
            src={(frozenImage || underlayImage || placeholder)!}
            alt=""
            draggable={false}
            style={{ position: 'absolute', inset: 0, width: '100%', height: '100%', objectFit: 'fill', pointerEvents: 'none' }}
          />
        )}
      </div>

      {isNewTab && (
        <Homescreen
          isDarkMode={isDarkMode}
          onNavigate={onNavigate}
          onOpenCommandBar={onOpenCommandBar}
          userName={userName}
          widgets={homeWidgets}
          onUpdateWidgets={onUpdateHomeWidgets}
        />
      )}
    </div>
  );
};
