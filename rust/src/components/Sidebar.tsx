import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import {
  ArrowRightLeft,
  BookOpen,
  Briefcase,
  Camera,
  ChevronRight,
  Compass,
  Code,
  Coffee,
  Copy,
  Edit2,
  Folder,
  FolderOpen,
  FolderPlus,
  Flame,
  Gamepad2,
  Globe,
  GraduationCap,
  Heart,
  Home,
  Leaf,
  MoreHorizontal,
  Music,
  PanelLeftClose,
  Pin,
  PinOff,
  Plus,
  Rocket,
  Sparkles,
  Star,
  Sun,
  StarOff,
  Trash2,
  X,
  type LucideIcon,
} from 'lucide-react';
import { SIDEBAR_EDGE_PX, sidebarVisibility } from '../shared/layout';
import { useSpaceSwipe } from '../hooks/useSpaceSwipe';
import { useSidebarDrag } from '../hooks/useSidebarDrag';
import { useHoverPreview } from '../hooks/useHoverPreview';
import { TabPreviewCard } from './TabPreviewCard';
import { SpaceMenu } from './SpaceMenu';
import { GuideTour, type GuideStep } from './GuideTour';
import { addSettledBlurListener } from '../utils/settledBlur';
import { type HoleRect, preparePageUnderlay, prewarmPageUnderlay, removeViewportHole, setPageUnderlay, setViewportHole, useElementHole } from '../hooks/useViewportOverlay';

/** The auto-hide sidebar's slide (keep in step with `.nsb.autohide` in sidebar.css). */
const SLIDE_IN_MS = 220;
const SLIDE_OUT_MS = 180;
/** How long the pointer must be away before the sidebar hides. */
const HIDE_DELAY_MS = 100;
/** A sidebar just revealed stays at least this long (a pass over the edge). */
const MIN_SHOWN_MS = 150;
const sleepMs = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
import {
  buildSidebarView,
  contextMenuActions,
  favoriteActivation,
  moveToSpaceOp,
  type SidebarItemKind,
  type SidebarItemRef,
  type SidebarMenuAction,
  type SidebarSource,
  type SidebarView,
} from '../utils/sidebarModel';
import type { FolderRecord, MoveOp, Profile, SidebarState, Space } from '../types';
import '../styles/sidebar.css';

export type { SidebarItemKind, SidebarItemRef } from '../utils/sidebarModel';

/**
 * Commands the sidebar issues. The names match `useBrowser()` so App can pass
 * `{ ...browser, openCommandBar }` directly.
 */
export interface SidebarActions {
  selectTab: (tabId: string) => void;
  selectSpace: (spaceId: string) => void;
  createTab: (url?: string, spaceId?: string) => unknown;
  closeTab: (tabId: string) => void;
  pinTab: (tabId: string) => unknown;
  unpinTab: (tabId: string) => unknown;
  addFavorite: (tabId: string) => unknown;
  removeFavorite: (favoriteId: string) => unknown;
  duplicateTab: (id: string, spaceId?: string) => unknown;
  moveTab: (op: MoveOp) => unknown;
  createFolder?: (spaceId: string, name: string) => unknown;
  updateFolder?: (folderId: string, patch: any) => unknown;
  deleteFolder?: (folderId: string) => unknown;
  moveTabToFolder?: (tabId: string, folderId: string | null) => unknown;
  /** Space management from the sidebar (no trip to Settings). */
  createSpace?: (name: string, color?: string) => Promise<Space | null | undefined> | unknown;
  updateSpace?: (spaceId: string, patch: { name?: string; icon?: string; color?: string }) => unknown;
  deleteSpace?: (spaceId: string) => unknown;
  reorderSpace?: (spaceId: string, toIndex: number) => unknown;
  /** New tab control (R12.8): App opens the Command_Bar, which creates the tab in the active space. */
  openCommandBar: () => void;
}

type ExtraProps = React.HTMLAttributes<HTMLElement> & { draggable?: boolean; [data: `data-${string}`]: unknown };

export interface SidebarProps {
  /** Live browser state (`useBrowser().state`). */
  state: SidebarSource;
  actions: SidebarActions;
  /** `settings.sidebarExpanded`. */
  expanded: boolean;
  /** `settings.sidebarAutoHide`. */
  autoHide: boolean;
  /** Custom sidebar width in px (defaults to 240). */
  sidebarWidth?: number;
  /** Live width change during drag resize. */
  onUpdateSidebarWidth?: (width: number) => void;
  /** Persists through the settings IPC (`useSettings().updateSettings` or `updateSidebarState`). */
  onUpdateSidebarState: (patch: Partial<SidebarState>) => void;

  // Extension points (drag and drop, hover previews). Handlers returned here
  // run before the sidebar's own handlers for the same event; calling
  // `preventDefault()` in one skips the sidebar's handler.
  /** Extra attributes/handlers for each favorite, pinned or Today item. */
  itemProps?: (item: SidebarItemRef) => ExtraProps;
  /** Extra attributes/handlers for each section's list container (drop zones). */
  sectionProps?: (section: SidebarItemKind) => ExtraProps;
  /** Extra attributes/handlers for each Space_Switcher icon (drop onto a space). */
  spaceProps?: (space: Space) => ExtraProps;
  /** Rendered at the end of a section's list (for example a drop indicator). */
  renderSectionExtras?: (section: SidebarItemKind) => React.ReactNode;
  /** Overlays rendered inside the sidebar (for example a hover preview card). */
  children?: React.ReactNode;
  className?: string;
  /** Opened from the toolbar or Ctrl+S: stays open until dismissed. */
  pinnedOpen?: boolean;
  /** Asks App to close a pinned-open sidebar (Esc, click outside, a tab picked). */
  onRequestClose?: () => void;
  /** A live web page is on screen (a native view the sidebar is cut out of). */
  livePage?: boolean;
  /** Size of the sidebar's contents relative to the default (Settings → Sizes). */
  scale?: number;
}

// ---------------------------------------------------------------------------
// Small helpers

const SPACE_ICONS: Record<string, LucideIcon> = {
  Home,
  Briefcase,
  BookOpen,
  GraduationCap,
  Code,
  Coffee,
  Music,
  Gamepad2,
  Camera,
  Heart,
  Star,
  Sparkles,
  Rocket,
  Compass,
  Globe,
  Leaf,
  Sun,
  Flame,
  Folder,
};
const SPACE_ICON_NAMES = Object.keys(SPACE_ICONS);

export const SpaceIcon: React.FC<{ space: Pick<Space, 'icon' | 'name' | 'color'>; size?: number }> = ({
  space,
  size = 14,
}) => {
  const Icon = SPACE_ICONS[space.icon];
  return (
    <span className="nsb-space-icon" style={{ color: space.color }} aria-hidden="true">
      {Icon ? <Icon size={size} /> : <span className="nsb-space-letter">{(space.name || '?').charAt(0).toUpperCase()}</span>}
    </span>
  );
};

const Favicon: React.FC<{ src?: string; size?: number }> = ({ src, size = 16 }) => {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [src]);
  if (!src || failed) return <Globe size={size - 2} className="nsb-favicon-fallback" aria-hidden="true" />;
  return <img src={src} alt="" width={size} height={size} className="nsb-favicon" onError={() => setFailed(true)} />;
};


const HANDLER_KEYS = [
  'onClick',
  'onContextMenu',
  'onMouseEnter',
  'onMouseLeave',
  'onPointerDown',
  'onKeyDown',
  'onDragStart',
  'onDragOver',
  'onDragLeave',
  'onDrop',
  'onDragEnd',
] as const;

/** Merges extension props over base props; extension handlers run first, then the base ones. */
function mergeProps<T extends Record<string, any>>(base: T, extra?: ExtraProps): T {
  if (!extra) return base;
  const out: Record<string, any> = { ...base, ...extra };
  for (const key of HANDLER_KEYS) {
    const a = (extra as any)[key];
    const b = (base as any)[key];
    if (typeof a === 'function' && typeof b === 'function') {
      out[key] = (e: React.SyntheticEvent) => {
        a(e);
        if (!e.defaultPrevented) b(e);
      };
    }
  }
  if (base.className || extra.className) out.className = [base.className, extra.className].filter(Boolean).join(' ');
  if (base.style || extra.style) out.style = { ...base.style, ...extra.style };
  return out as T;
}

export const MENU_LABELS: Record<SidebarMenuAction, string> = {
  pin: 'Pin',
  unpin: 'Unpin',
  'add-favorite': 'Add to favorites',
  'remove-favorite': 'Remove from favorites',
  'move-to-space': 'Move to space',
  'move-to-folder': 'Move to folder',
  duplicate: 'Duplicate',
  close: 'Close',
};

export const MENU_ICONS: Record<SidebarMenuAction, LucideIcon> = {
  pin: Pin,
  unpin: PinOff,
  'add-favorite': Star,
  'remove-favorite': StarOff,
  'move-to-space': ArrowRightLeft,
  'move-to-folder': Folder,
  duplicate: Copy,
  close: X,
};

interface MenuState {
  item: SidebarItemRef;
  x: number;
  y: number;
  showSpaces: boolean;
  showFolders?: boolean;
}

interface FolderMenuState {
  folder: FolderRecord;
  x: number;
  y: number;
}

interface SpaceMenuState {
  spaceId: string;
  x: number;
  y: number;
}

/** Keeps a menu of `width` x `height` inside the window. */
const clampMenu = (x: number, y: number, width = MENU_WIDTH, height = 300) => ({
  x: Math.max(4, Math.min(x, window.innerWidth - width - 4)),
  y: Math.max(4, Math.min(y, window.innerHeight - height - 4)),
});

const MENU_WIDTH = 210;

const SIDEBAR_GUIDE: GuideStep[] = [
  {
    target: '.nsb-switcher',
    title: 'Your spaces',
    body: 'Each space keeps its own tabs. Click one to switch, or swipe sideways with two fingers anywhere on the sidebar.',
  },
  {
    target: '.nsb-space-add',
    title: 'Add a space',
    body: 'Creates a space and lets you name it straight away.',
  },
  {
    target: '.nsb-header',
    title: 'Space actions',
    body: 'Double-click the name to rename. The ⋯ button (or a right-click on a space) changes its icon and color, moves it, adds a folder or deletes it.',
  },
  {
    target: '.nsb-pinned-section-header',
    title: 'Pinned and folders',
    body: 'Drag a tab up here to keep it. Drop it onto a folder to file it; the folder lights up when it will land there.',
  },
  {
    target: '.nsb-new-tab',
    title: 'Today',
    body: 'Tabs you open land here (Ctrl+T). Old ones are archived after a while, which you can change in Settings.',
  },
];
const SIDEBAR_GUIDE_KEY = 'serica_sidebar_guide_v1';

// ---------------------------------------------------------------------------

export const Sidebar: React.FC<SidebarProps> = ({
  state,
  actions,
  expanded,
  autoHide,
  sidebarWidth,
  onUpdateSidebarWidth,
  onUpdateSidebarState,
  itemProps,
  sectionProps,
  spaceProps,
  renderSectionExtras,
  children,
  className,
  pinnedOpen = false,
  onRequestClose,
  livePage = false,
  scale = 1,
}) => {
  // --- Sidebar Resizing (R12.11) -------------------------------------------
  const [isResizing, setIsResizing] = useState(false);
  const startXRef = useRef(0);
  const startWidthRef = useRef(sidebarWidth ?? 240);

  const handleResizeStart = (e: React.MouseEvent) => {
    if (!expanded) return;
    e.preventDefault();
    e.stopPropagation();
    setIsResizing(true);
    startXRef.current = e.clientX;
    startWidthRef.current = sidebarWidth ?? 240;
  };

  useEffect(() => {
    if (!isResizing) return;
    document.body.classList.add('sidebar-resizing-active');

    const onMouseMove = (e: MouseEvent) => {
      const delta = e.clientX - startXRef.current;
      const nextWidth = Math.max(160, Math.min(520, Math.round(startWidthRef.current + delta)));
      onUpdateSidebarWidth?.(nextWidth);
    };

    const onMouseUp = (e: MouseEvent) => {
      setIsResizing(false);
      document.body.classList.remove('sidebar-resizing-active');
      const delta = e.clientX - startXRef.current;
      const finalWidth = Math.max(160, Math.min(520, Math.round(startWidthRef.current + delta)));
      onUpdateSidebarState({ sidebarWidth: finalWidth });
    };

    window.addEventListener('mousemove', onMouseMove);
    window.addEventListener('mouseup', onMouseUp);
    return () => {
      document.body.classList.remove('sidebar-resizing-active');
      window.removeEventListener('mousemove', onMouseMove);
      window.removeEventListener('mouseup', onMouseUp);
    };
  }, [isResizing, onUpdateSidebarWidth, onUpdateSidebarState]);

  // Drag and drop (R17): renders the optimistic layout while dragging/committing.
  const drag = useSidebarDrag(state, actions.moveTab);
  const view = useMemo(() => buildSidebarView(drag.displayState), [drag.displayState]);
  const { activeSpaceId, activeTabId } = state;

  // --- Auto-hide (R12.4) -----------------------------------------------------
  const [pointerX, setPointerX] = useState<number | null>(null);
  const [overSidebar, setOverSidebar] = useState(false);
  const [autoHideRevealed, setAutoHideRevealed] = useState(false);
  const autoHideRevealedAtRef = useRef<number>(0);
  const autoHideTimerRef = useRef<number | null>(null);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [folderMenu, setFolderMenu] = useState<FolderMenuState | null>(null);
  const [creatingFolder, setCreatingFolder] = useState(false);
  const [newFolderName, setNewFolderName] = useState('');
  const [renamingFolderId, setRenamingFolderId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [spaceMenu, setSpaceMenu] = useState<SpaceMenuState | null>(null);
  const [renamingSpaceId, setRenamingSpaceId] = useState<string | null>(null);
  const [spaceNameDraft, setSpaceNameDraft] = useState('');
  const [guideOpen, setGuideOpen] = useState(false);

  const isMenuOrDragOpen = menu !== null || folderMenu !== null || spaceMenu !== null || drag.dragging;

  const revealSidebar = useCallback(() => {
    if (autoHideTimerRef.current !== null) {
      window.clearTimeout(autoHideTimerRef.current);
      autoHideTimerRef.current = null;
    }
    autoHideRevealedAtRef.current = Date.now();
    setAutoHideRevealed(true);
  }, []);

  const scheduleHideSidebar = useCallback((delay = HIDE_DELAY_MS) => {
    if (menu !== null || folderMenu !== null || spaceMenu !== null || drag.dragging) return;
    if (autoHideTimerRef.current !== null) {
      window.clearTimeout(autoHideTimerRef.current);
    }
    // The slide out runs over a snapshot of the page: take it during the delay.
    if (livePage) prewarmPageUnderlay();
    autoHideTimerRef.current = window.setTimeout(() => {
      const elapsed = Date.now() - autoHideRevealedAtRef.current;
      if (elapsed < MIN_SHOWN_MS) {
        autoHideTimerRef.current = window.setTimeout(() => {
          setAutoHideRevealed(false);
          autoHideTimerRef.current = null;
        }, MIN_SHOWN_MS - elapsed);
      } else {
        setAutoHideRevealed(false);
        autoHideTimerRef.current = null;
      }
    }, delay);
  }, [menu, folderMenu, spaceMenu, drag.dragging, livePage]);

  const cancelHideSidebar = useCallback(() => {
    if (autoHideTimerRef.current !== null) {
      window.clearTimeout(autoHideTimerRef.current);
      autoHideTimerRef.current = null;
    }
  }, []);

  useEffect(() => {
    if (!autoHide) return;
    const targetWidth = (expanded ? (sidebarWidth ?? 240) : 56) + 24;
    const onMove = (e: MouseEvent) => {
      setPointerX(e.clientX);
      if (e.clientX <= SIDEBAR_EDGE_PX) {
        revealSidebar();
      } else if (e.clientX > targetWidth && !overSidebar) {
        scheduleHideSidebar();
      } else {
        cancelHideSidebar();
      }
    };
    const onLeave = () => {
      setPointerX(null);
      scheduleHideSidebar(HIDE_DELAY_MS);
    };
    window.addEventListener('mousemove', onMove, { passive: true });
    document.documentElement.addEventListener('mouseleave', onLeave);
    const removeBlur = addSettledBlurListener(onLeave);
    return () => {
      window.removeEventListener('mousemove', onMove);
      document.documentElement.removeEventListener('mouseleave', onLeave);
      removeBlur();
      if (autoHideTimerRef.current !== null) {
        window.clearTimeout(autoHideTimerRef.current);
      }
    };
  }, [autoHide, expanded, sidebarWidth, overSidebar, revealSidebar, scheduleHideSidebar, cancelHideSidebar]);

  useEffect(() => {
    if (!spaceMenu) return;
    const close = () => setSpaceMenu(null);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    window.addEventListener('mousedown', close);
    const removeBlur = addSettledBlurListener(close);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('mousedown', close);
      removeBlur();
      window.removeEventListener('keydown', onKey);
    };
  }, [spaceMenu]);

  useEffect(() => {
    if (!folderMenu) return;
    const close = () => setFolderMenu(null);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    window.addEventListener('mousedown', close);
    const removeBlur = addSettledBlurListener(close);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('mousedown', close);
      removeBlur();
      window.removeEventListener('keydown', onKey);
    };
  }, [folderMenu]);

  // An open context menu keeps an auto-hidden sidebar visible.
  // An active drag keeps it visible too.
  const isOverOrActive = overSidebar || isMenuOrDragOpen || autoHideRevealed || pinnedOpen || guideOpen;
  const visible = sidebarVisibility(autoHide, pointerX, isOverOrActive);
  // The sidebar shows only once the page under it is clipped (the backend
  // confirms the hole is applied), so it never appears beneath the page.
  const [holeReady, setHoleReady] = useState(false);
  // Over a live page the clip is cut and restored in one step: the page's
  // window region is set on the app's main thread, which can stall for tens
  // of ms (a page loading), so a clip animated step by step falls behind the
  // sidebar's slide and the page draws over it. Instead a snapshot of the
  // page goes beneath it first, the clip is cut in one step (nothing visibly
  // changes), and the sidebar slides over the snapshot; closing runs the
  // same steps backwards.
  const instant = Boolean(livePage);
  const shown = instant ? holeReady : visible && holeReady;

  const holeWidth = (sidebarWidth ?? 240) + 1;
  const holeShown = useRef(false);
  const holeSeq = useRef(0);
  useLayoutEffect(() => {
    if (!autoHide) return;
    const H = 100000;
    const seq = ++holeSeq.current;
    const current = () => seq === holeSeq.current;
    const opening = visible && !holeShown.current;
    const closing = !visible && holeShown.current;
    if (instant && opening) {
      void (async () => {
        await preparePageUnderlay();
        if (!current()) return;
        await setViewportHole('sidebar', [0, 0, holeWidth, H]);
        if (!current()) return;
        setHoleReady(true); // slides in over the snapshot
        await sleepMs(SLIDE_IN_MS + 40);
        if (current()) setPageUnderlay(null); // fully covered by the sidebar now
      })();
    } else if (instant && closing) {
      void (async () => {
        await preparePageUnderlay();
        if (!current()) return;
        setHoleReady(false); // slides out over the snapshot
        await sleepMs(SLIDE_OUT_MS + 20);
        if (!current()) return;
        await setViewportHole('sidebar', [0, 0, 0, 0], { remove: true });
        if (current()) setPageUnderlay(null);
      })();
    } else if (instant) {
      // A resize while open, or nothing to undo.
      if (visible) {
        void setViewportHole('sidebar', [0, 0, holeWidth, H]).then(() => {
          if (current()) setHoleReady(true);
        });
      } else {
        setHoleReady(false);
      }
    } else if (visible) {
      const anim = opening ? { from: [0, 0, -24, H] as HoleRect, ms: SLIDE_IN_MS, ease: [0.22, 1, 0.36, 1] as [number, number, number, number], lead: 8 } : undefined;
      void setViewportHole('sidebar', [0, 0, holeWidth, H], anim).then(() => {
        if (current()) setHoleReady(true);
      });
    } else {
      setHoleReady(false);
      if (holeShown.current) {
        void setViewportHole('sidebar', [0, 0, -24, H], { from: [0, 0, holeWidth, H], ms: SLIDE_OUT_MS, ease: [0.4, 0, 0.6, 1], lead: -12, remove: true });
      }
    }
    holeShown.current = visible;
  }, [autoHide, visible, holeWidth, instant]);
  useEffect(
    () => () => {
      removeViewportHole('sidebar');
      setPageUnderlay(null);
    },
    [],
  );

  // The top bar's left buttons (sidebar toggle, back/forward/reload/home,
  // new tab) move over to sit beside the open sidebar instead of under it.
  useLayoutEffect(() => {
    if (!autoHide) return;
    const root = document.documentElement;
    root.style.setProperty('--sidebar-cover', shown ? `${holeWidth}px` : '0px');
    root.classList.toggle('sidebar-covering', shown);
  }, [autoHide, shown, holeWidth]);
  useEffect(
    () => () => {
      document.documentElement.style.removeProperty('--sidebar-cover');
      document.documentElement.classList.remove('sidebar-covering');
    },
    [],
  );

  // --- Hover previews (R18.1, R18.2) -------------------------------------------
  // Nothing shows while dragging, while a context menu is open or while the sidebar is hidden.
  const preview = useHoverPreview(drag.dragging || menu !== null || folderMenu !== null || spaceMenu !== null || !shown);
  // Menus show through the live page too.
  const [menuEl, setMenuEl] = useState<HTMLDivElement | null>(null);
  const [folderMenuEl, setFolderMenuEl] = useState<HTMLDivElement | null>(null);
  const menuReady = useElementHole('sidebar-menu', menu ? menuEl : null);
  const folderMenuReady = useElementHole('sidebar-folder-menu', folderMenu ? folderMenuEl : null);
  const [spaceMenuEl, setSpaceMenuEl] = useState<HTMLDivElement | null>(null);
  const spaceMenuReady = useElementHole('sidebar-space-menu', spaceMenu ? spaceMenuEl : null);

  // Hides the sidebar now (a pick was made or it was dismissed).
  const dismiss = useCallback(() => {
    if (autoHideTimerRef.current !== null) {
      window.clearTimeout(autoHideTimerRef.current);
      autoHideTimerRef.current = null;
    }
    setAutoHideRevealed(false);
    setOverSidebar(false);
    setPointerX(null);
    onRequestClose?.();
  }, [onRequestClose]);

  // A pinned-open sidebar closes on Escape or a press outside it.
  useEffect(() => {
    if (!pinnedOpen) return;
    const onDown = (e: PointerEvent) => {
      const t = e.target as Element | null;
      if (t?.closest('.nsb, .nsb-menu, .nsb-preview, .guide-card, [data-sidebar-toggle]')) return;
      onRequestClose?.();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onRequestClose?.();
    };
    window.addEventListener('pointerdown', onDown, true);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('keydown', onKey);
    };
  }, [pinnedOpen, onRequestClose]);

  // Over a live page the UI gets no pointer events (the page is a separate
  // native window), so it cannot see the pointer leave the sidebar or click
  // the page. While the sidebar is open there, the OS cursor is polled: once
  // it has been inside the sidebar and then stays outside it (and its menus
  // and hover card) for HIDE_DELAY_MS, the sidebar hides, however it opened.
  // A sidebar opened from the keyboard with the pointer already on the page
  // waits until the pointer has visited it.
  const busyRef = useRef(false);
  busyRef.current = isMenuOrDragOpen || creatingFolder || renamingFolderId !== null || renamingSpaceId !== null || guideOpen;

  // The guide plays the first time the sidebar opens, and on request (Settings).
  useEffect(() => {
    const onGuide = (e: Event) => {
      if ((e as CustomEvent).detail === 'sidebar') setGuideOpen(true);
    };
    window.addEventListener('serica:guide', onGuide);
    return () => window.removeEventListener('serica:guide', onGuide);
  }, []);
  useEffect(() => {
    if (!shown) return;
    try {
      if (localStorage.getItem(SIDEBAR_GUIDE_KEY)) return;
      localStorage.setItem(SIDEBAR_GUIDE_KEY, '1');
    } catch {
      return;
    }
    setGuideOpen(true);
  }, [shown]);
  useEffect(() => {
    if (!autoHide || !shown || !livePage || !window.serica?.uiCursor) return;
    let alive = true;
    let timer = 0;
    let armed = false;
    let outsideSince: number | null = null;
    const insideUi = (x: number, y: number) => {
      if (x <= (sidebarWidth ?? 240) + 4) return true;
      for (const el of document.querySelectorAll('.nsb-menu, .nsb-preview')) {
        const r = el.getBoundingClientRect();
        if (x >= r.left - 4 && x <= r.right + 4 && y >= r.top - 4 && y <= r.bottom + 4) return true;
      }
      return false;
    };
    const tick = async () => {
      const c = await window.serica!.uiCursor!().catch(() => null);
      if (!alive) return;
      const k = c && c[2] > 0 ? window.innerWidth / c[2] : 0;
      const inside = c !== null && k > 0 && insideUi(c[0] * k, c[1] * k);
      const now = performance.now();
      if (inside || busyRef.current) {
        armed = armed || inside;
        outsideSince = null;
      } else if (armed) {
        if (outsideSince === null) {
          outsideSince = now;
          prewarmPageUnderlay(); // for the slide out
        } else if (now - outsideSince >= HIDE_DELAY_MS) {
          dismiss();
          return;
        }
      }
      timer = window.setTimeout(tick, 40);
    };
    timer = window.setTimeout(tick, 40);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [autoHide, shown, livePage, sidebarWidth, dismiss]);
  const hidePreview = preview.hide;
  const previewKey = preview.state?.target.key;
  useEffect(() => {
    // The hovered item was closed or moved out of view.
    if (!previewKey) return;
    const present = [view.favorites, view.pinned, view.today].some((list) => list.some((i) => i.id === previewKey));
    if (!present) hidePreview();
  }, [view, previewKey, hidePreview]);
  /** Drag handlers, then preview handlers (both chain; neither prevents default), then extensions. */
  const itemExtraProps = (item: SidebarItemRef): ExtraProps =>
    mergeProps(drag.itemProps(item) as ExtraProps, preview.itemProps(item) as ExtraProps);

  // --- Swipe between spaces (R13.3) -------------------------------------------
  // Panes: previous, current and next space on a horizontal track. useSpaceSwipe
  // moves the track by ref; the current pane is the only interactive one.
  const rootRef = useRef<HTMLElement>(null);
  const trackRef = useRef<HTMLDivElement>(null);
  const swipeOrder = useMemo(() => view.spaceOrder.map((s) => ({ id: s.id, color: s.color })), [view.spaceOrder]);
  const outgoing = useSpaceSwipe({
    rootRef,
    trackRef,
    order: swipeOrder,
    activeId: activeSpaceId,
    blocked: drag.dragging || menu !== null || folderMenu !== null || spaceMenu !== null || renamingSpaceId !== null,
    onCommit: actions.selectSpace,
    onStart: hidePreview,
  });
  const activeIndex = view.spaceOrder.findIndex((s) => s.id === activeSpaceId);
  const prevId =
    outgoing?.side === 'prev' ? outgoing.id : activeIndex > 0 ? view.spaceOrder[activeIndex - 1].id : undefined;
  const nextId =
    outgoing?.side === 'next'
      ? outgoing.id
      : activeIndex >= 0 && activeIndex < view.spaceOrder.length - 1
        ? view.spaceOrder[activeIndex + 1].id
        : undefined;
  const displayState = drag.displayState;
  const prevView = useMemo(
    () => (prevId ? buildSidebarView({ ...displayState, activeSpaceId: prevId }) : null),
    [displayState, prevId],
  );
  const nextView = useMemo(
    () => (nextId ? buildSidebarView({ ...displayState, activeSpaceId: nextId }) : null),
    [displayState, nextId],
  );
  // Favorites belong to the profile: static above the track unless a neighbour has another profile.
  const profileOfSpace = new Map(view.groups.flatMap((g) => g.spaces.map((s) => [s.id, g.profile.id] as const)));
  const ownProfile = profileOfSpace.get(activeSpaceId);
  const favoritesInPanes = [prevId, nextId].some((id) => id !== undefined && profileOfSpace.get(id) !== ownProfile);

  // --- Context menu (R12.10) --------------------------------------------------
  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    window.addEventListener('mousedown', close);
    const removeBlur = addSettledBlurListener(close);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('mousedown', close);
      removeBlur();
      window.removeEventListener('keydown', onKey);
    };
  }, [menu]);

  const openMenu = (item: SidebarItemRef, x: number, y: number) => {
    setMenu({
      item,
      x: Math.max(4, Math.min(x, window.innerWidth - MENU_WIDTH - 4)),
      y: Math.max(4, Math.min(y, window.innerHeight - 280)),
      showSpaces: false,
    });
  };

  const runMenuAction = (item: SidebarItemRef, action: SidebarMenuAction) => {
    switch (action) {
      case 'pin':
        actions.pinTab(item.id);
        break;
      case 'unpin':
        actions.unpinTab(item.id);
        break;
      case 'add-favorite':
        actions.addFavorite(item.id);
        break;
      case 'remove-favorite':
        actions.removeFavorite(item.id);
        break;
      case 'duplicate':
        actions.duplicateTab(item.id, item.kind === 'favorite' ? activeSpaceId : undefined);
        break;
      case 'close':
        if (item.kind === 'favorite') {
          if (item.tab) actions.closeTab(item.tab.id);
        } else {
          actions.closeTab(item.id);
        }
        break;
      case 'move-to-space':
        setMenu((m) => (m ? { ...m, showSpaces: true } : m));
        return;
      case 'move-to-folder':
        setMenu((m) => (m ? { ...m, showFolders: true } : m));
        return;
    }
    setMenu(null);
  };

  // --- Folders and spaces, managed in place ------------------------------------
  /** New folder (optionally holding a tab), named in place right away. */
  const createFolderAndRename = async (tabId?: string) => {
    const res: any = await actions.createFolder?.(activeSpaceId, 'New Folder');
    const folderId: string | undefined = res?.folder?.id ?? res?.id;
    if (!folderId) return;
    if (tabId) await actions.moveTabToFolder?.(tabId, folderId);
    setRenamingFolderId(folderId);
    setRenameValue('New Folder');
  };

  const openSpaceMenu = (spaceId: string, x: number, y: number) => {
    setMenu(null);
    setFolderMenu(null);
    setSpaceMenu({ spaceId, ...clampMenu(x, y, MENU_WIDTH, 320) });
  };

  const startRenameSpace = (space: Space) => {
    setSpaceNameDraft(space.name);
    setRenamingSpaceId(space.id);
  };

  const commitRenameSpace = () => {
    const id = renamingSpaceId;
    setRenamingSpaceId(null);
    if (!id) return;
    const name = spaceNameDraft.trim();
    const current = state.spaces.find((s) => s.id === id);
    if (name && current && name !== current.name) actions.updateSpace?.(id, { name });
  };

  /** "+" in the switcher: a new space, active at once, named in place. */
  const addSpace = async () => {
    const taken = new Set(state.spaces.map((s) => s.name));
    let n = state.spaces.length + 1;
    while (taken.has(`Space ${n}`)) n += 1;
    const created: any = await actions.createSpace?.(`Space ${n}`);
    if (created?.id) {
      setSpaceNameDraft(created.name);
      setRenamingSpaceId(created.id);
    }
  };

  // --- Activation -------------------------------------------------------------
  const activate = (item: SidebarItemRef) => {
    // The sidebar floats over the page: get out of the way of the page just picked.
    dismiss();
    if (item.kind === 'favorite' && item.favorite) {
      const target = favoriteActivation(item.favorite, state.tabs, activeSpaceId);
      if (target.type === 'select') actions.selectTab(target.tabId);
      else actions.createTab(target.url, target.spaceId);
      return;
    }
    // Unloaded pinned tabs load their saved URL when selected (R15.4).
    actions.selectTab(item.id);
  };

  const itemBaseProps = (item: SidebarItemRef) => ({
    role: 'button',
    tabIndex: 0,
    'data-sidebar-item': item.kind,
    'data-sidebar-id': item.id,
    'data-sidebar-index': item.index,
    // aria-label, not title: a native tooltip would cover the hover preview card.
    'aria-label': item.title || item.url || 'New Tab',
    onClick: () => activate(item),
    onKeyDown: (e: React.KeyboardEvent) => {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        activate(item);
      } else if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
        e.preventDefault();
        const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
        openMenu(item, r.left + 12, r.bottom);
      }
    },
    onContextMenu: (e: React.MouseEvent) => {
      e.preventDefault();
      openMenu(item, e.clientX, e.clientY);
    },
  });

  const isActiveItem = (item: SidebarItemRef) =>
    item.tab !== undefined && item.tab.id === activeTabId;

  // --- Rendering --------------------------------------------------------------
  const renderFavorite = (item: SidebarItemRef, inert = false) => {
    const cls = `nsb-fav ${isActiveItem(item) ? 'active' : ''} ${item.tab ? 'open' : ''}`;
    if (inert) {
      return (
        <div key={item.id} className={cls}>
          <Favicon src={item.favicon} size={18} />
        </div>
      );
    }
    const base = { ...itemBaseProps(item), className: cls };
    return (
      <div key={item.id} {...mergeProps(mergeProps(base, itemExtraProps(item)), itemProps?.(item))}>
        <Favicon src={item.favicon} size={18} />
      </div>
    );
  };
  const renderTabRow = (item: SidebarItemRef, inert = false) => {
    const active = isActiveItem(item);
    // Sleeping tabs look like any other: memory is freed in the background.
    const cls = `nsb-tab ${active ? 'active' : ''} ${item.tab?.unloaded ? 'unloaded' : ''}`;
    const content = (
      <>
        <span className="nsb-tab-favicon">
          <Favicon src={item.favicon ?? item.tab?.favicon} />
        </span>
        <span className="nsb-tab-title">{item.title || item.url || 'New Tab'}</span>
      </>
    );
    // Neighbour panes only need to look right: no handlers, no focusable controls.
    if (inert) {
      return (
        <div key={item.id} className={cls}>
          {content}
        </div>
      );
    }
    const base = {
      ...itemBaseProps(item),
      'aria-current': active ? ('page' as const) : undefined,
      className: cls,
    };
    return (
      <div key={item.id} {...mergeProps(mergeProps(base, itemExtraProps(item)), itemProps?.(item))}>
        {content}
        <button
          type="button"
          className="nsb-tab-close"
          title={item.kind === 'pinned' ? 'Close (keeps the pin)' : 'Close tab'}
          aria-label="Close tab"
          onClick={(e) => {
            e.stopPropagation();
            actions.closeTab(item.id);
          }}
        >
          <X size={12} />
        </button>
      </div>
    );
  };
  const section = (kind: SidebarItemKind, body: React.ReactNode, cls: string) => (
    <div {...mergeProps({ className: cls, 'data-sidebar-section': kind }, sectionProps?.(kind))}>
      {body}
      {renderSectionExtras?.(kind)}
    </div>
  );

  const renderPinnedContent = (v: SidebarView, inert: boolean) => {
    const folders = v.folders || [];
    const rootPinned = v.pinned.filter((t) => !t.folderId);

    return (
      <div className="nsb-pinned-container">
        {expanded && (
          <div className="nsb-pinned-section-header">
            <span className="nsb-pinned-section-label">Pinned</span>
            {!inert && (
              <button
                type="button"
                className="nsb-folder-add-btn"
                title="New Folder"
                aria-label="New Folder"
                onClick={(e) => {
                  e.stopPropagation();
                  setCreatingFolder(true);
                }}
              >
                <FolderPlus size={12} />
              </button>
            )}
          </div>
        )}
        {!inert && creatingFolder && (
          <form
            className="nsb-folder-create-form"
            onSubmit={(e) => {
              e.preventDefault();
              if (newFolderName.trim()) {
                actions.createFolder?.(activeSpaceId, newFolderName.trim());
              }
              setNewFolderName('');
              setCreatingFolder(false);
            }}
          >
            <Folder size={13} className="nsb-folder-form-icon" />
            <input
              autoFocus
              type="text"
              className="nsb-folder-create-input"
              placeholder="Folder name..."
              value={newFolderName}
              onChange={(e) => setNewFolderName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Escape') {
                  setCreatingFolder(false);
                  setNewFolderName('');
                }
              }}
              onBlur={() => {
                if (newFolderName.trim()) {
                  actions.createFolder?.(activeSpaceId, newFolderName.trim());
                }
                setNewFolderName('');
                setCreatingFolder(false);
              }}
            />
          </form>
        )}
        {folders.map((folder) => {
          const folderTabs = v.pinned.filter((t) => t.folderId === folder.id);
          return (
            <div
              className={`nsb-folder ${folder.collapsed ? 'collapsed' : 'expanded'}${!inert && drag.dropFolderId === folder.id ? ' drop-target' : ''}`}
              key={folder.id}
              data-folder-id={inert ? undefined : folder.id}
            >
              <div
                className="nsb-folder-header"
                role="button"
                tabIndex={0}
                onClick={() => {
                  if (!inert) actions.updateFolder?.(folder.id, { collapsed: !folder.collapsed });
                }}
                onContextMenu={(e) => {
                  if (inert) return;
                  e.preventDefault();
                  e.stopPropagation();
                  setFolderMenu({
                    folder,
                    x: Math.max(4, Math.min(e.clientX, window.innerWidth - MENU_WIDTH - 4)),
                    y: Math.max(4, Math.min(e.clientY, window.innerHeight - 200)),
                  });
                }}
              >
                <button
                  type="button"
                  className="nsb-folder-chevron-btn"
                  title={folder.collapsed ? 'Expand folder' : 'Collapse folder'}
                  aria-label={folder.collapsed ? 'Expand folder' : 'Collapse folder'}
                  onClick={(e) => {
                    e.stopPropagation();
                    if (!inert) actions.updateFolder?.(folder.id, { collapsed: !folder.collapsed });
                  }}
                >
                  <ChevronRight
                    size={11}
                    className={`nsb-folder-chevron ${!folder.collapsed ? 'expanded' : ''}`}
                  />
                </button>
                <span className="nsb-folder-icon">
                  {!folder.collapsed ? <FolderOpen size={13} /> : <Folder size={13} />}
                </span>
                {!inert && renamingFolderId === folder.id ? (
                  <input
                    autoFocus
                    type="text"
                    className="nsb-folder-rename-input"
                    value={renameValue}
                    onChange={(e) => setRenameValue(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') {
                        if (renameValue.trim()) {
                          actions.updateFolder?.(folder.id, { name: renameValue.trim() });
                        }
                        setRenamingFolderId(null);
                      } else if (e.key === 'Escape') {
                        setRenamingFolderId(null);
                      }
                    }}
                    onBlur={() => {
                      if (renameValue.trim()) {
                        actions.updateFolder?.(folder.id, { name: renameValue.trim() });
                      }
                      setRenamingFolderId(null);
                    }}
                    onClick={(e) => e.stopPropagation()}
                  />
                ) : (
                  <span
                    className="nsb-folder-title"
                    onDoubleClick={(e) => {
                      if (inert) return;
                      e.stopPropagation();
                      setRenamingFolderId(folder.id);
                      setRenameValue(folder.name);
                    }}
                  >
                    {folder.name}
                  </span>
                )}
                {folderTabs.length > 0 && (
                  <span className="nsb-folder-badge">{folderTabs.length}</span>
                )}
                {!inert && (
                  <button
                    type="button"
                    className="nsb-folder-menu-trigger"
                    title="Folder actions"
                    aria-label="Folder actions"
                    onClick={(e) => {
                      e.stopPropagation();
                      const r = e.currentTarget.getBoundingClientRect();
                      setFolderMenu({
                        folder,
                        x: Math.max(4, Math.min(r.left, window.innerWidth - MENU_WIDTH - 4)),
                        y: Math.max(4, Math.min(r.bottom + 4, window.innerHeight - 200)),
                      });
                    }}
                  >
                    <MoreHorizontal size={12} />
                  </button>
                )}
              </div>
              {!folder.collapsed && (
                <div className="nsb-folder-children">
                  {folderTabs.length > 0 ? (
                    folderTabs.map((item) => renderTabRow(item, inert))
                  ) : (
                    <div className="nsb-folder-empty-hint">Drop tabs here</div>
                  )}
                </div>
              )}
            </div>
          );
        })}
        {rootPinned.map((item) => renderTabRow(item, inert))}
        {expanded && rootPinned.length === 0 && folders.length === 0 && (
          <div className="nsb-section-empty-hint">Drag tabs here to pin them</div>
        )}
      </div>
    );
  };

  /** Current pane: a real drop-target section. Neighbour panes: plain markup. */
  const paneSection = (kind: SidebarItemKind, items: SidebarItemRef[], cls: string, current: boolean, v?: SidebarView) =>
    current ? (
      section(
        kind,
        kind === 'pinned' && v
          ? renderPinnedContent(v, false)
          : items.map((i) => (kind === 'favorite' ? renderFavorite(i) : renderTabRow(i))),
        cls,
      )
    ) : (
      <div className={cls}>
        {kind === 'pinned' && v
          ? renderPinnedContent(v, true)
          : items.map((i) => (kind === 'favorite' ? renderFavorite(i, true) : renderTabRow(i, true)))}
      </div>
    );
  const renderPane = (id: string | undefined, slot: 'prev' | 'next' | 'current', v: SidebarView | null) => {
    if (!id || !v) return <div key={`empty-${slot}`} className="nsb-pane" aria-hidden="true" />;
    const current = slot === 'current';
    const showPinnedOrToday = v.pinned.length > 0 || v.today.length > 0 || (v.folders && v.folders.length > 0);
    return (
      <div
        key={id}
        className={`nsb-pane ${current ? 'current' : 'inert'}`}
        aria-hidden={current ? undefined : true}
        style={current ? undefined : { pointerEvents: 'none' }}
      >
        {favoritesInPanes && (current || v.favorites.length > 0) && paneSection('favorite', v.favorites, 'nsb-favorites', current)}
        {paneSection('pinned', v.pinned, 'nsb-list nsb-pinned', current, v)}
        {showPinnedOrToday && <div className="nsb-divider" role="separator" />}
        {expanded && (
          <div className="nsb-pinned-section-header nsb-today-section-header">
            <span className="nsb-pinned-section-label">Today</span>
          </div>
        )}
        {current ? (
          <button
            type="button"
            className="nsb-new-tab"
            title="New tab (Ctrl+T)"
            onClick={() => {
              dismiss();
              actions.openCommandBar();
            }}
          >
            <Plus size={14} aria-hidden="true" />
            <span>New Tab</span>
            <kbd className="nsb-new-tab-kbd">Ctrl T</kbd>
          </button>
        ) : (
          <div className="nsb-new-tab" aria-hidden="true">
            <Plus size={14} />
            <span>New Tab</span>
          </div>
        )}
        {paneSection('today', v.today, 'nsb-list nsb-today', current)}
      </div>
    );
  };
  const activeSpace = view.activeSpace;
  const activeProfile = view.activeProfile;
  const profileCount = view.groups.length;

  const rootClass = [
    'nsb',
    expanded ? 'expanded' : 'collapsed',
    drag.dragging ? 'dragging' : '',
    autoHide ? 'autohide' : '',
    shown ? 'visible' : 'hidden',
    className ?? '',
  ]
    .filter(Boolean)
    .join(' ');

  const spaceButton = (space: Space, profile: Profile) => {
    const active = space.id === activeSpaceId;
    const base = {
      type: 'button' as const,
      className: `nsb-space-btn ${active ? 'active' : ''}`,
      'data-space-id': space.id,
      'data-profile-id': profile.id,
      'aria-pressed': active,
      title: profileCount > 1 ? `${space.name} (${profile.name})` : space.name,
      'aria-label': `Space ${space.name}, profile ${profile.name}`,
      style: { '--nsb-btn-color': space.color } as React.CSSProperties,
      onClick: () => actions.selectSpace(space.id),
      onContextMenu: (e: React.MouseEvent) => {
        e.preventDefault();
        openSpaceMenu(space.id, e.clientX, e.clientY - 300);
      },
    };
    return (
      <button key={space.id} {...mergeProps(mergeProps(base, drag.spaceProps(space.id)), spaceProps?.(space))}>
        <SpaceIcon space={space} size={14} />
      </button>
    );
  };

  const menuSpaces = menu
    ? view.spaceOrder.filter((s) => menu.item.kind === 'favorite' || s.id !== menu.item.tab?.spaceId)
    : [];

  return (
    <>
      {autoHide && (
        <div
          className="nsb-edge-zone"
          aria-hidden="true"
          onMouseEnter={(e) => {
            setPointerX(e.clientX);
            revealSidebar();
          }}
          onMouseMove={(e) => {
            setPointerX(e.clientX);
            revealSidebar();
          }}
        />
      )}
      <aside
        ref={rootRef}
        className={rootClass}
        aria-label="Sidebar"
        aria-hidden={!shown}
        style={{
          '--nsb-width-expanded': `${sidebarWidth ?? 240}px`,
          '--nsb-space-color': activeSpace?.color ?? 'var(--theme-accent)',
          '--sb-scale': scale,
        } as React.CSSProperties}
        onMouseEnter={() => {
          cancelHideSidebar();
          setOverSidebar(true);
        }}
        onMouseLeave={() => {
          setOverSidebar(false);
          scheduleHideSidebar();
        }}
      >
        {expanded && (
          <div
            className={`nsb-resizer ${isResizing ? 'active' : ''}`}
            onMouseDown={handleResizeStart}
            title="Drag to resize tab holder"
            role="separator"
            aria-orientation="vertical"
          />
        )}
        {/* Active space: name, icon and color (R13.6) */}
        <div className="nsb-header">
          {activeSpace && (
            <div
              className="nsb-space-title"
              title={renamingSpaceId === activeSpace.id ? undefined : `${activeSpace.name} (double-click to rename)`}
              onDoubleClick={() => startRenameSpace(activeSpace)}
              onContextMenu={(e) => {
                e.preventDefault();
                openSpaceMenu(activeSpace.id, e.clientX, e.clientY);
              }}
            >
              <SpaceIcon space={activeSpace} size={15} />
              {renamingSpaceId === activeSpace.id ? (
                <input
                  autoFocus
                  className="nsb-space-rename"
                  aria-label="Space name"
                  value={spaceNameDraft}
                  maxLength={32}
                  onFocus={(e) => e.currentTarget.select()}
                  onChange={(e) => setSpaceNameDraft(e.target.value)}
                  onBlur={commitRenameSpace}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') commitRenameSpace();
                    else if (e.key === 'Escape') {
                      e.stopPropagation();
                      setRenamingSpaceId(null);
                    }
                  }}
                />
              ) : (
                <span className="nsb-space-name">{activeSpace.name}</span>
              )}
              {activeProfile && profileCount > 1 && renamingSpaceId !== activeSpace.id && (
                <span className="nsb-profile-chip" title={`Profile: ${activeProfile.name}`}>
                  {activeProfile.name}
                </span>
              )}
            </div>
          )}
          <div className="nsb-header-actions">
            {activeSpace && (
              <button
                type="button"
                className={`nsb-icon-btn${spaceMenu?.spaceId === activeSpace.id ? ' active' : ''}`}
                title="Space actions"
                aria-label="Space actions"
                aria-haspopup="menu"
                onMouseDown={(e) => e.stopPropagation()}
                onClick={(e) => {
                  if (spaceMenu) {
                    setSpaceMenu(null);
                    return;
                  }
                  const r = e.currentTarget.getBoundingClientRect();
                  openSpaceMenu(activeSpace.id, r.right - MENU_WIDTH, r.bottom + 4);
                }}
              >
                <MoreHorizontal size={14} />
              </button>
            )}
            <button
              type="button"
              className="nsb-icon-btn"
              title="Hide sidebar (Ctrl+S)"
              aria-label="Hide sidebar"
              onPointerEnter={livePage ? prewarmPageUnderlay : undefined}
              onClick={dismiss}
            >
              <PanelLeftClose size={14} />
            </button>
          </div>
        </div>

        {!favoritesInPanes && section('favorite', view.favorites.map((f) => renderFavorite(f)), 'nsb-favorites')}
        <div className="nsb-panes">
          <div className="nsb-track" ref={trackRef}>
            {renderPane(prevId, 'prev', prevView)}
            {renderPane(activeSpaceId, 'current', view)}
            {renderPane(nextId, 'next', nextView)}
          </div>
        </div>
        {/* Space_Switcher: one row of icons, profile groups separated by a divider (R13.1) */}
        <nav className="nsb-switcher" aria-label="Spaces">
          {view.groups.map(({ profile, spaces }) =>
            spaces.length === 0 ? null : (
              <div key={profile.id} className="nsb-switcher-group" data-profile-id={profile.id}>
                {spaces.map((s) => spaceButton(s, profile))}
              </div>
            ),
          )}
          {actions.createSpace && (
            <button type="button" className="nsb-space-add" title="New space" aria-label="New space" onClick={() => void addSpace()}>
              <Plus size={14} />
            </button>
          )}
        </nav>
        {children}
      </aside>

      {preview.state && !drag.dragging && (
        <TabPreviewCard
          preview={preview.state}
          onMouseEnter={preview.cardProps.onMouseEnter}
          onMouseLeave={preview.cardProps.onMouseLeave}
        />
      )}

      {drag.ghost && (
        <div
          ref={drag.ghostRef}
          className={`nsb-drag-ghost ${drag.ghost.item.kind === 'favorite' ? 'tile' : ''}`}
          aria-hidden="true"
          style={{ transform: `translate(${drag.ghost.x + 12}px, ${drag.ghost.y + 8}px)` }}
        >
          <Favicon src={drag.ghost.item.favicon ?? drag.ghost.item.tab?.favicon} />
          {drag.ghost.item.kind !== 'favorite' && (
            <span className="nsb-tab-title">{drag.ghost.item.title || drag.ghost.item.url || 'New Tab'}</span>
          )}
        </div>
      )}

      {menu && (
        <div
          ref={setMenuEl}
          className="nsb-menu"
          role="menu"
          style={{ left: menu.x, top: menu.y, width: MENU_WIDTH, visibility: menuReady ? 'visible' : 'hidden' }}
          onMouseDown={(e) => e.stopPropagation()}
          onContextMenu={(e) => e.preventDefault()}
        >
          {menu.showFolders ? (
            <>
              <button
                type="button"
                role="menuitem"
                className="nsb-menu-item nsb-menu-back"
                onClick={() => setMenu((m) => (m ? { ...m, showFolders: false } : m))}
              >
                <span>‹ Move to folder</span>
              </button>
              {menu.item.folderId && (
                <button
                  type="button"
                  role="menuitem"
                  className="nsb-menu-item"
                  onClick={() => {
                    actions.moveTabToFolder?.(menu.item.id, null);
                    setMenu(null);
                  }}
                >
                  <Pin size={13} />
                  <span>No folder (Root)</span>
                </button>
              )}
              {view.folders.map((f) => (
                <button
                  key={f.id}
                  type="button"
                  role="menuitem"
                  className="nsb-menu-item"
                  disabled={f.id === menu.item.folderId}
                  onClick={() => {
                    actions.moveTabToFolder?.(menu.item.id, f.id);
                    setMenu(null);
                  }}
                >
                  <Folder size={13} />
                  <span>{f.name}</span>
                </button>
              ))}
              <button
                type="button"
                role="menuitem"
                className="nsb-menu-item"
                onClick={() => {
                  const tabId = menu.item.id;
                  setMenu(null);
                  void createFolderAndRename(tabId);
                }}
              >
                <FolderPlus size={13} />
                <span>New folder with this tab</span>
              </button>
            </>
          ) : !menu.showSpaces ? (
            contextMenuActions(menu.item, { hasFolders: view.folders.length > 0 || menu.item.kind === 'pinned' }).map((action) => {
              const Icon = MENU_ICONS[action];
              return (
                <button
                  key={action}
                  type="button"
                  role="menuitem"
                  className={`nsb-menu-item ${action === 'close' ? 'danger' : ''}`}
                  disabled={action === 'move-to-space' && menuSpaces.length === 0}
                  onClick={() => runMenuAction(menu.item, action)}
                >
                  <Icon size={13} />
                  <span>{MENU_LABELS[action]}</span>
                  {(action === 'move-to-space' || action === 'move-to-folder') && <span className="nsb-menu-more">›</span>}
                </button>
              );
            })
          ) : (
            <>
              <button
                type="button"
                role="menuitem"
                className="nsb-menu-item nsb-menu-back"
                onClick={() => setMenu((m) => (m ? { ...m, showSpaces: false } : m))}
              >
                <span>‹ Move to space</span>
              </button>
              {view.groups.map(({ profile, spaces }) =>
                spaces
                  .filter((s) => menuSpaces.includes(s))
                  .map((s) => (
                    <button
                      key={s.id}
                      type="button"
                      role="menuitem"
                      className="nsb-menu-item"
                      onClick={() => {
                        actions.moveTab(moveToSpaceOp(menu.item, s.id));
                        setMenu(null);
                      }}
                    >
                      <SpaceIcon space={s} size={13} />
                      <span>{s.name}</span>
                      {profileCount > 1 && <span className="nsb-menu-hint">{profile.name}</span>}
                    </button>
                  )),
              )}
            </>
          )}
        </div>
      )}

      {folderMenu && (
        <div
          ref={setFolderMenuEl}
          className="nsb-menu"
          role="menu"
          style={{ left: folderMenu.x, top: folderMenu.y, width: MENU_WIDTH, visibility: folderMenuReady ? 'visible' : 'hidden' }}
          onMouseDown={(e) => e.stopPropagation()}
          onContextMenu={(e) => e.preventDefault()}
        >
          <button
            type="button"
            role="menuitem"
            className="nsb-menu-item"
            onClick={() => {
              const tabPromise = actions.createTab('serica://newtab', activeSpaceId);
              Promise.resolve(tabPromise).then((newTab: any) => {
                if (newTab?.id) {
                  actions.moveTabToFolder?.(newTab.id, folderMenu.folder.id);
                }
              });
              setFolderMenu(null);
            }}
          >
            <Plus size={13} />
            <span>New Tab Inside</span>
          </button>
          <button
            type="button"
            role="menuitem"
            className="nsb-menu-item"
            onClick={() => {
              setRenamingFolderId(folderMenu.folder.id);
              setRenameValue(folderMenu.folder.name);
              setFolderMenu(null);
            }}
          >
            <Edit2 size={13} />
            <span>Rename Folder</span>
          </button>
          <button
            type="button"
            role="menuitem"
            className="nsb-menu-item danger"
            onClick={() => {
              actions.deleteFolder?.(folderMenu.folder.id);
              setFolderMenu(null);
            }}
          >
            <Trash2 size={13} />
            <span>Delete Folder</span>
          </button>
        </div>
      )}

      {guideOpen && shown && <GuideTour id="sidebar" steps={SIDEBAR_GUIDE} onClose={() => setGuideOpen(false)} />}

      {spaceMenu &&
        (() => {
          const group = view.groups.find((g) => g.spaces.some((s) => s.id === spaceMenu.spaceId));
          const space = group?.spaces.find((s) => s.id === spaceMenu.spaceId);
          if (!group || !space) return null;
          const select = () => {
            if (space.id !== activeSpaceId) actions.selectSpace(space.id);
          };
          return (
            <div
              ref={setSpaceMenuEl}
              className="nsb-menu space-menu"
              role="menu"
              aria-label={`${space.name} actions`}
              style={{ left: spaceMenu.x, top: spaceMenu.y, width: MENU_WIDTH, visibility: spaceMenuReady ? 'visible' : 'hidden' }}
              onMouseDown={(e) => e.stopPropagation()}
              onContextMenu={(e) => e.preventDefault()}
            >
              <SpaceMenu
                key={space.id}
                space={space}
                index={group.spaces.indexOf(space)}
                count={group.spaces.length}
                tabCount={state.tabs.filter((t) => t.spaceId === space.id && !t.unloaded).length}
                icons={SPACE_ICON_NAMES}
                renderIcon={(icon, color) => <SpaceIcon space={{ icon, color, name: space.name }} size={15} />}
                onRename={() => {
                  select();
                  startRenameSpace(space);
                }}
                onUpdate={(patch) => actions.updateSpace?.(space.id, patch)}
                onMove={(to) => actions.reorderSpace?.(space.id, to)}
                onNewTab={() => {
                  select();
                  dismiss();
                  actions.openCommandBar();
                }}
                onNewFolder={() => {
                  select();
                  void (async () => {
                    const res: any = await actions.createFolder?.(space.id, 'New Folder');
                    const id = res?.folder?.id ?? res?.id;
                    if (id) {
                      setRenamingFolderId(id);
                      setRenameValue('New Folder');
                    }
                  })();
                }}
                onDelete={() => actions.deleteSpace?.(space.id)}
                onClose={() => setSpaceMenu(null)}
              />
            </div>
          );
        })()}
    </>
  );
};

export default Sidebar;
