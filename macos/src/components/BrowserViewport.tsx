import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Space, TabState, ActiveMediaState, SiteAction, type SplitView } from '../types';
import { Homescreen } from './Homescreen';
import { useSplitDropHint } from '../hooks/splitDrop';
import { FindBar } from './FindBar';
import type { PageColorReport } from '../utils/topBarTint';
import type { HomeBackground, HomeWidgets, UiScale } from '../shared/settings';
import { acquireViewportOverlay, prewarmPageUnderlay, refreshPageCapture, setUnderlayPanes, setViewportCovered, setViewportPrewarmHandler, type Underlay, type UnderlayPane, usePageUnderlays, useReservedPageBottom, useViewportOverlayCount } from '../hooks/useViewportOverlay';

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
  /** New tab pages show a search box (Settings › New tab: a real tab). */
  homeSearchBox?: boolean;
  searchEngine?: import('../shared/settings').SearchEngineId;
  onTopHover?: (isHovering: boolean) => void;
  onFullscreenChange?: (isFullscreen: boolean) => void;
  /** Page color of the active tab for the top bar tint (theme-color meta). */
  onPageColor?: (tabId: string, report: PageColorReport) => void;
  /** Zoom of an untouched page (the text size setting); ⌘0 returns to it. */
  defaultZoom?: number;
  /** Where web pages start: below the top bar (and the tab strip in top-tabs mode); 0 in HTML fullscreen. */
  contentTop?: number;
  /** Strips at the page edges kept free for hover-reveal zones (auto-hide sidebar, pins dock). */
  insetLeft?: number;
  insetRight?: number;
  homeWidgets?: HomeWidgets;
  onUpdateHomeWidgets?: (next: HomeWidgets) => void;
  homeBackground?: HomeBackground;
  dockVisible?: boolean;
  uiScale?: UiScale;
  /** Panels slide over the live page (auto-hide sidebar or top bar): keep a capture of it ready. */
  captureAhead?: boolean;
  /** The active tab's split view (both tabs are web pages), and the other pane's tab. */
  split?: SplitView | null;
  partnerTab?: TabState;
}

/** Split view geometry, as in the backend (tabhost.rs `pane_bounds`), CSS px. */
const SPLIT_GAP = 10;
const SPLIT_TOP = 3;

interface PaneRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** A pane of a split in a page area `w` x `h` (page-area coordinates). */
function paneRect(w: number, h: number, ratio: number, left: boolean): PaneRect {
  const inner = Math.max(0, w - SPLIT_GAP);
  const lw = Math.round(inner * Math.min(0.8, Math.max(0.2, ratio)));
  const top = Math.min(SPLIT_TOP, h);
  return left ? { x: 0, y: top, w: lw, h: h - top } : { x: lw + SPLIT_GAP, y: top, w: inner - lw, h: h - top };
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

const rectStyle = (r: PaneRect): React.CSSProperties => ({ left: r.x, top: r.y, width: r.w, height: r.h });

/**
 * Split view chrome, around the two page panes (native views placed in the
 * same rectangles by the backend): a bar above each pane (the focused one is
 * lit), and the divider between them, which resizes the panes when dragged,
 * resets them on double-click, and has a button that ends the split.
 */
const SplitChrome: React.FC<{ split: SplitView; activeId: string; panes: { h: number }; areaWidth: number }> = ({ split, activeId, panes, areaWidth }) => {
  const [live, setLive] = useState<number | null>(null);
  const ratio = live ?? split.ratio;
  const h = panes.h;
  const left = paneRect(areaWidth, h, ratio, true);
  const right = paneRect(areaWidth, h, ratio, false);
  const drag = useRef<{ x0: number; raf: number; next: number | null } | null>(null);
  // The saved ratio arrived: stop showing the local one.
  useEffect(() => {
    if (!drag.current) setLive(null);
  }, [split.ratio]);
  const clampRatio = (r: number) => Math.min(0.8, Math.max(0.2, r));
  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    const area = e.currentTarget.parentElement?.getBoundingClientRect();
    drag.current = { x0: area?.left ?? 0, raf: 0, next: null };
    document.body.classList.add('split-resizing');
  };
  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    const r = clampRatio((e.clientX - d.x0 - SPLIT_GAP / 2) / Math.max(1, areaWidth - SPLIT_GAP));
    setLive(r);
    d.next = r;
    // One resize of the pages per frame.
    if (!d.raf) {
      d.raf = requestAnimationFrame(() => {
        d.raf = 0;
        if (d.next !== null) void window.serica?.setSplitRatio?.(activeId, d.next, false);
      });
    }
  };
  const onPointerUp = () => {
    const d = drag.current;
    if (!d) return;
    if (d.raf) cancelAnimationFrame(d.raf);
    drag.current = null;
    document.body.classList.remove('split-resizing');
    if (d.next !== null) void window.serica?.setSplitRatio?.(activeId, d.next, true);
  };
  return (
    <>
      <div className={`split-focus-bar${split.left === activeId ? ' active' : ''}`} style={{ position: 'absolute', left: left.x, top: 0, width: left.w, height: SPLIT_TOP }} />
      <div className={`split-focus-bar${split.right === activeId ? ' active' : ''}`} style={{ position: 'absolute', left: right.x, top: 0, width: right.w, height: SPLIT_TOP }} />
      <div
        className="split-divider"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize split view"
        aria-valuenow={Math.round(ratio * 100)}
        style={{ position: 'absolute', left: left.w, top: 0, width: SPLIT_GAP, height: h }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onDoubleClick={() => void window.serica?.setSplitRatio?.(activeId, 0.5, true)}
      >
        <span className="split-divider-grip" aria-hidden="true" />
        <button
          type="button"
          className="split-close"
          title="Close split view"
          aria-label="Close split view"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={() => void window.serica?.unsplitTab?.(activeId)}
        >
          <svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true">
            <path d="M1 1l6 6M7 1L1 7" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
          </svg>
        </button>
      </div>
    </>
  );
};

/** A capture of part of the page, drawn where that part is (page-area coordinates). */
const UnderlayCanvas: React.FC<{ underlay: Underlay; left: number; top: number }> = ({ underlay, left, top }) => {
  const ref = useRef<HTMLCanvasElement>(null);
  useLayoutEffect(() => {
    const c = ref.current;
    if (!c) return;
    const b = underlay.bitmap;
    c.width = b.width;
    c.height = b.height;
    c.getContext('2d')?.drawImage(b, 0, 0);
  }, [underlay.bitmap]);
  const [x, y, w, h] = underlay.rect;
  return (
    <canvas
      ref={ref}
      className="native-page-snapshot"
      aria-hidden="true"
      style={{ position: 'absolute', left: x - left, top: y - top, width: w, height: h, pointerEvents: 'none' }}
    />
  );
};

/**
 * The web page area. Pages are native WebKit views owned by the Rust tab
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
  homeSearchBox,
  searchEngine,
  onFullscreenChange,
  onPageColor,
  insetLeft = 0,
  insetRight = 0,
  defaultZoom = 1,
  contentTop = 44,
  homeWidgets,
  onUpdateHomeWidgets,
  homeBackground,
  dockVisible,
  uiScale,
  captureAhead = false,
  split = null,
  partnerTab,
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
  // In a split view, the other pane's picture while the page is covered.
  const [frozenPartner, setFrozenPartner] = useState<string | null>(null);
  const partnerRef = useRef<string | null>(null);
  partnerRef.current = split && partnerTab ? partnerTab.id : null;
  const frozenImageRef = useRef<string | null>(null);
  frozenImageRef.current = frozenImage;
  // Captures of the page beneath panels sliding over it (see preparePageUnderlay).
  const underlays = usePageUnderlays();
  // A tab dragged from the sidebar over the page (split view).
  const splitDrop = useSplitDropHint();

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

  // Where captures drawn under sliding panels go, and which page they show:
  // the page area, or each pane of a split.
  const underlayKey = isWebPage && activeTab ? `${activeTab.id}|${activeTab.url}` : '';
  const partnerKey = split && partnerTab ? `${partnerTab.id}|${partnerTab.url}` : '';
  const splitRatio = split?.ratio ?? 0.5;
  const leftId = split?.left ?? '';
  useLayoutEffect(() => {
    const update = () => {
      if (!underlayKey) {
        setUnderlayPanes([]);
        return;
      }
      const w = Math.max(0, window.innerWidth - insetLeft - insetRight);
      const h = Math.max(0, window.innerHeight - pageTop - reservedBottom);
      const at = (tabId: string, key: string, r: PaneRect): UnderlayPane => ({ tabId, key, left: insetLeft + r.x, top: pageTop + r.y, width: r.w, height: r.h });
      const activeId = underlayKey.split('|')[0];
      if (partnerKey) {
        const partnerId = partnerKey.split('|')[0];
        setUnderlayPanes([
          at(activeId, underlayKey, paneRect(w, h, splitRatio, leftId === activeId)),
          at(partnerId, partnerKey, paneRect(w, h, splitRatio, leftId === partnerId)),
        ]);
      } else {
        setUnderlayPanes([at(activeId, underlayKey, { x: 0, y: 0, w, h })]);
      }
    };
    update();
    window.addEventListener('resize', update);
    return () => window.removeEventListener('resize', update);
  }, [underlayKey, partnerKey, splitRatio, leftId, insetLeft, insetRight, pageTop, reservedBottom]);

  // The page area's size (CSS px), for laying out a split's panes.
  const [areaSize, setAreaSize] = useState({ w: 0, h: 0 });
  useLayoutEffect(() => {
    const update = () => {
      const w = Math.max(0, window.innerWidth - insetLeft - insetRight);
      const h = Math.max(0, window.innerHeight - pageTop - reservedBottom);
      setAreaSize((s) => (s.w === w && s.h === h ? s : { w, h }));
    };
    update();
    window.addEventListener('resize', update);
    return () => window.removeEventListener('resize', update);
  }, [insetLeft, insetRight, pageTop, reservedBottom]);
  const splitPanes =
    isWebPage && split && partnerTab && activeTab
      ? {
          active: paneRect(areaSize.w, areaSize.h, split.ratio, split.left === activeTab.id),
          partner: paneRect(areaSize.w, areaSize.h, split.ratio, split.left === partnerTab.id),
          h: areaSize.h,
        }
      : null;

  // The pointer is heading for an edge over the page: capture it now, so a
  // panel opening there has a capture to slide over without waiting.
  useEffect(() => {
    if (!underlayKey) return;
    return window.serica?.onEdgeApproach?.(() => prewarmPageUnderlay());
  }, [underlayKey]);
  // Keep a current capture ready: after the page shows up or finishes
  // loading, and after each scroll settles.
  const pageLoading = Boolean(activeTab?.isLoading);
  useEffect(() => {
    if (!underlayKey || !captureAhead || pageLoading) return;
    const t = window.setTimeout(() => refreshPageCapture(), 700);
    return () => window.clearTimeout(t);
  }, [underlayKey, partnerKey, captureAhead, pageLoading]);
  useEffect(() => {
    if (!underlayKey || !captureAhead) return;
    const ids = [activeTab?.id, partnerTab?.id];
    return window.serica?.onPageSettled?.((id) => {
      if (ids.includes(id)) refreshPageCapture(id);
    });
  }, [underlayKey, captureAhead, activeTab?.id, partnerTab?.id]);

  useEffect(() => {
    window.addEventListener('resize', reportBounds);
    return () => window.removeEventListener('resize', reportBounds);
  }, [reportBounds]);

  // A freshly created native view can come up at stale bounds (the first
  // web view of a session is created while the layout is still settling), so
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

  // Covering a page that has not painted yet: there is nothing to capture.
  const loadCoverRef = useRef<string | null>(null);
  const applyFreeze = useCallback(async () => {
    if (freezeBusy.current) return;
    freezeBusy.current = true;
    try {
      while (freezeApplied.current !== wantFrozenRef.current) {
        if (wantFrozenRef.current) {
          if (loadCoverRef.current && loadCoverRef.current === activeTabRef.current?.id) {
            await window.serica?.freezeViewport?.(false).catch(() => null);
            setFrozenImage(null);
          } else if (isWebPageRef.current && window.serica?.captureViewport) {
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
            // The other pane of a split: its last capture, or a fresh one.
            const partner = partnerRef.current;
            const partnerImg = partner
              ? await Promise.race([
                  window.serica.captureViewport(partner).catch(() => null),
                  sleep(160).then(() => window.serica?.getTabPreview?.(partner).catch(() => null) ?? null),
                ])
              : null;
            if (img && wantFrozenRef.current) {
              if (!ready) await decodeImage(img);
              if (partnerImg) await decodeImage(partnerImg);
              setFrozenImage(img);
              setFrozenPartner(partnerImg ?? null);
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
          if (!wantFrozenRef.current) {
            setFrozenImage(null);
            setFrozenPartner(null);
          }
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

  // A page opened in a new tab (or from the new tab page) has nothing to
  // show until it first paints: the page area went theme colour, then black
  // (the page being built), then white (blank), then the site. The page
  // stays covered by one calm surface until it has loaded, then shows once.
  // Tabs that were already open (restored, or woken from sleep) keep their
  // last capture instead (see wakeImage).
  const knownUrls = useRef<Map<string, string> | null>(null);
  if (!knownUrls.current) knownUrls.current = new Map(tabs.map((t) => [t.id, t.url]));
  const [loadCover, setLoadCover] = useState<string | null>(null);
  const coverSawLoad = useRef(false);
  useEffect(() => {
    const id = activeTab?.id;
    if (!id) return;
    const before = knownUrls.current!.get(id);
    knownUrls.current!.set(id, activeTab!.url);
    const fresh = before === undefined || !isWebUrl(before);
    if (fresh && isWebPage && !placeholder) {
      loadCoverRef.current = id;
      coverSawLoad.current = false;
      setLoadCover(id);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeTab?.id, activeTab?.url]);
  useEffect(() => {
    if (!loadCover) return;
    if (loadCover !== activeTab?.id || !isWebPage) {
      setLoadCover(null);
      return;
    }
    if (activeTabLoading) coverSawLoad.current = true;
    // Loaded: a moment for its first frame, then the page shows. A page that
    // never reports loading, or a slow site, shows as it is after a while.
    const ms = activeTabLoading ? 1800 : coverSawLoad.current ? 350 : 1500;
    const t = window.setTimeout(() => setLoadCover(null), ms);
    return () => window.clearTimeout(t);
  }, [loadCover, activeTab?.id, isWebPage, activeTabLoading]);
  // The cover itself, held for the whole load (not per loading change).
  useEffect(() => {
    if (!loadCover) {
      loadCoverRef.current = null;
      return;
    }
    const release = acquireViewportOverlay();
    return release;
  }, [loadCover]);

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

  // ---- Find in page (the backend's find, with every match highlighted)
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

  // ---- Media: poll the tab WebKit reports as playing audio
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

  // Send each command once. The effect also re-runs when the playing tab
  // changes (it does each time playback starts or stops), and re-sending the
  // last command then paused music the user had just started again.
  const sentMediaCommandRef = useRef<number | null>(null);
  useEffect(() => {
    if (!mediaCommand || sentMediaCommandRef.current === mediaCommand.timestamp) return;
    sentMediaCommandRef.current = mediaCommand.timestamp;
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
        {loadCover && loadCover === activeTab?.id && <div className="page-load-cover" aria-hidden="true" />}
        {isWebPage &&
          !frozenImage &&
          underlays.map((u) => <UnderlayCanvas key={u.id} underlay={u} left={insetLeft} top={pageTop} />)}
        {(isWebPage || (isAsleep && !isNewTab)) && (frozenImage || placeholder) && (
          <img
            className="native-page-snapshot"
            src={(frozenImage || placeholder)!}
            alt=""
            draggable={false}
            style={{ position: 'absolute', ...(splitPanes ? rectStyle(splitPanes.active) : { inset: 0, width: '100%', height: '100%' }), objectFit: 'fill', pointerEvents: 'none' }}
          />
        )}
        {splitPanes && frozenImage && frozenPartner && (
          <img
            className="native-page-snapshot"
            src={frozenPartner}
            alt=""
            draggable={false}
            style={{ position: 'absolute', ...rectStyle(splitPanes.partner), objectFit: 'fill', pointerEvents: 'none' }}
          />
        )}
        {splitPanes && split && activeTab && (
          <SplitChrome split={split} activeId={activeTab.id} panes={splitPanes} areaWidth={areaSize.w} />
        )}
        {isWebPage && splitDrop && (
          <div className="split-drop" aria-hidden="true">
            <div className={`split-drop-target ${splitDrop.left ? 'left' : 'right'}`}>
              <div className="split-drop-card">
                {splitDrop.favicon ? <img src={splitDrop.favicon} alt="" width={16} height={16} draggable={false} /> : <span className="split-drop-dot" />}
                <span className="split-drop-title">{splitDrop.title}</span>
                <span className="split-drop-side">Opens on the {splitDrop.left ? 'left' : 'right'}</span>
              </div>
            </div>
          </div>
        )}
      </div>

      {isNewTab && (
        <Homescreen
          onNavigate={onNavigate}
          onOpenCommandBar={onOpenCommandBar}
          widgets={homeWidgets}
          onUpdateWidgets={onUpdateHomeWidgets}
          background={homeBackground}
          uiScale={uiScale}
          dockVisible={dockVisible}
          userName={userName}
          searchBox={homeSearchBox}
          searchEngine={searchEngine}
        />
      )}
    </div>
  );
};
