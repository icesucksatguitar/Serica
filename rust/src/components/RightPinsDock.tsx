import { removeViewportHole, setViewportHole, useElementHole, useViewportCovered, useViewportOverlay } from '../hooks/useViewportOverlay';
import React, { useState, useEffect, useLayoutEffect, useRef, useCallback } from 'react';
import { Plus, X, Globe, EyeOff } from 'lucide-react';
import { addSettledBlurListener } from '../utils/settledBlur';

export interface PinnedSite {
  id: string;
  title: string;
  url: string;
  icon?: string;
}

const DEFAULT_PINS: PinnedSite[] = [
  { id: 'pin-yt', title: 'YouTube', url: 'https://www.youtube.com' },
  { id: 'pin-gh', title: 'GitHub', url: 'https://github.com' },
  { id: 'pin-x', title: 'X', url: 'https://x.com' },
  { id: 'pin-rd', title: 'Reddit', url: 'https://www.reddit.com' },
  { id: 'pin-notion', title: 'Notion', url: 'https://www.notion.so' },
];

const DOCK_MENU_WIDTH = 200;

interface RightPinsDockProps {
  onNavigate: (url: string) => void;
  isOnWebpage?: boolean;
  persistedPins?: PinnedSite[];
  onPersistPins?: (pins: PinnedSite[]) => void;
  /** Hides the dock (Settings → Appearance brings it back). */
  onHideDock?: () => void;
}

export const RightPinsDock: React.FC<RightPinsDockProps> = ({
  onNavigate,
  isOnWebpage = false,
  persistedPins,
  onPersistPins,
  onHideDock,
}) => {
  // A saved list wins even when empty (the user removed every site);
  // the defaults are only for a profile that never saved one.
  const [pins, setPins] = useState<PinnedSite[]>(() => {
    if (Array.isArray(persistedPins)) return persistedPins;
    try {
      const saved = (localStorage.getItem('serica_pinned_sites') || localStorage.getItem('neutrino_pinned_sites'));
      if (saved) {
        const parsed = JSON.parse(saved);
        if (Array.isArray(parsed)) return parsed;
      }
    } catch {}
    return DEFAULT_PINS;
  });

  useEffect(() => {
    if (Array.isArray(persistedPins)) {
      setPins(persistedPins);
      try {
        localStorage.setItem('serica_pinned_sites', JSON.stringify(persistedPins));
      } catch {}
    }
  }, [persistedPins]);

  const [scales, setScales] = useState<{ [id: string]: number }>({});
  const [isHoveredDock, setIsHoveredDock] = useState(false);
  const revealedAtRef = useRef<number>(0);
  const hideTimerRef = useRef<number | null>(null);
  const [isAddModalOpen, setIsAddModalOpen] = useState(false);
  const [menuPos, setMenuPos] = useState<{ x: number; y: number } | null>(null);
  const keepOpen = isAddModalOpen || menuPos !== null;

  // Pages are native views above the HTML UI. The dock and its menu show
  // through the live page (it is clipped around them); only the add dialog,
  // which dims the whole page, freezes it.
  useViewportOverlay(isAddModalOpen);
  const covered = useViewportCovered();
  // Slides in once the page is clipped for it (see the hole effect below).
  const [holeReady, setHoleReady] = useState(false);
  const dockShown = isHoveredDock && (holeReady || !isOnWebpage);
  const [menuEl, setMenuEl] = useState<HTMLDivElement | null>(null);
  const menuReady = useElementHole('dock-menu', menuPos ? menuEl : null);

  const revealDock = useCallback(() => {
    if (hideTimerRef.current !== null) {
      window.clearTimeout(hideTimerRef.current);
      hideTimerRef.current = null;
    }
    revealedAtRef.current = Date.now();
    setIsHoveredDock(true);
  }, []);

  const scheduleHide = useCallback((delay = 220) => {
    if (keepOpen) return;
    if (hideTimerRef.current !== null) {
      window.clearTimeout(hideTimerRef.current);
    }
    hideTimerRef.current = window.setTimeout(() => {
      const elapsed = Date.now() - revealedAtRef.current;
      if (elapsed < 350) {
        hideTimerRef.current = window.setTimeout(() => {
          setIsHoveredDock(false);
          hideTimerRef.current = null;
        }, 350 - elapsed);
      } else {
        setIsHoveredDock(false);
        hideTimerRef.current = null;
      }
    }, delay);
  }, [keepOpen]);

  const cancelHide = useCallback(() => {
    if (hideTimerRef.current !== null) {
      window.clearTimeout(hideTimerRef.current);
      hideTimerRef.current = null;
    }
  }, []);

  useEffect(() => {
    if (!isOnWebpage || !isHoveredDock) return;
    const onMove = (e: MouseEvent) => {
      if (keepOpen) return;
      // Keep open if within 180px of right edge (covers track, magnified pins and tooltips)
      const threshold = window.innerWidth - 180;
      if (e.clientX < threshold) {
        scheduleHide();
      } else {
        cancelHide();
      }
    };
    const removeBlur = addSettledBlurListener(() => {
      if (!keepOpen) scheduleHide(0);
    });
    window.addEventListener('mousemove', onMove, { passive: true });
    return () => {
      window.removeEventListener('mousemove', onMove);
      removeBlur();
      if (hideTimerRef.current !== null) {
        window.clearTimeout(hideTimerRef.current);
      }
    };
  }, [isOnWebpage, isHoveredDock, keepOpen, scheduleHide, cancelHide]);

  // Dock context menu: closes on any outside press, Escape or focus loss.
  useEffect(() => {
    if (!menuPos) return;
    const close = () => setMenuPos(null);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    window.addEventListener('mousedown', close);
    window.addEventListener('keydown', onKey);
    const removeBlur = addSettledBlurListener(close);
    return () => {
      window.removeEventListener('mousedown', close);
      window.removeEventListener('keydown', onKey);
      removeBlur();
    };
  }, [menuPos]);
  const [newUrl, setNewUrl] = useState('');
  const [newTitle, setNewTitle] = useState('');
  const [hoveredPinId, setHoveredPinId] = useState<string | null>(null);

  const dockRef = useRef<HTMLDivElement>(null);

  // The page is clipped to the dock's pill (same shape) and the clip follows
  // the dock's slide from the right (same duration and curve).
  const holeShown = useRef(false);
  useLayoutEffect(() => {
    const el = dockRef.current;
    const track = el?.querySelector<HTMLElement>('.vertical-pins-dock-track');
    if (!isOnWebpage || !el || !track) {
      if (holeShown.current) removeViewportHole('dock');
      holeShown.current = false;
      return;
    }
    const W = window.innerWidth;
    const w = track.offsetWidth;
    const h = track.offsetHeight;
    const left = W - el.offsetWidth; // the track sits at the container's left edge
    const top = window.innerHeight / 2 - h / 2;
    const shift = el.offsetWidth + 20; // the hidden dock's translateX
    const radius = parseFloat(getComputedStyle(track).borderTopLeftRadius) || 0;
    const shownRect: [number, number, number, number] = [left, top, w, h];
    const hiddenRect: [number, number, number, number] = [left + shift, top, w, h];
    const ease: [number, number, number, number] = [0.22, 1, 0.36, 1];
    if (isHoveredDock) {
      void setViewportHole('dock', shownRect, holeShown.current ? { radius } : { from: hiddenRect, ms: 320, ease, lead: 8, radius }).then(() =>
        setHoleReady(true),
      );
    } else {
      setHoleReady(false);
      if (holeShown.current) {
        void setViewportHole('dock', hiddenRect, { from: shownRect, ms: 320, ease, lead: -12, remove: true, radius });
      }
    }
    holeShown.current = isHoveredDock;
  }, [isOnWebpage, isHoveredDock, pins.length]);
  useEffect(() => () => removeViewportHole('dock'), []);

  const itemRefs = useRef<{ [id: string]: HTMLDivElement | null }>({});

  const savePins = (updated: PinnedSite[]) => {
    setPins(updated);
    try {
      localStorage.setItem('serica_pinned_sites', JSON.stringify(updated));
    } catch {}
    onPersistPins?.(updated);
  };

  // macOS Dock Fish-Eye Magnification physics
  const handleMouseMove = useCallback((e: React.MouseEvent<HTMLElement>) => {
    const dock = dockRef.current;
    if (!dock) return;

    const dockRect = dock.getBoundingClientRect();
    const mouseY = e.clientY;
    const maxRadius = 72; // Influence radius in px
    // On sites the page is clipped to the dock's pill: icons must stay inside it.
    const maxScale = isOnWebpage ? 1.2 : 1.48; // Peak magnification scale

    const newScales: { [id: string]: number } = {};

    [...pins.map(p => p.id), 'add-btn'].forEach((id) => {
      const el = itemRefs.current[id];
      if (!el) {
        newScales[id] = 1;
        return;
      }

      const rect = el.getBoundingClientRect();
      const centerY = rect.top + rect.height / 2;
      const dist = Math.abs(mouseY - centerY);

      if (dist < maxRadius) {
        // Cosine falloff curve for organic fluid magnification
        const factor = Math.cos((dist / maxRadius) * (Math.PI / 2));
        newScales[id] = 1 + (maxScale - 1) * Math.pow(factor, 1.5);
      } else {
        newScales[id] = 1;
      }
    });

    setScales(newScales);
  }, [pins, isOnWebpage]);

  const handleMouseLeave = () => {
    setScales({});
    setHoveredPinId(null);
  };

  const handleAddPin = (e: React.FormEvent) => {
    e.preventDefault();
    if (!newUrl.trim()) return;

    let formattedUrl = newUrl.trim();
    if (!formattedUrl.startsWith('http://') && !formattedUrl.startsWith('https://')) {
      formattedUrl = 'https://' + formattedUrl;
    }

    let title = newTitle.trim();
    if (!title) {
      try {
        const urlObj = new URL(formattedUrl);
        title = urlObj.hostname.replace('www.', '').split('.')[0];
        title = title.charAt(0).toUpperCase() + title.slice(1);
      } catch {
        title = 'Website';
      }
    }

    const newPin: PinnedSite = {
      id: `pin-${Date.now()}`,
      title,
      url: formattedUrl,
    };

    const updated = [...pins, newPin];
    savePins(updated);
    setNewUrl('');
    setNewTitle('');
    setIsAddModalOpen(false);
  };

  const handleDeletePin = (e: React.MouseEvent, id: string) => {
    e.stopPropagation();
    const updated = pins.filter((p) => p.id !== id);
    savePins(updated);
  };

  const getFaviconUrl = (url: string) => {
    try {
      const hostname = new URL(url).hostname;
      return `https://www.google.com/s2/favicons?domain=${hostname}&sz=64`;
    } catch {
      return '';
    }
  };

  return (
    <>
      {isOnWebpage && (
        <div
          className="right-pins-edge-hover-zone"
          onMouseEnter={revealDock}
          onMouseMove={revealDock}
        />
      )}
      <aside
        className={`vertical-pins-dock-container ${isOnWebpage ? 'on-webpage' : ''} ${
          isOnWebpage ? (dockShown ? 'revealed' : 'autohidden') : ''
        }`}
        ref={dockRef}
        onMouseEnter={() => {
          cancelHide();
          setIsHoveredDock(true);
        }}
        onMouseMove={(e) => {
          cancelHide();
          setIsHoveredDock(true);
          if (!menuPos) handleMouseMove(e);
        }}
        onMouseLeave={() => {
          scheduleHide();
          handleMouseLeave();
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          cancelHide();
          setScales({});
          setHoveredPinId(null);
          // Open beside the dock, never over it.
          const left = dockRef.current?.querySelector('.vertical-pins-dock-track')?.getBoundingClientRect().left ?? e.clientX;
          setMenuPos({ x: Math.max(8, left - DOCK_MENU_WIDTH - 10), y: Math.max(8, Math.min(e.clientY - 16, window.innerHeight - 124)) });
        }}
        aria-label="Pinned Websites Dock"
      >
        <div className="vertical-pins-dock-track">
          {pins.map((pin) => {
            const scale = scales[pin.id] || 1;
            const favicon = getFaviconUrl(pin.url);
            const isItemHovered = hoveredPinId === pin.id;

            return (
              <div
                key={pin.id}
                ref={(el) => (itemRefs.current[pin.id] = el)}
                className="pin-dock-item"
                style={{
                  transform: `scale(${scale}) translateX(${scale > 1.05 && !isOnWebpage ? -(scale - 1) * 14 : 0}px)`,
                  zIndex: Math.round(scale * 10),
                }}
                onClick={() => onNavigate(pin.url)}
                onMouseEnter={() => {
                  if (!menuPos) setHoveredPinId(pin.id);
                }}
                onMouseLeave={() => setHoveredPinId(null)}
                title={pin.title}
              >
                {/* Monochromatic Frosted Icon Capsule */}
                <div className="pin-dock-icon-capsule">
                  {favicon ? (
                    <img
                      src={favicon}
                      alt={pin.title}
                      className="pin-dock-favicon"
                      onError={(e) => {
                        (e.target as HTMLElement).style.display = 'none';
                      }}
                    />
                  ) : (
                    <Globe size={18} className="pin-dock-fallback-icon" />
                  )}

                  {/* Remove pill button when hovered */}
                  {isItemHovered && (
                    <button
                      className="pin-dock-remove-btn"
                      onClick={(e) => handleDeletePin(e, pin.id)}
                      title={`Unpin ${pin.title}`}
                    >
                      <X size={10} />
                    </button>
                  )}
                </div>

                {/* macOS Style Name Tooltip (on sites the native title tooltip is used: it draws above the page) */}
                {isItemHovered && !isOnWebpage && (
                  <div className="pin-dock-tooltip">
                    <span>{pin.title}</span>
                  </div>
                )}
              </div>
            );
          })}

          {/* Divider hairline */}
          <div className="pin-dock-divider" />

          {/* Add Pin + Icon Button */}
          <div
            ref={(el) => (itemRefs.current['add-btn'] = el)}
            className="pin-dock-item pin-dock-add-item"
            style={{
              transform: `scale(${scales['add-btn'] || 1})`,
            }}
            onClick={() => setIsAddModalOpen(true)}
            title="Add Website Pin"
          >
            <div className="pin-dock-icon-capsule add-capsule">
              <Plus size={16} />
            </div>
            {hoveredPinId === 'add-btn' && !isOnWebpage && (
              <div className="pin-dock-tooltip">
                <span>Add website</span>
              </div>
            )}
          </div>
        </div>
      </aside>

      {menuPos && (
        <div
          ref={setMenuEl}
          className="nsb-menu pin-dock-menu"
          role="menu"
          style={{ left: menuPos.x, top: menuPos.y, width: DOCK_MENU_WIDTH, visibility: menuReady ? 'visible' : 'hidden' }}
          onMouseDown={(e) => e.stopPropagation()}
          onContextMenu={(e) => e.preventDefault()}
        >
          <button
            type="button"
            role="menuitem"
            className="nsb-menu-item"
            onClick={() => {
              setMenuPos(null);
              setIsAddModalOpen(true);
            }}
          >
            <Plus size={13} />
            <span>Add website</span>
          </button>
          {onHideDock && (
            <button
              type="button"
              role="menuitem"
              className="nsb-menu-item"
              onClick={() => {
                setMenuPos(null);
                setIsHoveredDock(false);
                onHideDock();
              }}
            >
              <EyeOff size={13} />
              <span>Hide dock</span>
            </button>
          )}
          <div className="pin-dock-menu-hint">Turn it back on in Settings</div>
        </div>
      )}

      {/* Add Website Pin Modal */}
      {isAddModalOpen && covered && (
        <div className="pin-add-modal-backdrop" onClick={() => setIsAddModalOpen(false)}>
          <div className="pin-add-modal-dialog" onClick={(e) => e.stopPropagation()}>
            <div className="pin-add-modal-header">
              <h3>Pin Website</h3>
              <button
                className="pin-add-close-btn"
                onClick={() => setIsAddModalOpen(false)}
                aria-label="Close"
              >
                <X size={14} />
              </button>
            </div>

            <form onSubmit={handleAddPin} className="pin-add-modal-body">
              <div className="pin-add-field">
                <label htmlFor="pin-url">Website URL</label>
                <input
                  id="pin-url"
                  type="text"
                  autoFocus
                  placeholder="https://linear.app"
                  value={newUrl}
                  onChange={(e) => setNewUrl(e.target.value)}
                  className="pin-add-input"
                />
              </div>

              <div className="pin-add-field">
                <label htmlFor="pin-title">Display Name (Optional)</label>
                <input
                  id="pin-title"
                  type="text"
                  placeholder="Linear"
                  value={newTitle}
                  onChange={(e) => setNewTitle(e.target.value)}
                  className="pin-add-input"
                />
              </div>

              <div className="pin-add-actions">
                <button
                  type="button"
                  className="pin-add-cancel-btn"
                  onClick={() => setIsAddModalOpen(false)}
                >
                  Cancel
                </button>
                <button type="submit" className="pin-add-submit-btn">
                  Pin to Dock
                </button>
              </div>
            </form>
          </div>
        </div>
      )}
    </>
  );
};

