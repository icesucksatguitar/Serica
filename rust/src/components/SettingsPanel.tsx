import React, { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import {
  Puzzle,
  PanelLeft,
  PanelTop,
  X,
  Moon,
  Sun,
  Palette,
  Shield,
  Search,
  Zap,
  Layers,
  KeyRound,
  Info,
} from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import {
  ARCHIVE_INTERVAL_OPTIONS,
  DEFAULT_HOME_WIDGETS,
  DEFAULT_UI_SCALE,
  SEARCH_ENGINES,
  UI_SCALE_MAX,
  UI_SCALE_MIN,
  type UiScale,
  TAB_SLEEP_OPTIONS,
  type ArchiveIntervalHours,
  type SearchEngineId,
  type TabSleepMinutes,
} from '../shared/settings';
import type { BrowserSettings, TabState } from '../types';
import { ProfilesSettings } from './ProfilesSettings';
import { AboutSettings } from './AboutSettings';
import { CookieSettings } from './CookieSettings';
import { startGuide } from './GuideTour';
import { Switch } from './Switch';
import { TOOLS, normalizePinnedTools, type ToolActions } from '../shared/tools';
import { TEXT_SCALE_OPTIONS } from '../shared/settings';

export type SettingsTabId =
  | 'appearance'
  | 'privacy'
  | 'search'
  | 'performance'
  | 'profiles'
  | 'passwords'
  | 'about';

export interface SettingsPanelProps {
  isOpen: boolean;
  onClose: () => void;
  settings: BrowserSettings;
  onUpdateSettings: (patch: Partial<BrowserSettings>) => void;
  isDarkMode: boolean;
  /** Same side effects as the top-bar theme button; `true` picks Dark, `false` picks Light. */
  onSetDarkMode: (dark: boolean) => void;
  /** Opens the space theme editor. The panel closes itself first. */
  onOpenThemeStudio: () => void;
  userName: string;
  onUserNameChange: (name: string) => void;
  /** Open tabs, so profile/space deletion can say what it closes (R1.7, R13.8). Optional. */
  tabs?: TabState[];
  /** Optionally open with a specific tab selected. */
  initialTab?: SettingsTabId;
  /** Runs a browser tool (the panel closes itself first). */
  toolActions?: ToolActions;
}

export const tabSleepLabel = (minutes: TabSleepMinutes): string =>
  minutes < 60 ? `${minutes} min` : `${minutes / 60} hr`;

const tabSleepLongLabel = (minutes: TabSleepMinutes): string =>
  minutes < 60 ? `${minutes} minutes` : minutes === 60 ? '1 hour' : `${minutes / 60} hours`;

export const zenSleepHint = (minutes: TabSleepMinutes): string =>
  `Frees memory from background tabs unused for ${tabSleepLongLabel(minutes)}. They reload when you go back to them. Pinned tabs and tabs playing audio are kept`;

const ENGINE_IDS = Object.keys(SEARCH_ENGINES) as SearchEngineId[];

export const ARCHIVE_INTERVAL_LABELS: Readonly<Record<string, string>> = Object.freeze({
  '12': '12 hours',
  '24': '24 hours',
  '168': '7 days',
  '720': '30 days',
  never: 'Never',
});

export const ARCHIVE_INTERVAL_HINT = 'Closes inactive Today tabs and keeps them in the archive';
export const RIGHT_PINS_DOCK_HINT = 'One-click sites on the right edge. On web pages, move the cursor to the edge to reveal it';

const archiveIntervalLabel = (value: ArchiveIntervalHours): string =>
  ARCHIVE_INTERVAL_LABELS[String(value)];

/** Kept for existing imports; the control itself lives in Switch.tsx. */
export const SettingsSwitch = Switch;

interface SettingsTabMeta {
  id: SettingsTabId;
  label: string;
  icon: LucideIcon;
}

export const SETTINGS_TABS: readonly SettingsTabMeta[] = [
  { id: 'appearance', label: 'Appearance', icon: Palette },
  { id: 'privacy', label: 'Privacy', icon: Shield },
  { id: 'search', label: 'Search', icon: Search },
  { id: 'performance', label: 'Performance', icon: Zap },
  { id: 'profiles', label: 'Profiles & Spaces', icon: Layers },
  { id: 'passwords', label: 'Passwords', icon: KeyRound },
  { id: 'about', label: 'About', icon: Info },
];

// ---------------------------------------------------------------------------
// Building blocks: a group is a label over hairline-separated rows.

const Group: React.FC<{ title?: string; children: React.ReactNode }> = ({ title, children }) => (
  <section className="settings-group">
    {title && <h3 className="settings-group-label">{title}</h3>}
    <div className="settings-group-rows">{children}</div>
  </section>
);

const Row: React.FC<{ label: React.ReactNode; hint?: React.ReactNode; children?: React.ReactNode; htmlFor?: string }> = ({
  label,
  hint,
  children,
  htmlFor,
}) => (
  <div className="settings-row">
    <div className="settings-row-text">
      {htmlFor ? (
        <label className="settings-row-label" htmlFor={htmlFor}>
          {label}
        </label>
      ) : (
        <span className="settings-row-label">{label}</span>
      )}
      {hint && <span className="settings-hint">{hint}</span>}
    </div>
    {children !== undefined && <div className="settings-row-control">{children}</div>}
  </div>
);

function Segmented<T extends string | number | boolean>(props: {
  label: string;
  value: T;
  options: readonly { value: T; label: string; icon?: LucideIcon }[];
  onChange: (v: T) => void;
}) {
  return (
    <div className="settings-segmented" role="radiogroup" aria-label={props.label}>
      {props.options.map((o) => {
        const active = o.value === props.value;
        const Icon = o.icon;
        return (
          <button
            key={String(o.value)}
            type="button"
            role="radio"
            aria-checked={active}
            className={`settings-segment${active ? ' active' : ''}`}
            onClick={() => {
              if (!active) props.onChange(o.value);
            }}
          >
            {Icon && <Icon size={13} aria-hidden="true" />}
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

/** A size slider: 80% to 140% of the default, with a reset. */
const ScaleSlider: React.FC<{ label: string; value: number; onChange: (v: number) => void }> = ({ label, value, onChange }) => (
  <div className="scale-control">
    <input
      type="range"
      className="scale-slider"
      min={UI_SCALE_MIN}
      max={UI_SCALE_MAX}
      step={0.05}
      value={value}
      aria-label={label}
      aria-valuetext={`${Math.round(value * 100)}%`}
      onChange={(e) => onChange(Number(e.target.value))}
    />
    <span className="scale-value">{Math.round(value * 100)}%</span>
    <button
      type="button"
      className="btn btn-quiet scale-reset"
      disabled={value === 1}
      onClick={() => onChange(1)}
      aria-label={`Reset ${label}`}
      title="Reset"
    >
      Reset
    </button>
  </div>
);

/**
 * A miniature of the browser drawn with the live theme variables, so it shows
 * exactly what the current mode, space colors and layout choices look like.
 */
const AppearancePreview: React.FC<{ topTabs: boolean; pills: boolean; dock: boolean }> = ({ topTabs, pills, dock }) => (
  <div className={`appearance-preview${topTabs ? ' top-tabs' : ''}${pills ? ' pills' : ''}`} aria-hidden="true">
    <div className="ap-topbar">
      <span className="ap-dot" />
      <span className="ap-ctrl" />
      <span className="ap-ctrl" />
      <span className="ap-url" />
      <span className="ap-ctrl" />
    </div>
    {topTabs && (
      <div className="ap-tabstrip">
        <span className="ap-tab active" />
        <span className="ap-tab" />
        <span className="ap-tab" />
      </div>
    )}
    <div className="ap-body">
      {!topTabs && (
        <div className="ap-sidebar">
          <span className="ap-row active" />
          <span className="ap-row" />
          <span className="ap-row" />
          <span className="ap-row short" />
        </div>
      )}
      <div className="ap-page">
        <span className="ap-line wide" />
        <span className="ap-line" />
        <span className="ap-line" />
      </div>
      {dock && (
        <div className="ap-dock">
          <span />
          <span />
          <span />
        </div>
      )}
    </div>
  </div>
);

export const SettingsPanel: React.FC<SettingsPanelProps> = ({
  isOpen,
  onClose,
  settings,
  onUpdateSettings,
  isDarkMode,
  onSetDarkMode,
  onOpenThemeStudio,
  userName,
  onUserNameChange,
  tabs,
  initialTab = 'appearance',
  toolActions,
}) => {
  const idBase = useId();
  const titleId = `${idBase}-title`;
  const nameId = `${idBase}-name`;

  const [activeTab, setActiveTab] = useState<SettingsTabId>(initialTab);
  const [nameDraft, setNameDraft] = useState(userName);
  const scrollRef = useRef<HTMLDivElement>(null);

  // Fresh state each time the panel opens. A name change made elsewhere only
  // updates the field; it never switches the page under the user.
  useEffect(() => {
    if (isOpen) setActiveTab(initialTab);
  }, [isOpen, initialTab]);
  useEffect(() => setNameDraft(userName), [userName]);

  // Every page starts at its top: one scroll position shared by all pages
  // would leave a short page scrolled past its end.
  useLayoutEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
  }, [activeTab]);

  useEffect(() => {
    if (!isOpen) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.defaultPrevented) {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const commitName = () => {
    const trimmed = nameDraft.trim();
    if (!trimmed) {
      setNameDraft(userName);
      return;
    }
    setNameDraft(trimmed);
    if (trimmed !== userName.trim()) onUserNameChange(trimmed);
  };

  const openThemeStudio = () => {
    onClose();
    onOpenThemeStudio();
  };

  const handleTablistKeyDown = (e: React.KeyboardEvent) => {
    const i = SETTINGS_TABS.findIndex((t) => t.id === activeTab);
    if (i < 0) return;
    let next = -1;
    if (e.key === 'ArrowDown') next = (i + 1) % SETTINGS_TABS.length;
    else if (e.key === 'ArrowUp') next = (i - 1 + SETTINGS_TABS.length) % SETTINGS_TABS.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = SETTINGS_TABS.length - 1;
    if (next < 0) return;
    e.preventDefault();
    setActiveTab(SETTINGS_TABS[next].id);
    document.getElementById(`${idBase}-tab-${SETTINGS_TABS[next].id}`)?.focus();
  };

  const currentTab = SETTINGS_TABS.find((t) => t.id === activeTab) ?? SETTINGS_TABS[0];
  const panel = (id: SettingsTabId, body: React.ReactNode) => (
    <div
      role="tabpanel"
      id={`${idBase}-tabpanel-${id}`}
      aria-labelledby={`${idBase}-tab-${id}`}
      hidden={activeTab !== id}
      className="settings-tabpanel"
    >
      {body}
    </div>
  );
  const pinnedTools = normalizePinnedTools(settings.pinnedTools);
  const topTabs = (settings.tabLayout ?? 'sidebar') === 'top';
  const widgets = settings.homeWidgets ?? DEFAULT_HOME_WIDGETS;
  const uiScale: UiScale = settings.uiScale ?? DEFAULT_UI_SCALE;

  return (
    <div
      className="modal-overlay settings-overlay"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="settings-panel-card" role="dialog" aria-modal="true" aria-labelledby={titleId}>
        <aside className="settings-nav-sidebar" aria-label="Settings navigation">
          <span className="settings-nav-title">Settings</span>
          <nav className="settings-nav-list" role="tablist" aria-orientation="vertical" onKeyDown={handleTablistKeyDown}>
            {SETTINGS_TABS.map((tab) => {
              const Icon = tab.icon;
              const isSelected = activeTab === tab.id;
              return (
                <button
                  key={tab.id}
                  id={`${idBase}-tab-${tab.id}`}
                  type="button"
                  role="tab"
                  tabIndex={isSelected ? 0 : -1}
                  aria-selected={isSelected}
                  aria-controls={`${idBase}-tabpanel-${tab.id}`}
                  className={`settings-nav-tab${isSelected ? ' active' : ''}`}
                  onClick={() => setActiveTab(tab.id)}
                >
                  <Icon size={15} className="settings-tab-icon" aria-hidden="true" />
                  <span className="settings-tab-label">{tab.label}</span>
                </button>
              );
            })}
          </nav>
        </aside>

        <main className="settings-content-pane">
          <header className="settings-pane-header">
            <h2 className="settings-pane-title" id={titleId}>
              {currentTab.label}
            </h2>
            <button type="button" className="btn btn-quiet btn-icon" onClick={onClose} title="Close (Esc)" aria-label="Close settings">
              <X size={16} />
            </button>
          </header>

          <div className="settings-content-scroll" ref={scrollRef}>
            {panel(
              'appearance',
              <>
                <div className="appearance-hero">
                  <AppearancePreview topTabs={topTabs} pills={settings.topBarStyle === 'pill'} dock={settings.rightPinsDockVisible} />
                  <div className="appearance-hero-controls">
                    <Segmented
                      label="Theme mode"
                      value={isDarkMode}
                      options={[
                        { value: true, label: 'Dark', icon: Moon },
                        { value: false, label: 'Light', icon: Sun },
                      ]}
                      onChange={(dark) => onSetDarkMode(dark)}
                    />
                    <div className="appearance-colors">
                      <span className="appearance-swatches" aria-hidden="true">
                        <span style={{ background: 'var(--theme-c1)' }} />
                        <span style={{ background: 'var(--theme-c2)' }} />
                        <span style={{ background: 'var(--theme-c3)' }} />
                      </span>
                      <button type="button" className="btn" onClick={openThemeStudio}>
                        <Palette size={13} aria-hidden="true" />
                        Edit space colors
                      </button>
                    </div>
                    <p className="settings-hint">Mode and colors belong to the current space.</p>
                  </div>
                </div>

                <Group title="Layout">
                  <Row label="Tabs">
                    <Segmented
                      label="Tab layout"
                      value={topTabs ? 'top' : 'sidebar'}
                      options={[
                        { value: 'sidebar', label: 'Sidebar', icon: PanelLeft },
                        { value: 'top', label: 'Top', icon: PanelTop },
                      ]}
                      onChange={(tabLayout) => onUpdateSettings({ tabLayout })}
                    />
                  </Row>
                  <Row label="Top bar buttons">
                    <Segmented
                      label="Top bar style"
                      value={settings.topBarStyle === 'pill' ? 'pill' : 'flat'}
                      options={[
                        { value: 'flat', label: 'Flat' },
                        { value: 'pill', label: 'Outlined' },
                      ]}
                      onChange={(topBarStyle) => onUpdateSettings({ topBarStyle })}
                    />
                  </Row>
                  <Row label="Hide the top bar on websites" hint="It slides in when the pointer reaches the top edge">
                    <Switch
                      checked={settings.topBarAutoHide === true}
                      label="Hide the top bar on websites"
                      onChange={(topBarAutoHide) => onUpdateSettings({ topBarAutoHide })}
                    />
                  </Row>
                  <Row label="Website dock" hint={RIGHT_PINS_DOCK_HINT}>
                    <Switch
                      checked={settings.rightPinsDockVisible}
                      label="Website dock"
                      onChange={(rightPinsDockVisible) => onUpdateSettings({ rightPinsDockVisible })}
                    />
                  </Row>
                  <Row label="Text size" hint="The interface and newly opened pages">
                    <Segmented
                      label="Text size"
                      value={TEXT_SCALE_OPTIONS.find((o) => Math.abs((settings.textScale ?? 1) - o.value) < 0.001)?.value ?? 1}
                      options={TEXT_SCALE_OPTIONS}
                      onChange={(textScale) => onUpdateSettings({ textScale })}
                    />
                  </Row>
                </Group>

                <Group title="New tab">
                  <Row label="Your name" hint="Used in the greeting" htmlFor={nameId}>
                    <input
                      id={nameId}
                      className="settings-input settings-name-input"
                      type="text"
                      value={nameDraft}
                      maxLength={40}
                      onChange={(e) => setNameDraft(e.target.value)}
                      onBlur={commitName}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') {
                          e.preventDefault();
                          commitName();
                        }
                      }}
                    />
                  </Row>
                  <Row label="Greeting and clock">
                    <Switch
                      checked={widgets.clock}
                      label="Greeting and clock"
                      onChange={(clock) => onUpdateSettings({ homeWidgets: { ...widgets, clock } })}
                    />
                  </Row>
                  <Row label="Search bar" hint="With it hidden, start typing on a new tab to search">
                    <Switch
                      checked={widgets.search}
                      label="Search bar"
                      onChange={(search) => onUpdateSettings({ homeWidgets: { ...widgets, search } })}
                    />
                  </Row>
                  <Row
                    label="Widgets"
                    hint={`${widgets.items.length === 0 ? 'None yet' : `${widgets.items.length} on the new tab`}. Add, move and resize them with Customize, at the bottom right of a new tab`}
                  />
                </Group>

                <Group title="Sizes">
                  {(
                    [
                      ['sidebar', 'Sidebar'],
                      ['dock', 'Website dock'],
                      ['hero', 'Greeting and clock'],
                      ['search', 'Search bar'],
                      ['widgets', 'Widgets'],
                    ] as const
                  ).map(([key, label]) => (
                    <Row key={key} label={label}>
                      <ScaleSlider
                        label={`${label} size`}
                        value={uiScale[key]}
                        onChange={(v) => onUpdateSettings({ uiScale: { ...uiScale, [key]: v } })}
                      />
                    </Row>
                  ))}
                </Group>

                <Group title="Guides">
                  <Row label="Show me around" hint="Short tours of the sidebar and of new-tab widgets">
                    <button
                      type="button"
                      className="btn"
                      onClick={() => {
                        onClose();
                        startGuide('sidebar');
                      }}
                    >
                      Sidebar
                    </button>
                    <button
                      type="button"
                      className="btn"
                      onClick={() => {
                        onClose();
                        startGuide('widgets');
                      }}
                    >
                      Widgets
                    </button>
                  </Row>
                </Group>

                <Group title="Top bar menu">
                  {TOOLS.map((tool) => {
                    const isPinned = pinnedTools.includes(tool.key);
                    const Icon = tool.icon;
                    return (
                      <div className="settings-row" key={tool.key}>
                        <span className="settings-row-icon" aria-hidden="true">
                          <Icon size={15} />
                        </span>
                        <div className="settings-row-text">
                          <span className="settings-row-label">{tool.name}</span>
                          <span className="settings-hint">{tool.hint}</span>
                        </div>
                        <div className="settings-row-control">
                          {toolActions && (
                            <button
                              type="button"
                              className="btn btn-quiet"
                              onClick={() => {
                                onClose();
                                toolActions[tool.key]();
                              }}
                            >
                              Open
                            </button>
                          )}
                          <Switch
                            checked={isPinned}
                            label={`Show ${tool.name} in the top bar menu`}
                            onChange={(next) =>
                              onUpdateSettings({
                                pinnedTools: next ? [...pinnedTools, tool.key] : pinnedTools.filter((k) => k !== tool.key),
                              })
                            }
                          />
                        </div>
                      </div>
                    );
                  })}
                </Group>
              </>,
            )}

            {panel(
              'privacy',
              <>
                <Group>
                  <Row label="Shields" hint="Blocks ads, trackers and fingerprinting on every site">
                    <Switch
                      checked={settings.adblockEnabled}
                      label="Shields"
                      onChange={(adblockEnabled) => onUpdateSettings({ adblockEnabled })}
                    />
                  </Row>
                  <Row label="Extensions" hint="Add, turn off or remove Chrome Web Store extensions">
                    <button
                      type="button"
                      className="btn"
                      onClick={() => {
                        onClose();
                        window.dispatchEvent(new CustomEvent('serica:open-extensions'));
                      }}
                    >
                      <Puzzle size={13} aria-hidden="true" />
                      Manage
                    </button>
                  </Row>
                </Group>
                <CookieSettings settings={settings} onUpdateSettings={onUpdateSettings} />
                <p className="settings-footnote">Cookies, sessions and site storage are kept separately for each profile.</p>
              </>,
            )}

            {panel(
              'search',
              <Group>
                <Row label="Search engine" hint="For searches typed in the address bar">
                  <Segmented
                    label="Search engine"
                    value={settings.searchEngine}
                    options={ENGINE_IDS.map((id) => ({ value: id, label: SEARCH_ENGINES[id].label }))}
                    onChange={(searchEngine) => onUpdateSettings({ searchEngine })}
                  />
                </Row>
              </Group>,
            )}

            {panel(
              'performance',
              <>
                <Group title="Memory">
                  <Row label="Memory saver" hint={zenSleepHint(settings.tabSleepMinutes)}>
                    <Switch
                      checked={settings.zenSleepEnabled}
                      label="Memory saver"
                      onChange={(zenSleepEnabled) => onUpdateSettings({ zenSleepEnabled })}
                    />
                  </Row>
                  {settings.zenSleepEnabled && (
                    <Row label="Free memory from tabs after">
                      <Segmented
                        label="Free memory from tabs after"
                        value={settings.tabSleepMinutes}
                        options={TAB_SLEEP_OPTIONS.map((v) => ({ value: v, label: tabSleepLabel(v) }))}
                        onChange={(tabSleepMinutes) => onUpdateSettings({ tabSleepMinutes })}
                      />
                    </Row>
                  )}
                  <Row label="Archive tabs after" hint={ARCHIVE_INTERVAL_HINT}>
                    <Segmented
                      label="Archive interval"
                      value={settings.archiveIntervalHours}
                      options={ARCHIVE_INTERVAL_OPTIONS.map((v) => ({ value: v, label: archiveIntervalLabel(v) }))}
                      onChange={(archiveIntervalHours) => onUpdateSettings({ archiveIntervalHours })}
                    />
                  </Row>
                </Group>
                <Group title="Media and documents">
                  <Row label="Video mini player" hint="Keep watching in a floating window when you switch away from a playing video">
                    <Switch
                      checked={settings.autoPictureInPicture}
                      label="Video mini player"
                      onChange={(autoPictureInPicture) => onUpdateSettings({ autoPictureInPicture })}
                    />
                  </Row>
                  <Row
                    label="Dark PDFs"
                    hint="In dark mode, PDF pages are dark and text is light; chart colors stay as they are. Each PDF can switch back from the address bar"
                  >
                    <Switch checked={settings.pdfDarkMode} label="Dark PDFs" onChange={(pdfDarkMode) => onUpdateSettings({ pdfDarkMode })} />
                  </Row>
                </Group>
              </>,
            )}

            {panel('profiles', <ProfilesSettings tabs={tabs} />)}

            {panel(
              'passwords',
              <p className="settings-prose">
                When you sign in to a website, Serica offers to save the password, and fills it in the next time you visit.
                Addresses and other form entries are filled in the same way. Passwords are encrypted on this device and kept
                separately for each profile.
              </p>,
            )}

            {panel('about', <AboutSettings active={isOpen && activeTab === 'about'} />)}
          </div>
        </main>
      </div>
    </div>
  );
};
