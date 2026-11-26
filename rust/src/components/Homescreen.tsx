import React, { useEffect, useRef, useState } from 'react';
import { HelpCircle, Image as ImageIcon, LayoutGrid, Plus, SlidersHorizontal } from 'lucide-react';
import {
  DEFAULT_HOME_BACKGROUND,
  DEFAULT_HOME_WIDGETS,
  DEFAULT_UI_SCALE,
  homeBackgroundCss,
  type HomeBackground,
  type HomeWidgets,
  type UiScale,
} from '../shared/settings';
import { WidgetBoard } from './home/WidgetBoard';
import { Switch } from './Switch';
import { startGuide } from './GuideTour';
import { hexLuminance } from '../shared/spaceTheme';
import { usePresence } from '../hooks/usePresence';

interface HomescreenProps {
  onNavigate: (url: string) => void;
  onOpenCommandBar?: () => void;
  widgets?: HomeWidgets;
  onUpdateWidgets?: (next: HomeWidgets) => void;
  background?: HomeBackground;
  uiScale?: UiScale;
  /** The website dock is on the right edge: keep widgets clear of it. */
  dockVisible?: boolean;
}

/** The bundled Serica Special artworks (oil-pastel re-paintings of 1789-1794 scenes). */
export const SPECIAL_ART = [
  { src: '/artworks/serica-special-1.avif', thumb: '/artworks/serica-special-1-thumb.webp', title: 'Prise de la Bastille', after: 'after Jean-Pierre Houël, 1789' },
  { src: '/artworks/serica-special-2.avif', thumb: '/artworks/serica-special-2-thumb.webp', title: 'La Bastille, premiers jours de sa démolition', after: 'after Hubert Robert, 1789' },
  { src: '/artworks/serica-special-3.avif', thumb: '/artworks/serica-special-3-thumb.webp', title: 'Fête de l’Être suprême', after: 'after Pierre-Antoine Demachy, 1794' },
];

const BG_URL = (file: string) => `http://sericabg.localhost/${encodeURIComponent(file)}`;

/** Whether text on this background should be light. */
function wantsLightInk(bg: HomeBackground, themeDark: boolean): boolean {
  if (bg.mode === 'image' || bg.mode === 'special') return true;
  if (bg.mode === 'solid') return hexLuminance(bg.solid) < 0.42;
  if (bg.mode === 'gradient') {
    const avg = bg.gradient.stops.reduce((a, s) => a + hexLuminance(s.color), 0) / bg.gradient.stops.length;
    return avg < 0.5;
  }
  return themeDark;
}

/**
 * Serica Special: the three artworks, each held for `seconds`, cross-fading
 * into the next. The next image is decoded before the fade starts; nothing
 * runs while the window is hidden.
 */
const SpecialSlideshow: React.FC<{ seconds: number }> = ({ seconds }) => {
  const [index, setIndex] = useState(() => {
    try {
      const n = (Number(localStorage.getItem('serica_special_index')) + 1) % SPECIAL_ART.length;
      localStorage.setItem('serica_special_index', String(n));
      return Number.isFinite(n) ? n : 0;
    } catch {
      return 0;
    }
  });
  // Two layers: the one shown, and the next fading in over it.
  const [layers, setLayers] = useState<{ a: number; b: number; top: 'a' | 'b' }>({ a: index, b: (index + 1) % SPECIAL_ART.length, top: 'a' });
  useEffect(() => {
    let alive = true;
    let timer = 0;
    const next = () => {
      if (!alive) return;
      if (document.hidden) {
        timer = window.setTimeout(next, 2000);
        return;
      }
      const n = (index + 1) % SPECIAL_ART.length;
      const img = new Image();
      img.src = SPECIAL_ART[n].src;
      void img
        .decode()
        .catch(() => undefined)
        .then(() => {
          if (!alive) return;
          setLayers((l) => (l.top === 'a' ? { ...l, b: n, top: 'b' } : { ...l, a: n, top: 'a' }));
          setIndex(n);
        });
    };
    timer = window.setTimeout(next, seconds * 1000);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [index, seconds]);
  const art = SPECIAL_ART[index];
  return (
    <>
      <img className={`home-art${layers.top === 'a' ? ' on' : ''}`} src={SPECIAL_ART[layers.a].src} alt="" draggable={false} decoding="async" />
      <img className={`home-art${layers.top === 'b' ? ' on' : ''}`} src={SPECIAL_ART[layers.b].src} alt="" draggable={false} decoding="async" />
      <div className="home-art-credit" key={index}>
        <span>{art.title}</span>
        <span className="home-art-after">{art.after} · oil pastel</span>
      </div>
    </>
  );
};

/** What fills the page behind the widgets (Settings › Home). */
export const HomeBackdrop: React.FC<{ bg: HomeBackground }> = ({ bg }) => {
  const css = homeBackgroundCss(bg);
  if (bg.mode === 'theme') return null;
  return (
    <div className={`home-bg mode-${bg.mode}`} aria-hidden="true">
      {css && <div className="home-bg-fill" style={{ background: css }} />}
      {bg.mode === 'image' && bg.image.file && (
        <>
          <div className={`home-bg-image fit-${bg.image.fit}`} style={{ backgroundImage: `url("${BG_URL(bg.image.file)}")` }} />
          <div className="home-bg-dim" style={{ opacity: bg.image.dim }} />
        </>
      )}
      {bg.mode === 'special' && <SpecialSlideshow seconds={bg.special.seconds} />}
      {(bg.mode === 'image' || bg.mode === 'special') && <div className="home-bg-scrim" />}
    </div>
  );
};

function clockParts(now: Date, hour12: boolean) {
  let h = now.getHours();
  let suffix = '';
  if (hour12) {
    suffix = h < 12 ? 'AM' : 'PM';
    h = h % 12 || 12;
  }
  return {
    hm: `${hour12 ? h : String(h).padStart(2, '0')}:${String(now.getMinutes()).padStart(2, '0')}`,
    seconds: String(now.getSeconds()).padStart(2, '0'),
    suffix,
    date: now.toLocaleDateString(undefined, { weekday: 'long', month: 'long', day: 'numeric' }),
  };
}

/** The clock, fixed in the bottom-left corner. Updates text in place once a second. */
const HomeClock: React.FC<{ hour12: boolean; seconds: boolean }> = ({ hour12, seconds }) => {
  const initial = useRef(clockParts(new Date(), hour12)).current;
  const hmRef = useRef<HTMLSpanElement>(null);
  const secRef = useRef<HTMLSpanElement>(null);
  const sufRef = useRef<HTMLSpanElement>(null);
  const dateRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let last = '';
    const tick = () => {
      if (document.hidden) return;
      const p = clockParts(new Date(), hour12);
      if (hmRef.current && hmRef.current.textContent !== p.hm) hmRef.current.textContent = p.hm;
      if (secRef.current) secRef.current.textContent = p.seconds;
      if (sufRef.current) sufRef.current.textContent = p.suffix;
      if (dateRef.current && p.date !== last) {
        dateRef.current.textContent = p.date;
        last = p.date;
      }
    };
    tick();
    // Aligned to the second so the digits change with the system clock.
    let t = 0;
    const loop = () => {
      tick();
      t = window.setTimeout(loop, 1000 - (Date.now() % 1000) + 5);
    };
    t = window.setTimeout(loop, 1000 - (Date.now() % 1000) + 5);
    const onVis = () => !document.hidden && tick();
    document.addEventListener('visibilitychange', onVis);
    return () => {
      window.clearTimeout(t);
      document.removeEventListener('visibilitychange', onVis);
    };
  }, [hour12]);
  return (
    <div className="home-clock" aria-live="off">
      <div className="home-clock-time tnum">
        <span ref={hmRef}>{initial.hm}</span>
        {seconds && (
          <span className="home-clock-sec" ref={secRef}>
            {initial.seconds}
          </span>
        )}
        {hour12 && (
          <span className="home-clock-suffix" ref={sufRef}>
            {initial.suffix}
          </span>
        )}
      </div>
      <div className="home-clock-date" ref={dateRef}>
        {initial.date}
      </div>
    </div>
  );
};

/**
 * The new-tab page: only the widgets and the clock, on the background chosen
 * in Settings › Home. Typing anywhere goes to the address bar.
 */
export const Homescreen: React.FC<HomescreenProps> = ({
  onNavigate,
  onOpenCommandBar,
  widgets = DEFAULT_HOME_WIDGETS,
  onUpdateWidgets,
  background = DEFAULT_HOME_BACKGROUND,
  uiScale = DEFAULT_UI_SCALE,
  dockVisible = false,
}) => {
  const [menuOpen, setMenuOpen] = useState(false);
  const [editing, setEditing] = useState(false);
  const [galleryOpen, setGalleryOpen] = useState(false);
  const menu = usePresence(menuOpen, 130);
  const themeDark = typeof document !== 'undefined' && document.documentElement.getAttribute('data-theme') !== 'light';
  const lightInk = wantsLightInk(background, themeDark);

  // Typing anywhere on the page goes to the address bar.
  useEffect(() => {
    if (editing) return;
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)) return;
      if (!e.ctrlKey && !e.metaKey && !e.altKey && e.key.length === 1 && e.key !== ' ') onOpenCommandBar?.();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [editing, onOpenCommandBar]);

  useEffect(() => {
    if (!menuOpen) return;
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key !== 'Escape') return;
      if (e instanceof MouseEvent && (e.target as HTMLElement)?.closest?.('.home-customize2')) return;
      setMenuOpen(false);
    };
    window.addEventListener('keydown', close);
    window.addEventListener('mousedown', close);
    return () => {
      window.removeEventListener('keydown', close);
      window.removeEventListener('mousedown', close);
    };
  }, [menuOpen]);

  const setClock = (patch: Partial<HomeWidgets['clock']>) => onUpdateWidgets?.({ ...widgets, clock: { ...widgets.clock, ...patch } });

  return (
    <div className={`home2${editing ? ' editing' : ''}${dockVisible ? ' with-dock' : ''}`} data-ink={lightInk ? 'light' : 'dark'} data-bg={background.mode}>
      <HomeBackdrop bg={background} />

      {onUpdateWidgets && (
        <div className="home2-board">
          <WidgetBoard
            widgets={widgets}
            onUpdate={onUpdateWidgets}
            onOpen={onNavigate}
            editing={editing}
            onEditingChange={setEditing}
            galleryOpen={galleryOpen}
            onGalleryOpenChange={setGalleryOpen}
            scale={uiScale.widgets}
          />
        </div>
      )}

      {widgets.items.length === 0 && !editing && onUpdateWidgets && (
        <button type="button" className="home2-empty" onClick={() => { setEditing(true); setGalleryOpen(true); }}>
          <Plus size={14} aria-hidden="true" />
          Add a widget
        </button>
      )}

      <HomeClock hour12={widgets.clock.hour12} seconds={widgets.clock.seconds} />

      {onUpdateWidgets && !editing && (
        <div className="home-customize2">
          {menu.mounted && (
            <div className="presence" data-state={menu.state}>
              <div className="home-customize-panel2 nsb-menu" role="menu" aria-label="Customize new tab">
                <button type="button" role="menuitem" className="nsb-menu-item" onClick={() => { setMenuOpen(false); setEditing(true); }}>
                  <LayoutGrid size={13} />
                  <span>Edit widgets</span>
                </button>
                <button type="button" role="menuitem" className="nsb-menu-item" onClick={() => { setMenuOpen(false); setEditing(true); setGalleryOpen(true); }}>
                  <Plus size={13} />
                  <span>Add a widget</span>
                </button>
                <button
                  type="button"
                  role="menuitem"
                  className="nsb-menu-item"
                  onClick={() => {
                    setMenuOpen(false);
                    window.dispatchEvent(new CustomEvent('serica:open-settings', { detail: 'home' }));
                  }}
                >
                  <ImageIcon size={13} />
                  <span>Background…</span>
                </button>
                <div className="app-context-menu-sep" role="separator" />
                <div className="home-customize-row">
                  <span>24-hour clock</span>
                  <Switch checked={!widgets.clock.hour12} label="24-hour clock" onChange={(on) => setClock({ hour12: !on })} />
                </div>
                <div className="home-customize-row">
                  <span>Seconds</span>
                  <Switch checked={widgets.clock.seconds} label="Seconds" onChange={(seconds) => setClock({ seconds })} />
                </div>
                <div className="app-context-menu-sep" role="separator" />
                <button type="button" role="menuitem" className="nsb-menu-item" onClick={() => { setMenuOpen(false); startGuide('widgets'); }}>
                  <HelpCircle size={13} />
                  <span>How widgets work</span>
                </button>
              </div>
            </div>
          )}
          <button type="button" className={`home-customize-btn2${menuOpen ? ' active' : ''}`} aria-expanded={menuOpen} aria-haspopup="menu" onClick={() => setMenuOpen((c) => !c)}>
            <SlidersHorizontal size={13} aria-hidden="true" />
            <span>Customize</span>
          </button>
        </div>
      )}
    </div>
  );
};
