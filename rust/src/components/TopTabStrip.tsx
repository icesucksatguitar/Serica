import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Globe, Plus, X } from 'lucide-react';
import type { BrowserAppState } from '../types';
import { buildSidebarView, type SidebarItemRef } from '../utils/sidebarModel';
import { useHoverPreview } from '../hooks/useHoverPreview';
import { useViewportOverlay } from '../hooks/useViewportOverlay';
import { TabPreviewCard } from './TabPreviewCard';
import { SpaceIcon } from './Sidebar';

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
}) => {
  const view = useMemo(() => buildSidebarView(state), [state]);
  const preview = useHoverPreview(false);
  // The preview card hangs over the native page; freeze the page while it shows.
  useViewportOverlay(Boolean(preview.state));
  const scrollRef = useRef<HTMLDivElement>(null);

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
        className={`ttab${pinned ? ' pinned' : ''}${active ? ' active' : ''}${item.tab?.isSleeping ? ' sleeping' : ''}`}
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
          }
        }}
        {...props}
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
