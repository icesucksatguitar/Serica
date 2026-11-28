// The window's macOS chrome, as the UI sees it.
//
// - Drag regions. The Windows build's engine honours `-webkit-app-region:
//   drag`; WebKit on the Mac ignores it, so a press in one of Serica's drag
//   regions asks the backend to move the window, and a double-click there
//   does what the user chose in System Settings (zoom or minimize), as on any
//   Mac window.
// - Full screen. The traffic lights live in the menu bar then, so the top bar
//   needs no room for them (`html[data-fullscreen]`).
// - Traffic lights. They sit at the top left, over the top bar's row; with an
//   auto-hidden top bar they hide with it, so they never cover a page.
import { useEffect, useState } from 'react';

/** Where the window can be dragged (the regions the Windows build marks `-webkit-app-region: drag`). */
const DRAG_REGIONS = '.macos-window-drag-bar, .nsb-header, .nsb-chrome-row, .dwin-bar';

/** What stays clickable inside a drag region (`-webkit-app-region: no-drag`). */
const NO_DRAG = [
  'button',
  'a[href]',
  'input',
  'textarea',
  'select',
  'label',
  '[contenteditable]:not([contenteditable="false"])',
  '[role="button"]',
  '[role="menuitem"]',
  '[role="tab"]',
  '[draggable="true"]',
  '.top-floating-url-pill',
  '.top-edge-hover-zone',
  '.minimal-top-right-cluster',
  '.macos-window-controls-cluster',
  '.site-nav-controls',
  '.site-nav-btn',
  '.topbar-new-tab-btn',
  '.dwin-address',
].join(', ');

/** Installs the drag-region handling; returns its removal. */
export function installDragRegions(): () => void {
  const onDown = (e: MouseEvent) => {
    if (e.button !== 0 || e.defaultPrevented || e.ctrlKey) return;
    const target = e.target instanceof Element ? e.target : null;
    const region = target?.closest(DRAG_REGIONS);
    if (!target || !region) return;
    // A control inside the region keeps its clicks.
    const control = target.closest(NO_DRAG);
    if (control && region.contains(control)) return;
    e.preventDefault();
    const action = e.detail === 2 ? 'titlebar-double-click' : e.detail === 1 ? 'drag' : null;
    if (action) void window.serica?.windowControl?.(action)?.catch?.(() => {});
  };
  window.addEventListener('mousedown', onDown, true);
  return () => window.removeEventListener('mousedown', onDown, true);
}

export interface MacWindowState {
  fullscreen: boolean;
  /** Right edge of the traffic lights, in CSS px from the window's left edge. */
  trafficLightsRight: number;
}

/** Standard traffic lights (x 14, three 14px buttons 20px apart). */
const DEFAULT_LIGHTS_RIGHT = 68;

/** Full screen and the traffic lights' extent; sets `html[data-fullscreen]`. */
export function useMacWindowState(): MacWindowState {
  const [state, setState] = useState<MacWindowState>({ fullscreen: false, trafficLightsRight: DEFAULT_LIGHTS_RIGHT });
  useEffect(() => {
    let alive = true;
    const read = () =>
      void window.serica
        ?.getWindowState?.()
        .then((s: { fullscreen?: boolean; trafficLights?: { right?: number; windowWidth?: number } | null } | undefined) => {
          if (!alive || !s) return;
          // Points to CSS px (the UI zoom setting scales the page).
          const w = s.trafficLights?.windowWidth;
          const zoom = typeof w === 'number' && w > 0 && window.innerWidth > 0 ? w / window.innerWidth : 1;
          const right = (s.trafficLights?.right ?? 0) / (zoom > 0.2 && zoom < 5 ? zoom : 1);
          setState({
            fullscreen: !!s.fullscreen,
            trafficLightsRight: right > 0 && right < 200 ? Math.round(right) : DEFAULT_LIGHTS_RIGHT,
          });
        })
        .catch(() => {});
    read();
    // Full screen changes move the buttons (and the UI zoom scales CSS px).
    const off = window.serica?.onWindowState?.(() => read());
    window.addEventListener('resize', read);
    return () => {
      alive = false;
      off?.();
      window.removeEventListener('resize', read);
    };
  }, []);
  useEffect(() => {
    document.documentElement.dataset.fullscreen = state.fullscreen ? 'true' : 'false';
  }, [state.fullscreen]);
  return state;
}

/** Whether the sidebar is out (it sets `html.sidebar-covering`). */
export function useSidebarCovering(): boolean {
  const [covering, setCovering] = useState(() => document.documentElement.classList.contains('sidebar-covering'));
  useEffect(() => {
    const root = document.documentElement;
    const mo = new MutationObserver(() => setCovering(root.classList.contains('sidebar-covering')));
    mo.observe(root, { attributes: true, attributeFilter: ['class'] });
    return () => mo.disconnect();
  }, []);
  return covering;
}

/** Shows or hides the traffic lights. */
export function useTrafficLights(visible: boolean): void {
  useEffect(() => {
    void window.serica?.setTrafficLights?.(visible)?.catch?.(() => {});
  }, [visible]);
}
