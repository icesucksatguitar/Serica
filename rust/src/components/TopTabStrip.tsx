import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Globe, Plus, X } from 'lucide-react';
import type { BrowserAppState, MoveOp } from '../types';
import {
  buildSidebarView,
  contextMenuActions,
  moveToSpaceOp,
  type SidebarItemRef,
  type SidebarMenuAction,
} from '../utils/sidebarModel';
import { useHoverPreview } from '../hooks/useHoverPreview';
import { useElementHole, useViewportOverlay } from '../hooks/useViewportOverlay';
import { addSettledBlurListener } from '../utils/settledBlur';
import { TabPreviewCard } from './TabPreviewCard';
import { MENU_ICONS, MENU_LABELS, SpaceIcon } from './Sidebar';

/** The tab commands the strip's right-click menu issues (names match `useBrowser()`). */
export interface TopTabActions {
  pinTab: (tabId: string) => unknown;
  unpinTab: (tabId: string) => unknown;
  addFavorite: (tabId: string) => unknown;
  duplicateTab: (id: string, spaceId?: string) => unknown;
  moveTab: (op: MoveOp) => unknown;
}

const MENU_WIDTH = 210;

interface MenuState {
  item: SidebarItemRef;
  x: number;
  y: number;
  showSpaces: boolean;
}

interface TopTabStripProps {
  state: BrowserAppState;
  /** True while a web page is shown (the strip then sits on a solid bar). */
  onWebpage: boolean;
  /** The top bar's page-color tint (CSS variables), so bar and strip read as one. */
  tint?: React.CSSProperties;
  onSelectTab: (tabId: string) => void;
  onCloseTab: (tabId: string) => void;
  onSelectSpace: (spaceId: string) => void;
  onNewTab: () => void;
  /** Right-click menu commands (same menu as the sidebar's tabs). */
  actions: TopTabActions;
}

const Favicon: React.FC<{ src?: string }> = ({ src }) => {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [src]);
  if (!src || failed) return <Globe size={14} className="ttab-favicon-fallback" aria-hidden="true" />;
  return <img src={src} alt="" width={16} height={16} className="ttab-favicon" onError={() => setFailed(true)} />;
};

/** Horizontal tab strip used when Settings → Appearance → Tab layout is "Top tabs". */
export const TopTabStrip: React.FC<TopTabStripProps> = ({
  state,
  onWebpage,
  tint,
  onSelectTab,
  onCloseTab,
  onSelectSpace,
  onNewTab,
  actions,
}) => {
  const view = useMemo(() => buildSidebarView(state), [state]);
  const preview = useHoverPreview(false);
  // The preview card hangs over the native page; freeze the page while it shows.
  useViewportOverlay(Boolean(preview.state));
  const scrollRef = useRef<HTMLDivElement>(null);

  // --- Right-click menu (same actions as the sidebar's) ------------------------
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [menuEl, setMenuEl] = useState<HTMLDivElement | null>(null);
  // The menu drops over the native page: cut it out so it is not hidden under it.
  const menuReady = useElementHole('top-tab-menu', menu ? menuEl : null);

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

  // Close the menu if its tab disappears.
  const menuTabId = menu?.item.id;
  useEffect(() => {
    if (menuTabId && !state.tabs.some((t) => t.id === menuTabId)) setMenu(null);
  }, [menuTabId, state.tabs]);

  const openMenu = (item: SidebarItemRef, x: number, y: number) => {
    preview.hide();
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
      case 'duplicate':
        actions.duplicateTab(item.id);
        break;
      case 'close':
        onCloseTab(item.id);
        break;
      case 'move-to-space':
        setMenu((m) => (m ? { ...m, showSpaces: true } : m));
        return;
      default:
        break;
    }
    setMenu(null);
  };

  // The strip shows no folders, so "Move to folder" stays in the sidebar.
  const menuActions = menu
    ? contextMenuActions(menu.item, { hasFolders: false }).filter((a) => a !== 'move-to-folder')
    : [];
  const menuSpaces = menu ? view.spaceOrder.filter((s) => s.id !== menu.item.tab?.spaceId) : [];
  const profileCount = view.groups.length;

  // Hide the card if its tab disappears.
  const hidePreview = preview.hide;
  const previewKey = preview.state?.target.key;
  useEffect(() => {
    if (previewKey && !state.tabs.some((t) => t.id === previewKey)) hidePreview();
  }, [previewKey, state.tabs, hidePreview]);

  // Keep the active tab in view.
  useEffect(() => {
    scrollRef.current?.querySelector('.ttab.active')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }, [state.activeTabId]);

  const renderTab = (item: SidebarItemRef, pinned: boolean) => {
    const active = item.id === state.activeTabId;
    const props = preview.itemProps(item, 'below');
    return (
      <div
        key={item.id}
        role="tab"
        aria-selected={active}
        tabIndex={0}
        className={`ttab${pinned ? ' pinned' : ''}${active ? ' active' : ''}`}
        title={pinned ? item.title : undefined}
        onClick={() => onSelectTab(item.id)}
        onAuxClick={(e) => {
          if (e.button === 1 && !pinned) {
            e.preventDefault();
            preview.hide();
            onCloseTab(item.id);
          }
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            onSelectTab(item.id);
          } else if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
            e.preventDefault();
            const r = e.currentTarget.getBoundingClientRect();
            openMenu(item, r.left + 12, r.bottom);
          }
        }}
        {...props}
        onContextMenu={(e) => {
          e.preventDefault();
          openMenu(item, e.clientX, e.clientY);
        }}
      >
        <Favicon src={item.favicon ?? item.tab?.favicon} />
        {!pinned && <span className="ttab-title">{item.title || 'New Tab'}</span>}
        {!pinned && (
          <button
            type="button"
            className="ttab-close"
            aria-label={`Close ${item.title || 'tab'}`}
            onClick={(e) => {
              e.stopPropagation();
              preview.hide();
              onCloseTab(item.id);
            }}
          >
            <X size={12} />
          </button>
        )}
      </div>
    );
  };

  return (
    <>
      <div
        className={`top-tab-strip${onWebpage ? ' on-webpage' : ''}${tint ? ' topbar-tinted' : ''}`}
        style={tint}
        role="tablist"
        aria-label="Tabs"
      >
        <div className="ttab-spaces" role="group" aria-label="Spaces">
          {view.spaceOrder.map((space) => {
            const active = space.id === state.activeSpaceId;
            return (
              <button
                key={space.id}
                type="button"
                className={`ttab-space${active ? ' active' : ''}`}
                title={space.name}
                aria-label={`Space ${space.name}`}
                aria-pressed={active}
                onClick={() => !active && onSelectSpace(space.id)}
              >
                <SpaceIcon space={space} size={13} />
                {active && <span className="ttab-space-name">{space.name}</span>}
              </button>
            );
          })}
        </div>
        <div className="ttab-divider" aria-hidden="true" />
        <div
          className="ttab-scroll"
          ref={scrollRef}
          onWheel={(e) => {
            if (scrollRef.current && Math.abs(e.deltaY) > Math.abs(e.deltaX)) scrollRef.current.scrollLeft += e.deltaY;
          }}
        >
          {view.pinned.map((item) => renderTab(item, true))}
          {view.pinned.length > 0 && view.today.length > 0 && <div className="ttab-divider small" aria-hidden="true" />}
          {view.today.map((item) => renderTab(item, false))}
          <button type="button" className="ttab-new" onClick={onNewTab} title="New tab (Ctrl+T)" aria-label="New tab">
            <Plus size={15} />
          </button>
        </div>
      </div>

      {menu && (
        <div
          ref={setMenuEl}
          className="nsb-menu"
          role="menu"
          style={{ left: menu.x, top: menu.y, width: MENU_WIDTH, visibility: menuReady ? 'visible' : 'hidden' }}
          onMouseDown={(e) => e.stopPropagation()}
          onContextMenu={(e) => e.preventDefault()}
        >
          {!menu.showSpaces ? (
            menuActions.map((action) => {
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
                  {action === 'move-to-space' && <span className="nsb-menu-more">›</span>}
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

      {preview.state && (
        <TabPreviewCard
          preview={preview.state}
          onMouseEnter={preview.cardProps.onMouseEnter}
          onMouseLeave={preview.cardProps.onMouseLeave}
        />
      )}
    </>
  );
};
