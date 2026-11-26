// Registry of UI layers that cover the web page area.
//
// Tab pages are native WebView2 views that always paint above the HTML UI.
// While any overlay (modal, popover, context menu, hover card) is registered,
// BrowserViewport replaces the live page with a snapshot and hides the native
// view so the overlay is visible and clickable.
import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from 'react';

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
// The handler resolves true once a capture of the current page is ready for
// the freeze (or returns nothing when no capture is needed).
let prewarmHandler: (() => Promise<boolean> | void) | null = null;

export function setViewportPrewarmHandler(fn: (() => Promise<boolean> | void) | null): void {
  prewarmHandler = fn;
}

/** Captures the page ahead of an overlay that is about to open. */
export function prewarmViewport(): void {
  void prewarmHandler?.();
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

// Underlay: a capture of the live page drawn beneath it, in the page's own
// place, for the part a panel is about to cover. Cutting a hole there then
// changes nothing on screen, and the panel slides in over the capture
// smoothly, however busy the app's main thread is.
//
// Opening must not wait for a capture: one takes 100-300 ms (more on a busy
// page). So the page is captured ahead of time: once it has loaded, after
// each scroll settles (`serica:page-settled`), after a tab switch, and when
// the pointer heads for an edge (`serica:edge-approach`). Opening uses the
// latest capture of the page as it is; if the page changed since, a fresher
// capture replaces it should one arrive during the slide. Only the strip under the panel is drawn (a
// small cropped bitmap), not the whole page, so showing it costs little.

/** Where the page area is (window CSS px) and which page it shows. */
interface PageGeometry {
  key: string;
  left: number;
  top: number;
  width: number;
  height: number;
}
let page: PageGeometry = { key: '', left: 0, top: 0, width: 0, height: 0 };

/** BrowserViewport reports the page on screen and its rectangle. */
export function setUnderlayPage(next: PageGeometry): void {
  if (next.key !== page.key) {
    dropCapture();
    pageChangedAt = performance.now();
  }
  page = next;
}

/** When the page last changed (scrolled, loaded): older captures are stale. */
let pageChangedAt = 0;

/** The page on screen changed (scroll settled, load finished): capture it again. */
export function refreshPageCapture(): void {
  pageChangedAt = performance.now();
  void capturePage();
}

export interface Underlay {
  id: string;
  /** Window CSS px. */
  rect: HoleRect;
  bitmap: ImageBitmap;
}

let underlays: readonly Underlay[] = [];
const underlayListeners = new Set<() => void>();

function publish(next: readonly Underlay[]) {
  underlays = next;
  underlayListeners.forEach((l) => l());
}

/** Removes the capture drawn under one panel (`null` id: all of them). */
export function setPageUnderlay(id: string | null): void {
  const keep = id === null ? [] : underlays.filter((u) => u.id !== id);
  if (keep.length !== underlays.length) publish(keep);
}

export function usePageUnderlays(): readonly Underlay[] {
  return useSyncExternalStore(
    (cb) => {
      underlayListeners.add(cb);
      return () => underlayListeners.delete(cb);
    },
    () => underlays,
  );
}

const wait = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
const twoFrames = () => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));

interface Capture {
  key: string;
  at: number;
  /** When the capture was asked for (it shows the page as it was then). */
  startedAt: number;
  /** Decoded once; panels crop their strip from it. */
  bitmap: ImageBitmap;
}

// One page capture at a time, shared; the latest is kept for reuse.
let inflight: Promise<Capture | null> | null = null;
let inflightAt = 0;
let lastCapture: Capture | null = null;
let releaseTimer = 0;

function dropCapture() {
  lastCapture?.bitmap.close();
  lastCapture = null;
}

async function decodeCapture(dataUrl: string): Promise<ImageBitmap | null> {
  try {
    const blob = await (await fetch(dataUrl)).blob();
    return await createImageBitmap(blob);
  } catch {
    return null;
  }
}

function capturePage(): Promise<Capture | null> {
  // One in flight that started after the page last changed will do. A stuck
  // one (a capture can stall while the page is hidden) is given up on.
  if (inflight && inflightAt >= pageChangedAt && performance.now() - inflightAt < 3000) return inflight;
  const p = window.serica?.captureViewport?.();
  if (!p) return Promise.resolve(null);
  const key = page.key;
  const startedAt = performance.now();
  inflightAt = startedAt;
  inflight = p
    .catch(() => null)
    .then(async (img): Promise<Capture | null> => {
      if (!img || key !== page.key) return null;
      const bitmap = await decodeCapture(img);
      if (!bitmap) return null;
      if (key !== page.key) {
        bitmap.close();
        return null;
      }
      // A later capture may have finished first.
      if (lastCapture && lastCapture.key === key && lastCapture.startedAt > startedAt) {
        bitmap.close();
        return lastCapture;
      }
      lastCapture?.bitmap.close();
      lastCapture = { key, at: performance.now(), startedAt, bitmap };
      // The decoded page is large: let it go once it is too old to use.
      window.clearTimeout(releaseTimer);
      releaseTimer = window.setTimeout(dropCapture, STALE_MS);
      return lastCapture;
    })
  const mine = inflight;
  void mine.finally(() => {
    if (inflight === mine) inflight = null;
  });
  return mine;
}

/** A capture this recent is used as is, even if the page changed since. */
const FRESH_MS = 400;
/** Older ones (same page) are used while a fresh one is taken. */
const STALE_MS = 60000;

function captureAge(): number {
  return lastCapture && lastCapture.key === page.key ? performance.now() - lastCapture.at : Infinity;
}

/** A capture of the page as it is now (taken after its last change). */
function currentCapture(): Capture | null {
  if (!lastCapture || lastCapture.key !== page.key) return null;
  return lastCapture.startedAt >= pageChangedAt || captureAge() < FRESH_MS ? lastCapture : null;
}

/** Starts a page capture ahead of UI that is about to slide over the page. */
export function prewarmPageUnderlay(): void {
  if (currentCapture()) return;
  void capturePage();
}

async function crop(cap: Capture, rect: HoleRect): Promise<ImageBitmap | null> {
  const g = page;
  if (g.width <= 0 || g.height <= 0) return null;
  const kx = cap.bitmap.width / g.width;
  const ky = cap.bitmap.height / g.height;
  const x = Math.max(0, rect[0] - g.left);
  const y = Math.max(0, rect[1] - g.top);
  const w = Math.min(g.width - x, rect[2] - Math.max(0, g.left - rect[0]));
  const h = Math.min(g.height - y, rect[3] - Math.max(0, g.top - rect[1]));
  if (w <= 0 || h <= 0) return null;
  try {
    return await createImageBitmap(cap.bitmap, Math.round(x * kx), Math.round(y * ky), Math.max(1, Math.round(w * kx)), Math.max(1, Math.round(h * ky)));
  } catch {
    return null;
  }
}

async function show(id: string, rect: HoleRect, cap: Capture): Promise<boolean> {
  const bitmap = await crop(cap, rect);
  if (!bitmap) return false;
  const g = page;
  // In window px: clamp to the page area.
  const x = Math.max(rect[0], g.left);
  const y = Math.max(rect[1], g.top);
  const placed: HoleRect = [x, y, Math.min(rect[0] + rect[2], g.left + g.width) - x, Math.min(rect[1] + rect[3], g.top + g.height) - y];
  underlays.find((u) => u.id === id)?.bitmap.close();
  publish([...underlays.filter((u) => u.id !== id), { id, rect: placed, bitmap }]);
  return true;
}

/**
 * Draws a capture of the live page beneath the part of it `rect` (window CSS
 * px) is about to cover, for the panel `id`. Resolves true once painted, false
 * when there is no capture of this page to show (the panel then slides over
 * the page area's plain background). Never waits long for a capture.
 */
export async function preparePageUnderlay(id: string, rect: HoleRect): Promise<boolean> {
  const key = page.key;
  let cap: Capture | null = currentCapture();
  let stale = false;
  if (!cap) {
    // One in flight for a while (the pointer neared the edge) is nearly done.
    const pending = inflight !== null && inflightAt >= pageChangedAt && performance.now() - inflightAt > 60;
    const p = capturePage();
    const limit = captureAge() < STALE_MS ? (pending ? 70 : 30) : 140;
    cap = await Promise.race([p, wait(limit).then(() => null)]);
    if (!cap && captureAge() < STALE_MS) {
      cap = lastCapture;
      stale = true;
    }
  }
  if (!cap || key !== page.key) return false;
  if (!(await show(id, rect, cap))) return false;
  await twoFrames();
  if (stale) {
    // A fresher one, if it comes while the panel is still sliding.
    void capturePage().then((fresh) => {
      if (fresh && fresh.key === page.key && underlays.some((u) => u.id === id)) void show(id, rect, fresh);
    });
  }
  return true;
}

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
export function useElementHole(id: string, el: HTMLElement | null, pad = 0, position?: string): boolean {
  const [ready, setReady] = useState(false);
  const pushRef = useRef<(() => Promise<unknown>) | null>(null);
  useLayoutEffect(() => {
    setReady(false);
    if (!el) return;
    let alive = true;
    const radius = parseFloat(getComputedStyle(el).borderTopLeftRadius) || 0;
    const push = () =>
      setViewportHole(id, [el.offsetLeft - pad, el.offsetTop - pad, el.offsetWidth + pad * 2, el.offsetHeight + pad * 2], {
        radius: radius + pad,
      });
    pushRef.current = push;
    void push().then(() => {
      if (alive) setReady(true);
    });
    const ro = new ResizeObserver(() => void push());
    ro.observe(el);
    return () => {
      alive = false;
      pushRef.current = null;
      ro.disconnect();
      removeViewportHole(id);
    };
  }, [id, el, pad]);
  // An element placed after it mounts (measured off screen first, then moved
  // beside its target) changes position without changing size, which the
  // ResizeObserver never reports: pass its position here so the hole follows.
  useLayoutEffect(() => {
    if (position !== undefined) void pushRef.current?.();
  }, [position]);
  return ready;
}
