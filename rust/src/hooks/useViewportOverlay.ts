// Registry of UI layers that cover the web page area.
//
// Tab pages are native WebView2 views that always paint above the HTML UI.
// While any overlay (modal, popover, context menu, hover card) is registered,
// BrowserViewport replaces the live page with a snapshot and hides the native
// view so the overlay is visible and clickable.
import { useEffect, useLayoutEffect, useState, useSyncExternalStore } from 'react';

let count = 0;
const listeners = new Set<() => void>();

function emit() {
  for (const l of listeners) l();
}

/** Registers an overlay; returns its release function (idempotent). */
export function acquireViewportOverlay(): () => void {
  count += 1;
  emit();
  let released = false;
  return () => {
    if (released) return;
    released = true;
    count = Math.max(0, count - 1);
    emit();
  };
}

/** Covers the page while `active` is true. */
export function useViewportOverlay(active: boolean): void {
  useEffect(() => {
    if (!active) return;
    return acquireViewportOverlay();
  }, [active]);
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Number of overlays currently covering the page. */
export function useViewportOverlayCount(): number {
  return useSyncExternalStore(subscribe, () => count);
}

// Whether the page is currently covered: its snapshot is painted and the
// native view hidden (always true when no web page is showing). Overlays wait
// for this before animating in, so they never slide in under the live page.
let covered = true;
const coveredListeners = new Set<() => void>();

export function setViewportCovered(next: boolean): void {
  if (covered === next) return;
  covered = next;
  for (const l of coveredListeners) l();
}

export function useViewportCovered(): boolean {
  return useSyncExternalStore(
    (l) => {
      coveredListeners.add(l);
      return () => {
        coveredListeners.delete(l);
      };
    },
    () => covered,
  );
}

// Pre-capture: the page snapshot takes ~100 ms, so controls that open an
// overlay start one when the pointer reaches them; the freeze then reuses it.
let prewarmHandler: (() => void) | null = null;

export function setViewportPrewarmHandler(fn: (() => void) | null): void {
  prewarmHandler = fn;
}

/** Captures the page ahead of an overlay that is about to open. */
export function prewarmViewport(): void {
  prewarmHandler?.();
}

/** True while `flag` is true and for `ms` after it turns false (lets exit animations finish over the snapshot). */
export function useLingering(flag: boolean, ms: number): boolean {
  const [lingering, setLingering] = useState(flag);
  useEffect(() => {
    if (flag) {
      setLingering(true);
      return;
    }
    const t = window.setTimeout(() => setLingering(false), ms);
    return () => window.clearTimeout(t);
  }, [flag, ms]);
  return flag || lingering;
}

// Space reserved at the bottom of the page area for UI that must stay
// visible next to a live page (the media mini player).
let reservedBottom = 0;
const reserveListeners = new Set<() => void>();

export function useReservePageBottom(px: number): void {
  useEffect(() => {
    if (px <= 0) return;
    reservedBottom = px;
    for (const l of reserveListeners) l();
    return () => {
      reservedBottom = 0;
      for (const l of reserveListeners) l();
    };
  }, [px]);
}

export function useReservedPageBottom(): number {
  return useSyncExternalStore(
    (l) => {
      reserveListeners.add(l);
      return () => {
        reserveListeners.delete(l);
      };
    },
    () => reservedBottom,
  );
}

// ---------------------------------------------------------------------------
// Holes: UI that shows through the live page without freezing it. The native
// page view gets a window region with these rectangles cut out, so it keeps
// running and stays clickable everywhere else, and nothing waits on a capture.

/** `[x, y, width, height]` in CSS px, window coordinates. */
export type HoleRect = [number, number, number, number];

export interface HoleOptions {
  /** Animate from this rectangle (or from the hole's current one, if it exists). */
  from?: HoleRect;
  ms?: number;
  /** CSS cubic-bezier control points, matching the UI's own transition. */
  ease?: [number, number, number, number];
  /** Drop the hole once `rect` is reached. */
  remove?: boolean;
  /** Run ahead (+) or behind (-) the UI animation, in ms. */
  lead?: number;
  /** Corner radius in CSS px. */
  radius?: number;
}

/** Resolves once the backend has the hole (or right away without a backend). */
export function setViewportHole(id: string, rect: HoleRect, opts?: HoleOptions): Promise<void> {
  const p = window.serica?.setViewportHole?.(id, rect, opts);
  return p ? p.catch(() => undefined) : Promise.resolve();
}

export function removeViewportHole(id: string): void {
  void setViewportHole(id, [0, 0, 0, 0], { remove: true });
}

/**
 * Cuts `el`'s layout box (untransformed, so entry animations don't shrink it)
 * out of the page while it is mounted. Returns whether the cut is in place:
 * keep the element invisible until then, or it would flash under the page.
 */
export function useElementHole(id: string, el: HTMLElement | null, pad = 0): boolean {
  const [ready, setReady] = useState(false);
  useLayoutEffect(() => {
    setReady(false);
    if (!el) return;
    let alive = true;
    const radius = parseFloat(getComputedStyle(el).borderTopLeftRadius) || 0;
    const push = () =>
      setViewportHole(id, [el.offsetLeft - pad, el.offsetTop - pad, el.offsetWidth + pad * 2, el.offsetHeight + pad * 2], {
        radius: radius + pad,
      });
    void push().then(() => {
      if (alive) setReady(true);
    });
    const ro = new ResizeObserver(() => void push());
    ro.observe(el);
    return () => {
      alive = false;
      ro.disconnect();
      removeViewportHole(id);
    };
  }, [id, el, pad]);
  return ready;
}
