import React, { useEffect, useState } from 'react';
import { useViewportOverlay } from './useViewportOverlay';

/**
 * Mount state for something that animates in and out.
 *
 * - `mounted` stays true until the exit has played (`exitMs`), so the element
 *   can leave instead of vanishing.
 * - `state` is "closed" on the first frame after mounting and while leaving,
 *   "open" otherwise: CSS transitions between the two (see foundation.css).
 */
export function usePresence(open: boolean, exitMs = 160): { mounted: boolean; state: 'open' | 'closed' } {
  const [mounted, setMounted] = useState(open);
  const [shown, setShown] = useState(false);

  useEffect(() => {
    if (open) {
      setMounted(true);
      // Two frames: the closed state must be painted once before flipping,
      // or the browser skips the transition.
      let r2 = 0;
      const r1 = requestAnimationFrame(() => {
        r2 = requestAnimationFrame(() => setShown(true));
      });
      return () => {
        cancelAnimationFrame(r1);
        cancelAnimationFrame(r2);
      };
    }
    setShown(false);
    const t = window.setTimeout(() => setMounted(false), exitMs);
    return () => window.clearTimeout(t);
  }, [open, exitMs]);

  return { mounted: mounted || open, state: open && shown ? 'open' : 'closed' };
}

/**
 * Renders `children` while open and during their exit, inside a box-less
 * wrapper carrying `data-state`. With `coversPage`, the live page stays
 * frozen under the overlay until the exit has finished.
 */
export const Presence: React.FC<{
  open: boolean;
  exitMs?: number;
  coversPage?: boolean;
  children: React.ReactNode;
}> = ({ open, exitMs = 160, coversPage = false, children }) => {
  const { mounted, state } = usePresence(open, exitMs);
  useViewportOverlay(coversPage && mounted);
  if (!mounted) return null;
  return React.createElement('div', { className: 'presence', 'data-state': state }, children);
};
