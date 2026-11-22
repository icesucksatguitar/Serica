// Registry of UI layers that cover the web page area.
//
// Tab pages are native WebView2 views that always paint above the HTML UI.
// While any overlay (modal, popover, context menu, hover card) is registered,
// BrowserViewport replaces the live page with a snapshot and hides the native
// view so the overlay is visible and clickable.
import { useEffect, useSyncExternalStore } from 'react';

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
