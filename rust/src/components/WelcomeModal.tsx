import { SericaLogo } from './SericaLogo';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { ArrowLeft, ArrowRight, Check, Monitor, Moon, PanelLeft, PanelTop, Sun } from 'lucide-react';
import { SPACE_THEME_PRESETS, type SpaceTheme, type SpaceThemeMode } from '../shared/spaceTheme';
import { TEXT_SCALE_OPTIONS } from '../shared/settings';
import { applySpaceThemeToRoot } from '../utils/applySpaceTheme';
import '../styles/onboarding.css';

export interface UserProfile {
  name: string;
  /** Always null: Serica keeps no account. Kept for profiles saved by older versions. */
  email: string | null;
  isGuest: boolean;
  theme: 'obsidian' | 'charcoal' | 'slate';
  mode: 'minimal' | 'architect';
}

export type WelcomeTabLayout = 'sidebar' | 'top';

export interface WelcomeSite {
  id: string;
  title: string;
  url: string;
}

export interface WelcomePrefs {
  tabLayout: WelcomeTabLayout;
  /** Theme for every space (preset colors with the chosen light/dark/auto mode). */
  spaceTheme: SpaceTheme;
  textScale: number;
  dockVisible: boolean;
  dockSites: WelcomeSite[];
  sidebarSites: WelcomeSite[];
}

interface WelcomeModalProps {
  isOpen: boolean;
  /** Live preview of the text size while choosing (zooms the interface). */
  onPreviewTextScale?: (scale: number) => void;
  onComplete: (profile: UserProfile, prefs: WelcomePrefs) => void;
}

export const WELCOME_SITES: readonly WelcomeSite[] = [
  { id: 'youtube', title: 'YouTube', url: 'https://www.youtube.com' },
  { id: 'github', title: 'GitHub', url: 'https://github.com' },
  { id: 'x', title: 'X', url: 'https://x.com' },
  { id: 'reddit', title: 'Reddit', url: 'https://www.reddit.com' },
  { id: 'notion', title: 'Notion', url: 'https://www.notion.so' },
  { id: 'gmail', title: 'Gmail', url: 'https://mail.google.com' },
  { id: 'chatgpt', title: 'ChatGPT', url: 'https://chatgpt.com' },
  { id: 'claude', title: 'Claude', url: 'https://claude.ai' },
  { id: 'spotify', title: 'Spotify', url: 'https://open.spotify.com' },
  { id: 'whatsapp', title: 'WhatsApp', url: 'https://web.whatsapp.com' },
  { id: 'netflix', title: 'Netflix', url: 'https://www.netflix.com' },
  { id: 'figma', title: 'Figma', url: 'https://www.figma.com' },
  { id: 'linear', title: 'Linear', url: 'https://linear.app' },
  { id: 'discord', title: 'Discord', url: 'https://discord.com/app' },
  { id: 'drive', title: 'Drive', url: 'https://drive.google.com' },
  { id: 'calendar', title: 'Calendar', url: 'https://calendar.google.com' },
];

const DEFAULT_DOCK = ['youtube', 'github', 'x', 'reddit', 'notion'];

const STEPS = ['name', 'tabs', 'theme', 'size', 'dock', 'pins'] as const;

/** Presets in onboarding order: the neutral defaults first. */
const ONBOARDING_FIRST = ['obsidian-mono', 'cobalt-dusk'];
const ONBOARDING_PRESETS = [
  ...ONBOARDING_FIRST.map((id) => SPACE_THEME_PRESETS.find((p) => p.id === id)!).filter(Boolean),
  ...SPACE_THEME_PRESETS.filter((p) => !ONBOARDING_FIRST.includes(p.id)),
];
type Step = (typeof STEPS)[number];

/** The Serica mark on the app-icon tile. */
const SericaMark: React.FC = () => (
  <div className="onb-mark" aria-hidden="true">
    <SericaLogo size="100%" />
  </div>
);

function hostOf(url: string): string {
  try {
    return new URL(url).hostname;
  } catch {
    return '';
  }
}

const SiteIcon: React.FC<{ site: WelcomeSite }> = ({ site }) => {
  const [failed, setFailed] = useState(false);
  return failed ? (
    <span className="onb-site-letter">{site.title.charAt(0)}</span>
  ) : (
    <img
      src={`https://www.google.com/s2/favicons?domain=${hostOf(site.url)}&sz=64`}
      alt=""
      width={22}
      height={22}
      draggable={false}
      onError={() => setFailed(true)}
    />
  );
};

const SitePicker: React.FC<{ selected: Set<string>; onToggle: (id: string) => void; label: string }> = ({
  selected,
  onToggle,
  label,
}) => (
  <div className="onb-sites" role="group" aria-label={label}>
    {WELCOME_SITES.map((site) => {
      const on = selected.has(site.id);
      return (
        <button
          key={site.id}
          type="button"
          className={`onb-site${on ? ' selected' : ''}`}
          aria-pressed={on}
          onClick={() => onToggle(site.id)}
        >
          <span className="onb-site-icon">
            <SiteIcon site={site} />
          </span>
          <span className="onb-site-title">{site.title}</span>
          <span className="onb-site-check" aria-hidden="true">
            <Check size={10} strokeWidth={3} />
          </span>
        </button>
      );
    })}
  </div>
);

const LayoutDiagram: React.FC<{ layout: WelcomeTabLayout }> = ({ layout }) => (
  <div className={`onb-diagram ${layout}`} aria-hidden="true">
    <div className="onb-diagram-bar" />
    {layout === 'top' ? (
      <div className="onb-diagram-tabs">
        <span className="active" />
        <span />
        <span />
      </div>
    ) : (
      <div className="onb-diagram-side">
        <span className="active" />
        <span />
        <span />
        <span />
      </div>
    )}
    <div className="onb-diagram-page" />
  </div>
);

function presetGradient(theme: SpaceTheme): string {
  const [a, b, c] = theme.points.map((p) => p.color);
  return `radial-gradient(120% 120% at 20% 15%, ${b ?? a} 0%, transparent 60%), linear-gradient(135deg, ${a} 0%, ${c ?? a} 100%)`;
}

function osPrefersDark(): boolean {
  return typeof window.matchMedia === 'function' ? window.matchMedia('(prefers-color-scheme: dark)').matches : true;
}

export const WelcomeModal: React.FC<WelcomeModalProps> = ({ isOpen, onPreviewTextScale, onComplete }) => {
  const [stepIndex, setStepIndex] = useState(0);
  const [direction, setDirection] = useState<'fwd' | 'back'>('fwd');
  const [name, setName] = useState('');
  const [tabLayout, setTabLayout] = useState<WelcomeTabLayout>('sidebar');
  const [presetId, setPresetId] = useState('obsidian-mono');
  const [mode, setMode] = useState<SpaceThemeMode>('dark');
  const [textScale, setTextScale] = useState(1);
  const [dockVisible, setDockVisible] = useState(true);
  const [dock, setDock] = useState<Set<string>>(() => new Set(DEFAULT_DOCK));
  const [pins, setPins] = useState<Set<string>>(() => new Set());
  const [leaving, setLeaving] = useState(false);
  const nameRef = useRef<HTMLInputElement>(null);

  const step: Step = STEPS[stepIndex];
  const trimmedName = name.trim();
  const isLast = stepIndex === STEPS.length - 1;

  const spaceTheme = useMemo<SpaceTheme>(() => {
    const preset = SPACE_THEME_PRESETS.find((p) => p.id === presetId) ?? SPACE_THEME_PRESETS[0];
    return { ...preset.theme, points: preset.theme.points.map((p) => ({ ...p })), mode };
  }, [presetId, mode]);

  // Live preview: the browser behind the dialog takes the chosen theme.
  useEffect(() => {
    if (isOpen) applySpaceThemeToRoot(spaceTheme, osPrefersDark());
  }, [isOpen, spaceTheme]);

  useEffect(() => {
    if (isOpen && step === 'name') {
      const t = window.setTimeout(() => nameRef.current?.focus(), 380);
      return () => window.clearTimeout(t);
    }
  }, [isOpen, step]);

  if (!isOpen) return null;

  const canContinue = step !== 'name' || trimmedName.length > 0;

  const go = (delta: 1 | -1) => {
    setDirection(delta > 0 ? 'fwd' : 'back');
    setStepIndex((i) => Math.max(0, Math.min(STEPS.length - 1, i + delta)));
  };

  const pick = (ids: Set<string>) => WELCOME_SITES.filter((s) => ids.has(s.id));

  const finish = () => {
    if (leaving) return;
    const profile: UserProfile = {
      name: trimmedName,
      email: null,
      isGuest: true,
      theme: 'obsidian',
      mode: 'minimal',
    };
    const prefs: WelcomePrefs = {
      tabLayout,
      spaceTheme,
      textScale,
      dockVisible,
      dockSites: pick(dock),
      sidebarSites: pick(pins),
    };
    // Let the dialog fade out before the browser takes over.
    setLeaving(true);
    window.setTimeout(() => onComplete(profile, prefs), 240);
  };

  const next = () => {
    if (!canContinue) {
      nameRef.current?.focus();
      return;
    }
    if (isLast) finish();
    else go(1);
  };

  const toggle = (set: React.Dispatch<React.SetStateAction<Set<string>>>) => (id: string) =>
    set((prev) => {
      const out = new Set(prev);
      if (out.has(id)) out.delete(id);
      else out.add(id);
      return out;
    });

  const chooseScale = (value: number) => {
    setTextScale(value);
    onPreviewTextScale?.(value);
  };

  let body: React.ReactNode;
  switch (step) {
    case 'name':
      body = (
        <>
          <SericaMark />
          <h1 className="onb-title">
            Welcome to <em>Serica</em>
          </h1>
          <p className="onb-sub">A calm, fast browser that stays out of your way. Let's set it up in a minute.</p>
          <label className="onb-label" htmlFor="onb-name">
            What should we call you?
          </label>
          <input
            id="onb-name"
            ref={nameRef}
            className="onb-input"
            type="text"
            autoComplete="given-name"
            spellCheck={false}
            maxLength={40}
            placeholder="Your name"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
          <p className="onb-hint">Used for your greeting on the new-tab page. It never leaves this computer.</p>
        </>
      );
      break;
    case 'tabs':
      body = (
        <>
          <h1 className="onb-title">
            Nice to meet you, <em>{trimmedName}</em>.
          </h1>
          <p className="onb-sub">How do you like your tabs?</p>
          <div className="onb-cards two" role="radiogroup" aria-label="Tab layout">
            {([
              ['sidebar', 'Sidebar', 'Vertical tabs with pinned tabs, folders and hover previews', PanelLeft],
              ['top', 'Top tabs', 'A classic tab strip above the page', PanelTop],
            ] as const).map(([value, title, desc, Icon]) => (
              <button
                key={value}
                type="button"
                role="radio"
                aria-checked={tabLayout === value}
                className={`onb-card${tabLayout === value ? ' selected' : ''}`}
                onClick={() => setTabLayout(value)}
              >
                <LayoutDiagram layout={value} />
                <span className="onb-card-title">
                  <Icon size={14} aria-hidden="true" />
                  {title}
                </span>
                <span className="onb-card-desc">{desc}</span>
              </button>
            ))}
          </div>
          <p className="onb-hint">You can switch any time in Settings → Appearance.</p>
        </>
      );
      break;
    case 'theme':
      body = (
        <>
          <h1 className="onb-title">
            Pick a <em>look</em>.
          </h1>
          <p className="onb-sub">Every space gets this theme. You can give each space its own colors later in Theme Studio.</p>
          <div className="onb-segmented" role="radiogroup" aria-label="Appearance">
            {([
              ['dark', 'Dark', Moon],
              ['light', 'Light', Sun],
              ['auto', 'Match system', Monitor],
            ] as const).map(([value, label, Icon]) => (
              <button
                key={value}
                type="button"
                role="radio"
                aria-checked={mode === value}
                className={`onb-segment${mode === value ? ' active' : ''}`}
                onClick={() => setMode(value)}
              >
                <Icon size={13} aria-hidden="true" />
                {label}
              </button>
            ))}
          </div>
          <div className="onb-themes" role="radiogroup" aria-label="Theme">
            {ONBOARDING_PRESETS.map((p) => (
              <button
                key={p.id}
                type="button"
                role="radio"
                aria-checked={presetId === p.id}
                title={p.description}
                className={`onb-theme${presetId === p.id ? ' selected' : ''}`}
                onClick={() => setPresetId(p.id)}
              >
                <span className="onb-theme-swatch" style={{ background: presetGradient(p.theme) }} />
                <span className="onb-theme-name">{p.name}</span>
              </button>
            ))}
          </div>
        </>
      );
      break;
    case 'size':
      body = (
        <>
          <h1 className="onb-title">
            Comfortable <em>text size</em>?
          </h1>
          <p className="onb-sub">Scales the browser and how big web pages open. Try one: the whole window follows.</p>
          <div className="onb-cards four" role="radiogroup" aria-label="Text size">
            {TEXT_SCALE_OPTIONS.map(({ value, label }) => (
              <button
                key={value}
                type="button"
                role="radio"
                aria-checked={Math.abs(textScale - value) < 0.001}
                className={`onb-card onb-size${Math.abs(textScale - value) < 0.001 ? ' selected' : ''}`}
                onClick={() => chooseScale(value)}
              >
                <span className="onb-size-sample" style={{ fontSize: `${Math.round(22 * value)}px` }}>
                  Aa
                </span>
                <span className="onb-card-title">{label}</span>
                <span className="onb-card-desc">{Math.round(value * 100)}%</span>
              </button>
            ))}
          </div>
          <p className="onb-hint">Change it later in Settings → Appearance → Text size.</p>
        </>
      );
      break;
    case 'dock':
      body = (
        <>
          <h1 className="onb-title">
            Your <em>dock</em>.
          </h1>
          <p className="onb-sub">
            One-click sites on the right edge of the window. On web pages it tucks away until you move the cursor to the
            right edge.
          </p>
          <div className="onb-toggle-row">
            <span>Show the website dock</span>
            <button
              type="button"
              role="switch"
              aria-checked={dockVisible}
              aria-label="Show the website dock"
              className={`onb-switch${dockVisible ? ' on' : ''}`}
              onClick={() => setDockVisible((v) => !v)}
            >
              <span className="onb-switch-thumb" />
            </button>
          </div>
          <div className={`onb-collapsible${dockVisible ? '' : ' collapsed'}`} aria-hidden={!dockVisible}>
            <SitePicker selected={dock} onToggle={toggle(setDock)} label="Dock sites" />
          </div>
        </>
      );
      break;
    case 'pins':
      body = (
        <>
          <h1 className="onb-title">
            Pin your <em>apps</em>.
          </h1>
          <p className="onb-sub">
            Pinned tabs stay in the {tabLayout === 'top' ? 'tab strip' : 'sidebar'} and load only when you open them.
            Pick as many as you like, or none.
          </p>
          <SitePicker selected={pins} onToggle={toggle(setPins)} label="Pinned tabs" />
        </>
      );
      break;
  }

  return (
    <div className={`onb-backdrop${leaving ? ' leaving' : ''}`}>
      <div
        className="onb-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="Set up Serica"
        onKeyDown={(e) => {
          if (e.key === 'Enter' && !(e.target instanceof HTMLButtonElement)) {
            e.preventDefault();
            next();
          }
        }}
      >
        <div className="onb-glow" aria-hidden="true" />
        <div className="onb-progress" aria-hidden="true">
          {STEPS.map((s, i) => (
            <span key={s} className={i < stepIndex ? 'done' : i === stepIndex ? 'current' : ''} />
          ))}
        </div>
        <div className="onb-body">
          <div key={step} className={`onb-step ${direction}`}>
            {body}
          </div>
        </div>
        <div className="onb-footer">
          <button
            type="button"
            className={`onb-back${stepIndex === 0 ? ' hidden' : ''}`}
            onClick={() => go(-1)}
            tabIndex={stepIndex === 0 ? -1 : 0}
            aria-hidden={stepIndex === 0}
          >
            <ArrowLeft size={14} />
            Back
          </button>
          <span className="onb-count">
            {stepIndex + 1} of {STEPS.length}
          </span>
          <button type="button" className="onb-next" onClick={next} disabled={!canContinue}>
            {isLast ? 'Enter Serica' : 'Continue'}
            {isLast ? <Check size={15} /> : <ArrowRight size={15} />}
          </button>
        </div>
      </div>
    </div>
  );
};
