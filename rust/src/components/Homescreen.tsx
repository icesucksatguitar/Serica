import React, { useState, useEffect, useCallback, useRef } from 'react';
import { HelpCircle, LayoutGrid, SlidersHorizontal } from 'lucide-react';
import { DEFAULT_HOME_WIDGETS, DEFAULT_UI_SCALE, type HomeWidgets, type UiScale } from '../shared/settings';
import { WidgetBoard } from './home/WidgetBoard';
import { Switch } from './Switch';
import { startGuide } from './GuideTour';

interface HomescreenProps {
  isDarkMode?: boolean;
  onNavigate: (url: string) => void;
  onOpenCommandBar?: () => void;
  userName?: string;
  widgets?: HomeWidgets;
  onUpdateWidgets?: (next: HomeWidgets) => void;
  uiScale?: UiScale;
}

function formatClockParts(now: Date) {
  return {
    hm: `${String(now.getHours()).padStart(2, '0')}:${String(now.getMinutes()).padStart(2, '0')}`,
    seconds: String(now.getSeconds()).padStart(2, '0'),
    dateStr: now.toLocaleDateString('en-US', { weekday: 'long', month: 'long', day: 'numeric' }),
  };
}

/**
 * The new tab page. The greeting, clock and search field sit in one place
 * whatever else is on the page; widgets go below them and the page scrolls
 * when they need the room. Customize (bottom right) shows or hides each part
 * and edits the widgets.
 */
export const Homescreen: React.FC<HomescreenProps> = ({
  onNavigate,
  onOpenCommandBar,
  userName,
  widgets = DEFAULT_HOME_WIDGETS,
  onUpdateWidgets,
  uiScale = DEFAULT_UI_SCALE,
}) => {
  const [menuOpen, setMenuOpen] = useState(false);
  const [editing, setEditing] = useState(false);

  const [displayName, setDisplayName] = useState<string>(() => {
    if (userName) return userName;
    try {
      const saved = localStorage.getItem('serica_user_profile') || localStorage.getItem('neutrino_user_profile');
      if (saved) {
        const parsed = JSON.parse(saved);
        if (parsed.name) return parsed.name;
      }
    } catch {}
    return '';
  });
  useEffect(() => {
    if (userName) setDisplayName(userName);
  }, [userName]);

  const initialClock = useRef(formatClockParts(new Date())).current;
  const dateRef = useRef<HTMLDivElement>(null);
  const hmRef = useRef<HTMLSpanElement>(null);
  const secRef = useRef<HTMLSpanElement>(null);
  const greetRef = useRef<HTMLDivElement>(null);

  const getGreeting = useCallback((): string => {
    const hour = new Date().getHours();
    const who = displayName ? `, ${displayName}` : '';
    if (hour < 12) return `Good morning${who}`;
    if (hour < 17) return `Good afternoon${who}`;
    return `Good evening${who}`;
  }, [displayName]);

  useEffect(() => {
    if (!widgets.clock) return;
    let lastDate = '';
    let lastHm = '';
    let lastGreet = '';
    const tick = () => {
      if (document.hidden) return;
      const parts = formatClockParts(new Date());
      if (hmRef.current && parts.hm !== lastHm) {
        hmRef.current.textContent = parts.hm;
        lastHm = parts.hm;
      }
      if (secRef.current) secRef.current.textContent = parts.seconds;
      if (dateRef.current && parts.dateStr !== lastDate) {
        dateRef.current.textContent = parts.dateStr;
        lastDate = parts.dateStr;
      }
      if (greetRef.current) {
        const g = getGreeting();
        if (g !== lastGreet) {
          greetRef.current.textContent = g;
          lastGreet = g;
        }
      }
    };
    tick();
    const interval = setInterval(tick, 1000);
    const onVis = () => {
      if (!document.hidden) tick();
    };
    document.addEventListener('visibilitychange', onVis);
    return () => {
      clearInterval(interval);
      document.removeEventListener('visibilitychange', onVis);
    };
  }, [getGreeting, widgets.clock]);

  // Typing anywhere on an empty new tab starts a search, also with the field hidden.
  useEffect(() => {
    if (editing) return;
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return;
      if (!e.ctrlKey && !e.metaKey && !e.altKey && e.key.length === 1 && !widgets.search) onOpenCommandBar?.();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [editing, widgets.search, onOpenCommandBar]);

  useEffect(() => {
    if (!menuOpen) return;
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key !== 'Escape') return;
      if (e instanceof MouseEvent && (e.target as HTMLElement)?.closest?.('.home-customize')) return;
      setMenuOpen(false);
    };
    window.addEventListener('keydown', close);
    window.addEventListener('mousedown', close);
    return () => {
      window.removeEventListener('keydown', close);
      window.removeEventListener('mousedown', close);
    };
  }, [menuOpen]);

  const update = (patch: Partial<HomeWidgets>) => onUpdateWidgets?.({ ...widgets, ...patch });
  const hasWidgets = widgets.items.length > 0 || editing;

  return (
    <div
      className={`serica-minimal-newtab neutrino-minimal-newtab${hasWidgets ? ' with-widgets' : ''}${editing ? ' editing-widgets' : ''}`}
      style={
        {
          '--hero-scale': uiScale.hero,
          '--search-scale': uiScale.search,
        } as React.CSSProperties
      }
    >
      <div className="ambient-glow-layer" aria-hidden="true">
        <div className="ambient-glow-color-cycle">
          <div className="ambient-glow-container">
            <div className="ambient-glow-blob blob-1" />
            <div className="ambient-glow-blob blob-2" />
            <div className="ambient-glow-blob blob-3" />
            <div className="ambient-glow-blob blob-4" />
          </div>
        </div>
      </div>
      <div className="newtab-grain-veil" />

      <div className="home-scroll">
        <div className="homescreen-hero-center">
          {widgets.clock && (
            <div className="home-hero">
              <div className="yc-italic-greeting" ref={greetRef}>
                {getGreeting()}
              </div>
              <div className="digital-clock-container compact-clock">
                <div className="digital-clock-date" ref={dateRef}>
                  {initialClock.dateStr}
                </div>
                <div className="digital-clock-time">
                  <span className="digital-clock-digits" ref={hmRef}>
                    {initialClock.hm}
                  </span>
                  <span className="digital-clock-seconds" ref={secRef}>
                    {initialClock.seconds}
                  </span>
                </div>
              </div>
            </div>
          )}

          {widgets.search && (
            <div className="newtab-search-reveal-wrapper revealed home-search">
              <div
                className="newtab-prompt-trigger monochromatic-pill"
                tabIndex={0}
                role="button"
                onClick={(e) => {
                  e.stopPropagation();
                  onOpenCommandBar?.();
                }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    e.stopPropagation();
                    onOpenCommandBar?.();
                  }
                }}
                title="Search or enter URL (Ctrl+K or Ctrl+T)"
              >
                <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
                  <circle cx="11" cy="11" r="8" />
                  <line x1="21" y1="21" x2="16.65" y2="16.65" />
                </svg>
                <span className="prompt-text">Search or enter URL</span>
                <div className="prompt-kbd-badge">
                  <kbd className="prompt-kbd">Ctrl K</kbd>
                </div>
              </div>
            </div>
          )}
        </div>

        {onUpdateWidgets && (
          <WidgetBoard
            widgets={widgets}
            onUpdate={onUpdateWidgets}
            onOpen={onNavigate}
            editing={editing}
            onEditingChange={setEditing}
            scale={uiScale.widgets}
          />
        )}
      </div>

      {onUpdateWidgets && !editing && (
        <div className="home-customize">
          {menuOpen && (
            <div className="home-customize-panel nsb-menu" role="menu" aria-label="Customize new tab">
              <div className="home-customize-row">
                <span>Greeting and clock</span>
                <Switch checked={widgets.clock} label="Greeting and clock" onChange={(clock) => update({ clock })} />
              </div>
              <div className="home-customize-row">
                <span>Search bar</span>
                <Switch checked={widgets.search} label="Search bar" onChange={(search) => update({ search })} />
              </div>
              <div className="app-context-menu-sep" role="separator" />
              <button
                type="button"
                role="menuitem"
                className="nsb-menu-item"
                onClick={() => {
                  setMenuOpen(false);
                  setEditing(true);
                }}
              >
                <LayoutGrid size={13} />
                <span>Edit widgets</span>
              </button>
              <button
                type="button"
                role="menuitem"
                className="nsb-menu-item"
                onClick={() => {
                  setMenuOpen(false);
                  startGuide('widgets');
                }}
              >
                <HelpCircle size={13} />
                <span>How widgets work</span>
              </button>
            </div>
          )}
          <button
            type="button"
            className={`home-customize-btn${menuOpen ? ' active' : ''}`}
            aria-expanded={menuOpen}
            aria-haspopup="menu"
            onClick={() => setMenuOpen((c) => !c)}
          >
            <SlidersHorizontal size={13} aria-hidden="true" />
            <span>Customize</span>
          </button>
        </div>
      )}
    </div>
  );
};
