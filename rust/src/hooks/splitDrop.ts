// A tab dragged from the sidebar over the page: which half it would land in
// (split view). The sidebar's drag sets it; the page area draws the zones.
import { useSyncExternalStore } from 'react';

export interface SplitDropHint {
  /** The dragged tab would go on the left of the page's tab. */
  left: boolean;
  /** The dragged tab, shown in the half it would land in. */
  title?: string;
  favicon?: string;
}

let hint: SplitDropHint | null = null;
const listeners = new Set<() => void>();

export function setSplitDropHint(next: SplitDropHint | null): void {
  if (hint === next || (hint && next && hint.left === next.left && hint.title === next.title)) return;
  hint = next;
  // The sidebar's drag chip hides while the page shows where the tab goes.
  document.body.classList.toggle('split-dropping', !!next);
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
