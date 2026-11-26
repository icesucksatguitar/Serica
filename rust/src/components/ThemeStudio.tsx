import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Check, Monitor, Moon, RotateCcw, Sun, X } from 'lucide-react';
import type { Space } from '../types';
import {
  computeThemePalette,
  getDefaultSpaceTheme,
  hexToHsl,
  hslToHex,
  type SpaceTheme,
  type SpaceThemeMode,
  type ThemePalette,
} from '../shared/spaceTheme';
import { getOsPrefersDark } from '../utils/applySpaceTheme';

/** A curated colour family: its hue and how much colour it carries. */
interface Family {
  id: string;
  name: string;
  hue: number;
  sat: number;
}

export const FAMILIES: Family[] = [
  { id: 'graphite', name: 'Graphite', hue: 220, sat: 6 },
  { id: 'slate', name: 'Slate', hue: 212, sat: 18 },
  { id: 'harbor', name: 'Harbor', hue: 200, sat: 48 },
  { id: 'indigo', name: 'Indigo', hue: 232, sat: 44 },
  { id: 'plum', name: 'Plum', hue: 292, sat: 30 },
  { id: 'rose', name: 'Rose', hue: 346, sat: 40 },
  { id: 'ember', name: 'Ember', hue: 18, sat: 56 },
  { id: 'ochre', name: 'Ochre', hue: 38, sat: 52 },
  { id: 'sand', name: 'Sand', hue: 34, sat: 22 },
  { id: 'moss', name: 'Moss', hue: 92, sat: 26 },
  { id: 'pine', name: 'Pine', hue: 156, sat: 40 },
  { id: 'teal', name: 'Teal', hue: 178, sat: 40 },
];

/** Tint: how strongly the colours show through the interface. */
const TINTS = [
  { id: 'subtle', label: 'Subtle', value: 0.34 },
  { id: 'balanced', label: 'Balanced', value: 0.58 },
  { id: 'vivid', label: 'Vivid', value: 0.82 },
] as const;

/** 18 hues for "your own colour", evenly around the wheel. */
const OWN_HUES = Array.from({ length: 18 }, (_, i) => i * 20);

/**
 * The three theme points for a hue. Deep, close colours for dark mode and
 * soft ones for light mode, so text contrast holds either way (the palette
 * code then picks the text colour with the best worst-case contrast).
 */
export function familyTheme(hue: number, sat: number, mode: SpaceThemeMode, intensity: number, dark: boolean): SpaceTheme {
  const L = dark ? 22 : 86;
  const step = dark ? 6 : -4;
  const pt = (h: number, s: number, l: number, x: number, y: number) => ({ color: hslToHex(((h % 360) + 360) % 360, Math.max(0, Math.min(100, s)), l), x, y });
  return {
    mode,
    intensity,
    grain: 0,
    points: [pt(hue, sat, L, 0.3, 0.35), pt(hue + 16, sat * 0.9, L + step, 0.7, 0.4), pt(hue - 14, sat * 0.8, L - step, 0.5, 0.75)],
    glow: { style: 'off', color: '#000000', intensity: 0, speed: 0, size: 0 },
  };
}

/**
 * A theme moved to the other side (light or dark): the same hues, at the
 * lightness Studio's own palettes use there. Only flipping the mode would
 * leave dark colours under a light UI (or the reverse): a muddy mid-tone
 * surface with poor contrast.
 */
export function retoneTheme(theme: SpaceTheme, mode: SpaceThemeMode, dark: boolean): SpaceTheme {
  const L = dark ? 22 : 86;
  const step = dark ? 6 : -4;
  const offsets = [0, step, -step];
  return {
    ...theme,
    mode,
    points: theme.points.map((p, i) => {
      const { h, s } = hexToHsl(p.color);
      return { ...p, color: hslToHex(h, s, L + offsets[i % 3]) };
    }),
  };
}

function nearestTint(intensity: number) {
  return TINTS.reduce((best, t) => (Math.abs(t.value - intensity) < Math.abs(best.value - intensity) ? t : best), TINTS[1]);
}

/** Which family (or own hue) a saved theme was made from, if any. */
function identify(theme: SpaceTheme): { kind: 'family'; id: string } | { kind: 'own'; hue: number } | null {
  const first = theme.points[0]?.color;
  if (!first) return null;
  const { h, s, l } = hexToHsl(first);
  if (Math.abs(l - 22) > 3 && Math.abs(l - 86) > 3) return null;
  for (const f of FAMILIES) {
    if (Math.abs(((h - f.hue + 540) % 360) - 180) < 3 && Math.abs(s - f.sat) < 4) return { kind: 'family', id: f.id };
  }
  const hue = OWN_HUES.find((x) => Math.abs(((h - x + 540) % 360) - 180) < 3);
  return hue !== undefined && Math.abs(s - 46) < 5 ? { kind: 'own', hue } : null;
}

/** A small browser drawn with a palette's colours. */
const Preview: React.FC<{ p: ThemePalette; sidebar: boolean }> = ({ p, sidebar }) => (
  <div
    className="studio-preview"
    style={
      {
        '--pv-bg': p.bgApp,
        '--pv-surface': p.surfaceSolid,
        '--pv-fg': p.fg,
        '--pv-muted': p.fgMuted,
        '--pv-line': p.border,
        '--pv-accent': p.accent,
        '--pv-c1': p.backdropC1,
        '--pv-c2': p.backdropC2,
      } as React.CSSProperties
    }
    aria-hidden="true"
  >
    <div className="pv-bar">
      <span className="pv-dot" />
      <span className="pv-ctl" />
      <span className="pv-ctl" />
      <span className="pv-url" />
    </div>
    <div className="pv-body">
      {sidebar && (
        <div className="pv-side">
          <span className="pv-title" />
          <span className="pv-row on" />
          <span className="pv-row" />
          <span className="pv-row" />
          <span className="pv-row short" />
        </div>
      )}
      <div className="pv-page">
        <span className="pv-widget" />
        <span className="pv-widget" />
        <span className="pv-widget wide" />
        <span className="pv-clock">10:24</span>
      </div>
    </div>
  </div>
);

export interface ThemeStudioProps {
  isOpen: boolean;
  onClose: () => void;
  activeSpace?: Space;
  spaceTheme: SpaceTheme;
  /** Applies (and, unless told otherwise, saves) the active space's theme. */
  onChangeSpaceTheme: (theme: SpaceTheme, options?: { immediateSave?: boolean; animateTransition?: boolean }) => void;
  /** Gives every space this theme. */
  onApplyToAllSpaces: (theme: SpaceTheme) => void;
  sidebarLayout: boolean;
}

/**
 * Theme Studio: each space's colours, from a curated set. Hovering a palette
 * previews it in the miniature without touching the browser; choosing it
 * applies it at once. Every choice keeps text readable: palettes are built
 * per light/dark mode, and the text colour is picked for contrast.
 */
export const ThemeStudio: React.FC<ThemeStudioProps> = ({ isOpen, onClose, activeSpace, spaceTheme, onChangeSpaceTheme, onApplyToAllSpaces, sidebarLayout }) => {
  const [osDark, setOsDark] = useState(getOsPrefersDark);
  useEffect(() => {
    const mq = window.matchMedia?.('(prefers-color-scheme: dark)');
    if (!mq) return;
    const on = (e: MediaQueryListEvent) => setOsDark(e.matches);
    mq.addEventListener('change', on);
    return () => mq.removeEventListener('change', on);
  }, []);

  const mode = spaceTheme.mode;
  const dark = mode === 'dark' || (mode === 'auto' && osDark);
  const tint = nearestTint(spaceTheme.intensity);
  const current = useMemo(() => identify(spaceTheme), [spaceTheme]);
  const [hover, setHover] = useState<SpaceTheme | null>(null);
  const [appliedAll, setAppliedAll] = useState(false);
  const doneRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!isOpen) return;
    setAppliedAll(false);
    const t = window.setTimeout(() => doneRef.current?.focus({ preventScroll: true }), 60);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => {
      window.clearTimeout(t);
      window.removeEventListener('keydown', onKey);
    };
  }, [isOpen, onClose]);

  const themeFor = (sel: { kind: 'family'; id: string } | { kind: 'own'; hue: number }, m: SpaceThemeMode = mode, intensity: number = tint.value) => {
    const isDark = m === 'dark' || (m === 'auto' && osDark);
    if (sel.kind === 'family') {
      const f = FAMILIES.find((x) => x.id === sel.id)!;
      return familyTheme(f.hue, f.sat, m, intensity, isDark);
    }
    return familyTheme(sel.hue, 46, m, intensity, isDark);
  };

  const apply = (theme: SpaceTheme) => {
    setAppliedAll(false);
    onChangeSpaceTheme(theme, { animateTransition: true });
  };

  // Mode and tint keep the palette: rebuilt for the new mode when it is one
  // of Studio's, otherwise only the mode/intensity change.
  const setMode = (m: SpaceThemeMode) => {
    if (current) return apply(themeFor(current, m));
    const was = spaceTheme.mode === 'dark' || (spaceTheme.mode === 'auto' && osDark);
    const now = m === 'dark' || (m === 'auto' && osDark);
    apply(was === now ? { ...spaceTheme, mode: m } : retoneTheme(spaceTheme, m, now));
  };
  const setTint = (value: number) => apply(current ? themeFor(current, mode, value) : { ...spaceTheme, intensity: value });

  const shown = hover ?? spaceTheme;
  const palette = useMemo(() => computeThemePalette(shown, osDark), [shown, osDark]);

  if (!isOpen) return null;
  const swatch = (t: SpaceTheme) => {
    const p = computeThemePalette(t, osDark);
    return [p.backdropC1, p.backdropC2, p.surfaceSolid];
  };

  return (
    <div
      className="modal-overlay studio-overlay"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="sheet studio" role="dialog" aria-modal="true" aria-labelledby="studio-title">
        <header className="wed-head studio-head">
          <div className="wed-titles">
            <h2 className="wed-title" id="studio-title">
              Theme Studio
            </h2>
            <p className="wed-sub">Colours for {activeSpace ? <strong>{activeSpace.name}</strong> : 'this space'}. Each space keeps its own.</p>
          </div>
          <button type="button" className="btn btn-quiet btn-icon" aria-label="Close" onClick={onClose}>
            <X size={16} />
          </button>
        </header>

        <div className="studio-body">
          <div className="studio-left">
            <Preview p={palette} sidebar={sidebarLayout} />
            <p className="studio-caption">{hover ? 'Preview' : 'Now'} · text contrast {palette.minContrast >= 7 ? 'AAA' : palette.minContrast >= 4.5 ? 'AA' : 'low'}</p>
          </div>

          <div className="studio-right">
            <div className="studio-field">
              <span className="studio-label">Appearance</span>
              <div className="seg" role="radiogroup" aria-label="Appearance">
                {(
                  [
                    ['light', 'Light', Sun],
                    ['dark', 'Dark', Moon],
                    ['auto', 'Match Windows', Monitor],
                  ] as const
                ).map(([v, label, Icon]) => (
                  <button key={v} type="button" role="radio" aria-checked={mode === v} className={mode === v ? 'on' : ''} onClick={() => setMode(v)}>
                    <Icon size={13} aria-hidden="true" style={{ marginRight: 6, verticalAlign: -2 }} />
                    {label}
                  </button>
                ))}
              </div>
            </div>

            <div className="studio-field">
              <span className="studio-label">Palette</span>
              <div className="studio-palettes" role="radiogroup" aria-label="Palette" onMouseLeave={() => setHover(null)}>
                {FAMILIES.map((f) => {
                  const t = themeFor({ kind: 'family', id: f.id });
                  const on = current?.kind === 'family' && current.id === f.id;
                  const [a, b, c] = swatch(t);
                  return (
                    <button
                      key={f.id}
                      type="button"
                      role="radio"
                      aria-checked={on}
                      className={`studio-palette${on ? ' on' : ''}`}
                      onMouseEnter={() => setHover(t)}
                      onFocus={() => setHover(t)}
                      onBlur={() => setHover(null)}
                      onClick={() => apply(t)}
                    >
                      <span className="studio-chip" style={{ background: `linear-gradient(135deg, ${a}, ${b} 60%, ${c})` }}>
                        {on && <Check size={13} strokeWidth={2.6} />}
                      </span>
                      <span className="studio-name">{f.name}</span>
                    </button>
                  );
                })}
              </div>
            </div>

            <div className="studio-field">
              <span className="studio-label">Your own colour</span>
              <div className="studio-hues" role="radiogroup" aria-label="Your own colour" onMouseLeave={() => setHover(null)}>
                {OWN_HUES.map((h) => {
                  const t = themeFor({ kind: 'own', hue: h });
                  const on = current?.kind === 'own' && current.hue === h;
                  return (
                    <button
                      key={h}
                      type="button"
                      role="radio"
                      aria-checked={on}
                      aria-label={`Hue ${h}°`}
                      className={`studio-hue${on ? ' on' : ''}`}
                      style={{ background: hslToHex(h, 58, dark ? 46 : 62) }}
                      onMouseEnter={() => setHover(t)}
                      onFocus={() => setHover(t)}
                      onBlur={() => setHover(null)}
                      onClick={() => apply(t)}
                    />
                  );
                })}
              </div>
            </div>

            <div className="studio-field">
              <span className="studio-label">Tint</span>
              <div className="seg" role="radiogroup" aria-label="Tint">
                {TINTS.map((t) => (
                  <button key={t.id} type="button" role="radio" aria-checked={tint.id === t.id} className={tint.id === t.id ? 'on' : ''} onClick={() => setTint(t.value)}>
                    {t.label}
                  </button>
                ))}
              </div>
            </div>
          </div>
        </div>

        <footer className="wed-foot">
          <button
            type="button"
            className="btn btn-quiet"
            onClick={() => {
              const fallback = getDefaultSpaceTheme(activeSpace?.id ?? '');
              apply(fallback);
            }}
          >
            <RotateCcw size={13} aria-hidden="true" />
            Reset
          </button>
          <button
            type="button"
            className="btn btn-quiet"
            onClick={() => {
              onApplyToAllSpaces(spaceTheme);
              setAppliedAll(true);
            }}
          >
            {appliedAll ? <Check size={13} aria-hidden="true" /> : null}
            {appliedAll ? 'Every space uses it' : 'Use for every space'}
          </button>
          <span className="wed-spacer" />
          <button ref={doneRef} type="button" className="btn btn-primary" onClick={onClose}>
            Done
          </button>
        </footer>
      </div>
    </div>
  );
};

export default ThemeStudio;
