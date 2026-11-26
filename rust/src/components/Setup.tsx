import React, { useEffect, useMemo, useRef, useState } from 'react';
import { ArrowLeft, Check, Monitor, Moon, ShieldCheck, Sun } from 'lucide-react';
import { SericaLogo } from './SericaLogo';
import { ImportPanel } from './settings/ImportPanel';
import { FAMILIES, familyTheme, retoneTheme } from './ThemeStudio';
import { SPECIAL_ART } from './Homescreen';
import { DEFAULT_HOME_BACKGROUND, homeBackgroundCss, type BrowserSettings, type HomeBackground, type SpaceTheme } from '../shared/settings';
import type { SpaceThemeMode } from '../shared/spaceTheme';
import { computeThemePalette } from '../shared/spaceTheme';

type Step = 'welcome' | 'bring' | 'hold' | 'look' | 'links';
const STEPS: Step[] = ['welcome', 'bring', 'hold', 'look', 'links'];

export interface SetupProps {
  settings: BrowserSettings;
  onUpdateSettings: (patch: Partial<BrowserSettings>) => void;
  /** Someone updating from an earlier Serica: everything they have stays. */
  upgrading: boolean;
  /** Every space's theme as it is now (resolved), by space id. */
  spaceThemes: Record<string, SpaceTheme>;
  activeSpaceId: string;
  osDark: boolean;
  /** Replaces the saved space themes with `themes` and shows `shown` at once. */
  onApplyThemes: (themes: Record<string, unknown>, shown: SpaceTheme) => void;
  onFinish: () => void;
}

/**
 * Serica's setup, after Search's welcome: a handful of calm pages, each
 * optional. Shown on first launch and once after updating to this version.
 * It never removes or resets anything: tabs, sign-ins, cookies, passwords
 * and settings all stay; only what is picked here changes.
 */
export const Setup: React.FC<SetupProps> = ({ settings, onUpdateSettings, upgrading, spaceThemes, activeSpaceId, osDark, onApplyThemes, onFinish }) => {
  const [i, setI] = useState(0);
  const [dir, setDir] = useState(1);
  const step = STEPS[i];
  const go = (d: number) => {
    setDir(d);
    setI((x) => Math.max(0, Math.min(STEPS.length - 1, x + d)));
  };

  // Look. Everything is worked out from the themes as they were when setup
  // opened, so "Keep mine" always gives back each space's own theme exactly,
  // whatever was tried in between.
  const original = useRef<{ raw: Record<string, unknown>; resolved: Record<string, SpaceTheme>; active: SpaceTheme } | null>(null);
  if (!original.current) {
    original.current = {
      raw: { ...((settings.spaceThemes as Record<string, unknown> | undefined) ?? {}) },
      resolved: { ...spaceThemes },
      active: spaceThemes[activeSpaceId] ?? Object.values(spaceThemes)[0],
    };
  }
  const orig = original.current;
  const [palette, setPalette] = useState<string>(upgrading ? 'keep' : 'graphite');
  const [mode, setMode] = useState<SpaceThemeMode>(orig.active.mode);
  const [modeTouched, setModeTouched] = useState(false);
  const isDark = (m: SpaceThemeMode) => m === 'dark' || (m === 'auto' && osDark);
  const applyLook = (p: string, m: SpaceThemeMode, touched: boolean) => {
    setPalette(p);
    setMode(m);
    setModeTouched(touched);
    const dark = isDark(m);
    if (p === 'keep') {
      if (!touched) {
        onApplyThemes(orig.raw, orig.active);
        return;
      }
      // Each space keeps its colours, moved to the other side only when it changes.
      const adjust = (t: SpaceTheme) => (isDark(t.mode) === dark ? { ...t, mode: m } : retoneTheme(t, m, dark));
      const all = Object.fromEntries(Object.entries(orig.resolved).map(([id, t]) => [id, adjust(t)]));
      onApplyThemes({ ...orig.raw, ...all }, adjust(orig.active));
      return;
    }
    const f = FAMILIES.find((x) => x.id === p)!;
    const theme = familyTheme(f.hue, f.sat, m, 0.58, dark);
    const all = Object.fromEntries(Object.keys(orig.resolved).map((id) => [id, { ...theme, points: theme.points.map((pt) => ({ ...pt })) }]));
    onApplyThemes({ ...orig.raw, ...all }, theme);
  };
  useEffect(() => {
    // A new install starts from Graphite; people updating keep theirs.
    if (!upgrading) applyLook('graphite', orig.active.mode, false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const bg = settings.homeBackground ?? DEFAULT_HOME_BACKGROUND;
  const setBg = (patch: Partial<HomeBackground>) => onUpdateSettings({ homeBackground: { ...bg, ...patch } });

  const [isDefault, setIsDefault] = useState<boolean | null>(null);
  const [asked, setAsked] = useState(false);
  useEffect(() => {
    if (step !== 'links') return;
    let alive = true;
    const check = () => window.serica?.defaultBrowserStatus?.().then((s: any) => alive && setIsDefault(!!s?.isDefault));
    void check();
    const t = asked ? window.setInterval(check, 1500) : 0;
    return () => {
      alive = false;
      if (t) window.clearInterval(t);
    };
  }, [step, asked]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t.closest('input, textarea, select')) return;
      if (e.key === 'ArrowRight' || (e.key === 'Enter' && !t.closest('button'))) {
        e.preventDefault();
        if (i < STEPS.length - 1) go(1);
        else onFinish();
      } else if (e.key === 'ArrowLeft' && i > 0) {
        e.preventDefault();
        go(-1);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  const swatch = (id: string) => {
    const f = FAMILIES.find((x) => x.id === id)!;
    const p = computeThemePalette(familyTheme(f.hue, f.sat, mode, 0.58, mode === 'dark' || (mode === 'auto' && osDark)), osDark);
    return `linear-gradient(135deg, ${p.backdropC1}, ${p.backdropC2} 60%, ${p.surfaceSolid})`;
  };
  const keepSwatch = useMemo(() => {
    const t = modeTouched && isDark(mode) !== isDark(orig.active.mode) ? retoneTheme(orig.active, mode, isDark(mode)) : orig.active;
    const p = computeThemePalette(t, osDark);
    return `linear-gradient(135deg, ${p.backdropC1}, ${p.backdropC2} 60%, ${p.surfaceSolid})`;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, modeTouched, osDark]);

  const page = (() => {
    switch (step) {
      case 'welcome':
        return (
          <div className="setup-page centered">
            <SericaLogo size={64} />
            <h1 className="setup-title">{upgrading ? 'Serica, renewed.' : 'Serica'}</h1>
            <p className="setup-lede">
              {upgrading
                ? 'A quieter new tab with widgets you can place anywhere, Focus Mode, Theme Studio, and settings rebuilt from the ground up. Your tabs, spaces, sign-ins and passwords are all still here.'
                : 'A calm, fast browser with nothing in the way: spaces for each part of your life, a sidebar for your tabs, and the page taking the rest.'}
            </p>
            {upgrading && (
              <p className="setup-reassure">
                <ShieldCheck size={13} aria-hidden="true" /> Nothing you have changes unless you change it here.
              </p>
            )}
          </div>
        );
      case 'bring':
        return (
          <div className="setup-page">
            <h2 className="setup-heading">Bring things over.</h2>
            <p className="setup-sub">Bookmarks, history and passwords from another browser on this PC. Nothing in the other browser changes, and nothing leaves this PC.</p>
            <ImportPanel compact />
          </div>
        );
      case 'hold':
        return (
          <div className="setup-page">
            <h2 className="setup-heading">Two ways to hold your tabs.</h2>
            <p className="setup-sub">Down the side in a sidebar that slides over the page, or across the top. Change your mind any time in Settings › Appearance.</p>
            <div className="setup-ways" role="radiogroup" aria-label="Tabs">
              {(
                [
                  ['sidebar', 'Sidebar'],
                  ['top', 'Across the top'],
                ] as const
              ).map(([id, label]) => {
                const on = (settings.tabLayout === 'top' ? 'top' : 'sidebar') === id;
                return (
                  <button key={id} type="button" role="radio" aria-checked={on} className={`setup-way${on ? ' on' : ''}`} onClick={() => onUpdateSettings({ tabLayout: id })}>
                    <span className={`setup-way-art ${id}`} aria-hidden="true">
                      <span className="w-bar">
                        <i />
                        <i />
                        <i />
                      </span>
                      {id === 'top' && (
                        <span className="w-strip">
                          <i className="on" />
                          <i />
                          <i />
                        </span>
                      )}
                      <span className="w-body">
                        {id === 'sidebar' && (
                          <span className="w-side">
                            <i className="on" />
                            <i />
                            <i />
                            <i />
                          </span>
                        )}
                        <span className="w-page" />
                      </span>
                    </span>
                    <span className="setup-way-label">
                      {on && <Check size={13} aria-hidden="true" />}
                      {label}
                    </span>
                  </button>
                );
              })}
            </div>
          </div>
        );
      case 'look':
        return (
          <div className="setup-page">
            <h2 className="setup-heading">Make it yours.</h2>
            <p className="setup-sub">Every space can have its own colours later, in Theme Studio.</p>
            <div className="seg setup-mode" role="radiogroup" aria-label="Appearance">
              {(
                [
                  ['light', 'Light', Sun],
                  ['dark', 'Dark', Moon],
                  ['auto', 'Match Windows', Monitor],
                ] as const
              ).map(([m, label, Icon]) => (
                <button key={m} type="button" role="radio" aria-checked={mode === m} className={mode === m ? 'on' : ''} onClick={() => applyLook(palette, m, true)}>
                  <Icon size={13} aria-hidden="true" style={{ marginRight: 6, verticalAlign: -2 }} />
                  {label}
                </button>
              ))}
            </div>
            <div className="setup-palettes" role="radiogroup" aria-label="Colours">
              {upgrading && (
                <button type="button" role="radio" aria-checked={palette === 'keep'} className={`setup-palette${palette === 'keep' ? ' on' : ''}`} onClick={() => applyLook('keep', mode, modeTouched)}>
                  <span style={{ background: keepSwatch }} />
                  Keep mine
                </button>
              )}
              {FAMILIES.slice(0, upgrading ? 7 : 8).map((f) => (
                <button key={f.id} type="button" role="radio" aria-checked={palette === f.id} className={`setup-palette${palette === f.id ? ' on' : ''}`} onClick={() => applyLook(f.id, mode, modeTouched)}>
                  <span style={{ background: swatch(f.id) }} />
                  {f.name}
                </button>
              ))}
            </div>
            <div className="setup-label">New tab</div>
            <div className="setup-bgs" role="radiogroup" aria-label="New tab background">
              {(
                [
                  ['theme', 'Space colours', keepSwatch],
                  ['special', 'Serica Special', `center / cover url(${SPECIAL_ART[0].thumb})`],
                  ['gradient', 'Gradient', homeBackgroundCss({ ...bg, mode: 'gradient' })!],
                  ['solid', 'Colour', bg.solid],
                ] as const
              ).map(([m, label, look]) => (
                <button key={m} type="button" role="radio" aria-checked={bg.mode === m} className={`setup-bg${bg.mode === m ? ' on' : ''}`} onClick={() => setBg({ mode: m })}>
                  <span style={{ background: look }} />
                  {label}
                </button>
              ))}
            </div>
            <p className="setup-hint">Your own picture, exact colours and gradients are in Settings › New Tab.</p>
          </div>
        );
      case 'links':
        return (
          <div className="setup-page">
            <h2 className="setup-heading">Links from other apps.</h2>
            <p className="setup-sub">A click in Mail, Teams or a PDF goes to whichever browser Windows uses by default. It can be this one.</p>
            <div className="setup-default">
              {isDefault ? (
                <span className="s2-ok">
                  <Check size={14} aria-hidden="true" /> Serica is your default browser
                </span>
              ) : (
                <>
                  <button
                    type="button"
                    className="btn btn-primary setup-big"
                    onClick={() => {
                      setAsked(true);
                      void window.serica?.makeDefaultBrowser?.().then((s: any) => setIsDefault(!!s?.isDefault));
                    }}
                  >
                    Make Serica the default
                  </button>
                  {asked && <span className="setup-hint">Windows asks in Settings: choose Serica there.</span>}
                </>
              )}
            </div>
            <div className="setup-label">A few things worth knowing</div>
            <div className="setup-keys">
              {(
                [
                  ['Ctrl T', 'A new tab: type a place, or words to search.'],
                  ['Ctrl K', 'Every open tab, and commands (type >).'],
                  ['Ctrl S', 'The sidebar. Your back and forward buttons move into it.'],
                  ['Ctrl 1–9', 'Spaces: separate tabs and sign-ins.'],
                  ['Ctrl ,', 'Settings. Ctrl F there finds any setting.'],
                ] as const
              ).map(([k, d]) => (
                <div key={k} className="setup-key">
                  <kbd>{k}</kbd>
                  <span>{d}</span>
                </div>
              ))}
            </div>
          </div>
        );
    }
  })();

  return (
    <div className="modal-overlay setup-overlay">
      <div className="setup-card" role="dialog" aria-modal="true" aria-label="Set up Serica">
        <div className={`setup-stage dir-${dir > 0 ? 'fwd' : 'back'}`} key={step}>
          {page}
        </div>
        <footer className="setup-foot">
          <div className="setup-dots" aria-label={`Step ${i + 1} of ${STEPS.length}`}>
            {STEPS.map((s, k) => (
              <span key={s} className={k === i ? 'on' : k < i ? 'done' : ''} />
            ))}
          </div>
          <span className="setup-spacer" />
          {i > 0 && (
            <button type="button" className="btn btn-quiet" onClick={() => go(-1)}>
              <ArrowLeft size={13} aria-hidden="true" /> Back
            </button>
          )}
          {i < STEPS.length - 1 ? (
            <button type="button" className="btn btn-primary setup-next" onClick={() => go(1)}>
              {step === 'welcome' ? 'Get started' : step === 'bring' ? 'Continue' : 'Continue'}
            </button>
          ) : (
            <button type="button" className="btn btn-primary setup-next" onClick={onFinish}>
              Start browsing
            </button>
          )}
        </footer>
      </div>
    </div>
  );
};

export default Setup;
