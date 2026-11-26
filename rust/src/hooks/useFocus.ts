import { useEffect, useState, useSyncExternalStore } from 'react';
import type { FocusView } from '../adapter/tauriBridge';

// One Focus Mode state for the whole UI (settings page, top bar, widget),
// pushed by the backend whenever a list starts, ends or changes.

let view: FocusView | null = null;
/** performance.now() when `view` arrived: remaining times count down from it. */
let receivedAt = 0;
const listeners = new Set<() => void>();
let started = false;

function set(next: FocusView) {
  view = next;
  receivedAt = performance.now();
  listeners.forEach((l) => l());
}

function start() {
  if (started) return;
  started = true;
  void window.serica?.focusGet?.().then((v: FocusView) => v && set(v));
  window.serica?.onFocusUpdated?.((v: FocusView) => set(v));
}

export function setFocusView(v: FocusView) {
  set(v);
}

export function useFocusView(): FocusView | null {
  start();
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => view,
  );
}

/** Time left in ms for an active entry, counting down locally between updates. */
export function remainingNow(remainingMs: number | null): number | null {
  if (remainingMs === null) return null;
  return Math.max(0, remainingMs - (performance.now() - receivedAt));
}

/** Re-renders every `ms` while `on` (for countdowns). */
export function useTicker(on: boolean, ms = 1000): number {
  const [n, setN] = useState(0);
  useEffect(() => {
    if (!on) return;
    const t = window.setInterval(() => setN((x) => x + 1), ms);
    return () => window.clearInterval(t);
  }, [on, ms]);
  return n;
}

export function formatRemaining(ms: number | null): string {
  if (ms === null) return '∞';
  const s = Math.ceil(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const x = s % 60;
  return h ? `${h}:${String(m).padStart(2, '0')}:${String(x).padStart(2, '0')}` : `${m}:${String(x).padStart(2, '0')}`;
}

/** Messages for errors the backend returns. */
export function focusError(e: unknown): string {
  const code = typeof e === 'string' ? e : (e as any)?.message ?? '';
  switch (code) {
    case 'locked':
      return 'This list is locked until its time is up.';
    case 'lock-needs-time':
      return 'A locked session needs a length.';
    case 'too-many':
      return 'You have the most lists Serica keeps. Remove one first.';
    case 'no-such-list':
      return 'That list no longer exists.';
    default:
      return 'Focus Mode could not do that.';
  }
}
