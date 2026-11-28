// Auto-hiding top bar on web pages. The page fills the window except a thin
// strip at the top edge; the pointer reaching that strip slides the bar down
// over the page, and leaving the bar slides it back up.
//
// Pages are native views drawn above the UI, so the bar is shown the way the
// auto-hide sidebar is: a capture of the page goes beneath it, the bar's area
// is cut out of the page in one step (nothing visibly changes), the bar slides
// in over the capture, and the capture is dropped once the bar covers it.
// Hiding runs the same steps backwards.
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { preparePageUnderlay, prewarmPageUnderlay, removeViewportHole, setPageUnderlay, setViewportHole } from './useViewportOverlay';

/** Height of the top bar (CSS px). Keep in step with `--topbar-h` users in CSS. */
export const TOP_BAR_HEIGHT = 35;
/** Strip at the top edge left uncovered by the page, where the pointer reveals the bar. */
export const TOP_BAR_REVEAL_STRIP = 6;

const SLIDE_MS = 200;
const HIDE_DELAY_MS = 260;

const sleepMs = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

export interface TopBarAutoHide {
  /** Whether the bar is on screen (always true when auto-hide is off). */
  shown: boolean;
  /** Pointer handler for the reveal strip at the top edge. */
  revealNow: () => void;
}

/**
 * @param enabled auto-hide applies (a web page is showing and the setting is on)
 * @param pinned keep the bar out (a menu from it is open, an overlay covers the page)
 */
export function useTopBarAutoHide(enabled: boolean, pinned: boolean): TopBarAutoHide {
  const [revealed, setRevealed] = useState(false);
  const revealedRef = useRef(false);
  revealedRef.current = revealed;
  const [shownState, setShownState] = useState(false);
  const hideTimer = useRef<number | null>(null);

  const cancelHide = useCallback(() => {
    if (hideTimer.current !== null) {
      window.clearTimeout(hideTimer.current);
      hideTimer.current = null;
    }
  }, []);

  const revealNow = useCallback(() => {
    cancelHide();
    setRevealed(true);
  }, [cancelHide]);

  const scheduleHide = useCallback(() => {
    cancelHide();
    // The slide out runs over a capture of the page: take it meanwhile.
    prewarmPageUnderlay();
    hideTimer.current = window.setTimeout(() => {
      hideTimer.current = null;
      setRevealed(false);
    }, HIDE_DELAY_MS);
  }, [cancelHide]);

  // The bar stays while the pointer is on it. Moving below it, onto the page
  // (a separate native view: the UI sees the pointer leave) or out of the
  // window hides it. Empty parts of the bar drag the window and report no
  // pointer moves, which keeps it shown too.
  useEffect(() => {
    if (!enabled) return;
    // Acts on changes only: every scheduled hide starts a page capture, so
    // it must not run again for each pointer event below the bar.
    const onMove = (e: MouseEvent) => {
      if (e.clientY <= TOP_BAR_HEIGHT + 2) {
        if (e.clientY <= TOP_BAR_REVEAL_STRIP && !revealedRef.current) revealNow();
        else if (hideTimer.current !== null) cancelHide();
      } else if (revealedRef.current && hideTimer.current === null) {
        scheduleHide();
      }
    };
    const onLeave = () => scheduleHide();
    window.addEventListener('mousemove', onMove, { passive: true });
    document.documentElement.addEventListener('mouseleave', onLeave);
    window.addEventListener('blur', onLeave);
    return () => {
      window.removeEventListener('mousemove', onMove);
      document.documentElement.removeEventListener('mouseleave', onLeave);
      window.removeEventListener('blur', onLeave);
    };
  }, [enabled, scheduleHide, cancelHide, revealNow]);

  useEffect(() => () => cancelHide(), [cancelHide]);

  const want = enabled && (revealed || pinned);
  const holeShown = useRef(false);
  const seq = useRef(0);
  useLayoutEffect(() => {
    const mine = ++seq.current;
    const current = () => mine === seq.current;
    const rect: [number, number, number, number] = [0, 0, 100000, TOP_BAR_HEIGHT + 1];
    if (!enabled) {
      if (holeShown.current) {
        holeShown.current = false;
        removeViewportHole('topbar');
      }
      setShownState(false);
      return;
    }
    if (want && !holeShown.current) {
      holeShown.current = true;
      void (async () => {
        await preparePageUnderlay('topbar', rect);
        if (!current()) return;
        await setViewportHole('topbar', rect);
        if (!current()) return;
        setShownState(true); // slides in over the capture
        await sleepMs(SLIDE_MS + 40);
        if (current()) setPageUnderlay('topbar');
      })();
    } else if (!want && holeShown.current) {
      holeShown.current = false;
      void (async () => {
        await preparePageUnderlay('topbar', rect);
        if (!current()) return;
        setShownState(false); // slides out over the capture
        await sleepMs(SLIDE_MS + 20);
        if (!current()) return;
        await setViewportHole('topbar', [0, 0, 0, 0], { remove: true });
        if (current()) setPageUnderlay('topbar');
      })();
    }
  }, [enabled, want]);

  useEffect(
    () => () => {
      removeViewportHole('topbar');
    },
    [],
  );

  return {
    shown: !enabled || shownState,
    revealNow,
  };
}
