// A tab dragged from the sidebar over the page: which half it would land in
// (split view). The sidebar's drag sets it; the page area draws the zones.
import { useSyncExternalStore } from 'react';

export interface SplitDropHint {
  /** The dragged tab would go on the left of the page's tab. */
  left: boolean;
}

let hint: SplitDropHint | null = null;
const listeners = new Set<() => void>();

export function setSplitDropHint(next: SplitDropHint | null): void {
  if (hint === next || (hint && next && hint.left === next.left)) return;
  hint = next;
  listeners.forEach((l) => l());
}

export function useSplitDropHint(): SplitDropHint | null {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => hint,
  );
}
