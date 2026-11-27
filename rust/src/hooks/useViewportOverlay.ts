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
// page). So pages are captured ahead of time: once loaded, after each scroll
// settles (`serica:page-settled`), after a tab switch, and when the pointer
// heads for an edge (`serica:edge-approach`). Opening uses the latest
// capture as it is; if the page changed since, a fresher capture replaces it
// should one arrive during the slide. Only the strip under the panel is
// drawn (a small cropped bitmap), not the whole page, so showing it costs
// little. A split view has two pages (panes), each captured on its own.

/** A page on screen (window CSS px) and which page it shows. */
export interface UnderlayPane {
  tabId: string;
  /** Tab and URL: a capture of another page is never used. */
  key: string;
  left: number;
  top: number;
  width: number;
  height: number;
}

interface Capture {
  key: string;
  at: number;
  /** When the capture was asked for (it shows the page as it was then). */
  startedAt: number;
  /** Decoded once; panels crop their strip from it. */
  bitmap: ImageBitmap;
}

let panes: readonly UnderlayPane[] = [];
const captures = new Map<string, Capture>();
const inflight = new Map<string, { p: Promise<Capture | null>; at: number }>();
/** When each page last changed (scrolled, loaded): older captures are stale. */
const changedAt = new Map<string, number>();
const releaseTimers = new Map<string, number>();

/** A capture this recent is used as is, even if the page changed since. */
const FRESH_MS = 400;
/** Older ones (same page) are used while a fresh one is taken. */
const STALE_MS = 60000;

function dropCapture(tabId: string) {
  captures.get(tabId)?.bitmap.close();
  captures.delete(tabId);
}

/** BrowserViewport reports the pages on screen and where they are. */
export function setUnderlayPanes(next: readonly UnderlayPane[]): void {
  const now = performance.now();
  for (const old of panes) {
    const still = next.find((p) => p.tabId === old.tabId);
    if (!still || still.key !== old.key) dropCapture(old.tabId);
  }
  for (const p of next) {
    const old = panes.find((o) => o.tabId === p.tabId);
    if (!old || old.key !== p.key) changedAt.set(p.tabId, now);
  }
  panes = next;
}

export interface Underlay {
  id: string;
  /** The panel it is drawn for (one entry per page it covers). */
  group: string;
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

/** Removes the captures drawn under one panel (`null`: all of them). */
export function setPageUnderlay(group: string | null): void {
  const keep = group === null ? [] : underlays.filter((u) => u.group !== group);
  if (keep.length === underlays.length) return;
  for (const u of underlays) if (!keep.includes(u)) u.bitmap.close();
  publish(keep);
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

async function decodeCapture(url: string): Promise<ImageBitmap | null> {
  try {
    const blob = await (await fetch(url)).blob();
    return await createImageBitmap(blob);
  } catch {
    return null;
  }
}

function paneOf(tabId: string): UnderlayPane | undefined {
  return panes.find((p) => p.tabId === tabId);
}

function capturePage(tabId: string): Promise<Capture | null> {
  // One in flight that started after the page last changed will do. A stuck
  // one (a capture can stall while the page is hidden) is given up on.
  const running = inflight.get(tabId);
  if (running && running.at >= (changedAt.get(tabId) ?? 0) && performance.now() - running.at < 3000) return running.p;
  const pane = paneOf(tabId);
  const call = pane && window.serica?.captureViewport?.(tabId);
  if (!pane || !call) return Promise.resolve(null);
  const key = pane.key;
  const startedAt = performance.now();
  const p = call
    .catch(() => null)
    .then(async (img): Promise<Capture | null> => {
      if (!img || paneOf(tabId)?.key !== key) return null;
      const bitmap = await decodeCapture(img);
      if (!bitmap) return null;
      if (paneOf(tabId)?.key !== key) {
        bitmap.close();
        return null;
      }
      // A later capture may have finished first.
      const last = captures.get(tabId);
      if (last && last.key === key && last.startedAt > startedAt) {
        bitmap.close();
        return last;
      }
      last?.bitmap.close();
      const cap = { key, at: performance.now(), startedAt, bitmap };
      captures.set(tabId, cap);
      // The decoded page is large: let it go once it is too old to use.
      window.clearTimeout(releaseTimers.get(tabId));
      releaseTimers.set(tabId, window.setTimeout(() => dropCapture(tabId), STALE_MS));
      return cap;
    });
  const entry = { p, at: startedAt };
  inflight.set(tabId, entry);
  void p.finally(() => {
    if (inflight.get(tabId) === entry) inflight.delete(tabId);
  });
  return p;
}

function captureAge(tabId: string): number {
  const c = captures.get(tabId);
  return c && c.key === paneOf(tabId)?.key ? performance.now() - c.at : Infinity;
}

/** A capture of the page as it is now (taken after its last change). */
function currentCapture(tabId: string): Capture | null {
  const c = captures.get(tabId);
  if (!c || c.key !== paneOf(tabId)?.key) return null;
  return c.startedAt >= (changedAt.get(tabId) ?? 0) || captureAge(tabId) < FRESH_MS ? c : null;
}

/** A page on screen changed (scroll settled, load finished): capture it again. */
export function refreshPageCapture(tabId?: string): void {
  const now = performance.now();
  for (const p of panes) {
    if (tabId && p.tabId !== tabId) continue;
    changedAt.set(p.tabId, now);
    void capturePage(p.tabId);
  }
}

/** Starts capturing the pages ahead of UI that is about to slide over them. */
export function prewarmPageUnderlay(): void {
  for (const p of panes) if (!currentCapture(p.tabId)) void capturePage(p.tabId);
}

function intersect(a: HoleRect, p: UnderlayPane): HoleRect | null {
  const x0 = Math.max(a[0], p.left);
  const y0 = Math.max(a[1], p.top);
  const x1 = Math.min(a[0] + a[2], p.left + p.width);
  const y1 = Math.min(a[1] + a[3], p.top + p.height);
  return x1 > x0 && y1 > y0 ? [x0, y0, x1 - x0, y1 - y0] : null;
}

async function crop(cap: Capture, pane: UnderlayPane, r: HoleRect): Promise<ImageBitmap | null> {
  if (pane.width <= 0 || pane.height <= 0) return null;
  const kx = cap.bitmap.width / pane.width;
  const ky = cap.bitmap.height / pane.height;
  try {
    return await createImageBitmap(
      cap.bitmap,
      Math.round((r[0] - pane.left) * kx),
      Math.round((r[1] - pane.top) * ky),
      Math.max(1, Math.round(r[2] * kx)),
      Math.max(1, Math.round(r[3] * ky)),
    );
  } catch {
    return null;
  }
}

async function show(group: string, pane: UnderlayPane, r: HoleRect, cap: Capture): Promise<boolean> {
  const bitmap = await crop(cap, pane, r);
  if (!bitmap) return false;
  if (paneOf(pane.tabId)?.key !== pane.key) {
    bitmap.close();
    return false;
  }
  const id = `${group}|${pane.tabId}`;
  underlays.find((u) => u.id === id)?.bitmap.close();
  publish([...underlays.filter((u) => u.id !== id), { id, group, rect: r, bitmap }]);
  return true;
}

/** One page's part of `preparePageUnderlay`. */
async function preparePane(group: string, pane: UnderlayPane, r: HoleRect): Promise<boolean> {
  const tabId = pane.tabId;
  let cap = currentCapture(tabId);
  let stale = false;
  if (!cap) {
    // One in flight for a while (the pointer neared the edge) is nearly done.
    const running = inflight.get(tabId);
    const pending = !!running && running.at >= (changedAt.get(tabId) ?? 0) && performance.now() - running.at > 60;
    const p = capturePage(tabId);
    const limit = captureAge(tabId) < STALE_MS ? (pending ? 70 : 30) : 140;
    cap = await Promise.race([p, wait(limit).then(() => null)]);
    if (!cap && captureAge(tabId) < STALE_MS) {
      cap = captures.get(tabId) ?? null;
      stale = true;
    }
  }
  if (!cap || !(await show(group, pane, r, cap))) return false;
  if (stale) {
    // A fresher one, if it comes while the panel is still sliding.
    void capturePage(tabId).then((fresh) => {
      if (fresh && underlays.some((u) => u.id === `${group}|${tabId}`)) void show(group, pane, r, fresh);
    });
  }
  return true;
}

/**
 * Draws captures of the live pages beneath the part of them `rect` (window
 * CSS px) is about to cover, for the panel `group`. Resolves true once
 * painted, false when there is no capture to show (the panel then slides
 * over the page area's plain background). Never waits long for a capture.
 */
export async function preparePageUnderlay(group: string, rect: HoleRect): Promise<boolean> {
  const jobs = panes.map((p) => [p, intersect(rect, p)] as const).filter((j): j is readonly [UnderlayPane, HoleRect] => j[1] !== null);
  if (!jobs.length) return false;
  const shown = await Promise.all(jobs.map(([p, r]) => preparePane(group, p, r)));
  if (!shown.some(Boolean)) return false;
  await twoFrames();
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
