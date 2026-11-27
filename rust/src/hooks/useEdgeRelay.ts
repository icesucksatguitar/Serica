// Web pages run to the window's edges. The sidebar, the pins dock and the
// auto-hidden top bar open when the pointer reaches an edge, but over a page
// the UI sees no pointer events (the page is a separate native view). They
// used to rely on thin strips the page left uncovered, which showed as gaps
// at the window's sides and corners.
//
// Instead: the backend reports the pointer heading for an edge
// (`serica:edge-approach`); from then on the OS cursor is read
// (`ui_cursor`, which never waits on the main thread) until the pointer
// leaves the edge area, and while it touches an edge the UI gets the same
// pointer events the strips used to give it.
import { useEffect } from 'react';
import { SIDEBAR_EDGE_PX } from '../shared/layout';
import { TOP_BAR_REVEAL_STRIP } from './useTopBarAutoHide';

/** Width of the dock's reveal zone at the right edge (.right-pins-edge-hover-zone). */
const DOCK_EDGE_PX = 6;
/** Keep reading the cursor while it is this close to an edge (CSS px). */
const SIDE_BAND = 220;
const TOP_BAND = 120;

/** The element the relayed pointer is over, so leaving it is reported too. */
let over: Element | null = null;

function relay(x: number, y: number): void {
  const target = document.elementFromPoint(x, y) ?? document.documentElement;
  const init: MouseEventInit = { bubbles: true, cancelable: true, clientX: x, clientY: y, view: window };
  // mouseout/mouseover pairs as a real pointer makes them: React derives
  // onMouseEnter/onMouseLeave from these (edge zones open, the sidebar sees
  // the pointer leave it).
  if (target !== over) {
    if (over && over.isConnected) over.dispatchEvent(new MouseEvent('mouseout', { ...init, relatedTarget: target }));
    target.dispatchEvent(new MouseEvent('mouseover', { ...init, relatedTarget: over }));
    over = target;
  }
  target.dispatchEvent(new MouseEvent('mousemove', init));
}

export function useEdgeRelay(enabled: boolean): void {
  useEffect(() => {
    const api = window.serica;
    if (!enabled || !api?.onEdgeApproach || !api.uiCursor) return;
    let alive = true;
    let polling = false;
    let until = 0;
    // After touching an edge the pointer is followed (opened panels hide
    // once it moves away over the page) until it leaves the edge area.
    let engaged = false;
    let last = '';
    const poll = async () => {
      if (!alive) return;
      const c = await api.uiCursor!().catch(() => null);
      const now = performance.now();
      if (c) {
        const dpr = window.devicePixelRatio || 1;
        const x = c[0] / dpr;
        const y = c[1] / dpr;
        const w = window.innerWidth;
        const atEdge = x <= SIDEBAR_EDGE_PX || x >= w - DOCK_EDGE_PX || y <= TOP_BAR_REVEAL_STRIP;
        if (atEdge) engaged = true;
        const key = `${Math.round(x)},${Math.round(y)}`;
        if (engaged && key !== last) relay(x, y);
        last = key;
        if (x < SIDE_BAND || x > w - SIDE_BAND || y < TOP_BAND) until = now + 400;
      }
      if (!alive || now > until) {
        polling = false;
        engaged = false;
        last = '';
        over = null;
        return;
      }
      window.setTimeout(() => void poll(), 16);
    };
    const off = api.onEdgeApproach(() => {
      until = performance.now() + 600;
      if (!polling) {
        polling = true;
        void poll();
      }
    });
    return () => {
      alive = false;
      off?.();
    };
  }, [enabled]);
}
