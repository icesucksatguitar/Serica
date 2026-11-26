import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
  AppWindow,
  ArrowDownToLine,
  Bot,
  Clock,
  Cookie,
  Download,
  ExternalLink,
  Gauge,
  Globe,
  History,
  Image as ImageIcon,
  Info,
  KeyRound,
  Keyboard,
  Layers,
  LayoutGrid,
  Link,
  Monitor,
  Moon,
  MousePointer2,
  Palette,
  PanelLeft,
  PanelTop,
  Puzzle,
  RotateCcw,
  Search,
  Settings as Gear,
  Shield,
  ShieldCheck,
  Sparkles,
  Sun,
  Target,
  Trash2,
  Type,
  Upload,
  X,
} from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import {
  ARCHIVE_INTERVAL_OPTIONS,
  DEFAULT_HOME_BACKGROUND,
  DEFAULT_HOME_WIDGETS,
  DEFAULT_UI_SCALE,
  SEARCH_ENGINES,
  TAB_SLEEP_OPTIONS,
  TEXT_SCALE_OPTIONS,
  type ArchiveIntervalHours,
  type SearchEngineId,
  type TabSleepMinutes,
  type UiScale,
} from '../../shared/settings';
import type { BrowserSettings, TabState } from '../../types';
import { TOOLS, normalizePinnedTools, type ToolActions } from '../../shared/tools';
import { Switch } from '../Switch';
import { Segmented } from '../home/WidgetEditor';
import { ProfilesSettings } from '../ProfilesSettings';
import { AboutSettings } from '../AboutSettings';
import { CookieSettings } from '../CookieSettings';
import { PERMISSION_COPY } from '../PermissionPrompt';
import { Group, Row, searchSettings, SETTINGS_INDEX, type SettingsPage } from './parts';
import { FocusSettings } from './FocusSettings';
import { ImportPanel } from './ImportPanel';
import { HomeBackgroundSettings } from './HomeBackgroundSettings';
import { SPECIAL_ART } from '../Homescreen';

const PAGES: { id: SettingsPage; label: string; icon: LucideIcon }[] = [
  { id: 'general', label: 'General', icon: Gear },
  { id: 'appearance', label: 'Appearance', icon: Palette },
  { id: 'home', label: 'New Tab', icon: ImageIcon },
  { id: 'tabs', label: 'Tabs', icon: AppWindow },
  { id: 'focus', label: 'Focus', icon: Target },
  { id: 'spaces', label: 'Spaces & Profiles', icon: Layers },
  { id: 'passwords', label: 'Passwords', icon: KeyRound },
  { id: 'privacy', label: 'Privacy', icon: Shield },
  { id: 'ai', label: 'AI', icon: Sparkles },
  { id: 'extensions', label: 'Extensions', icon: Puzzle },
  { id: 'keyboard', label: 'Keyboard & Mouse', icon: Keyboard },
  { id: 'about', label: 'About', icon: Info },
];

const ENGINE_IDS = Object.keys(SEARCH_ENGINES) as SearchEngineId[];
const tabSleepLabel = (m: TabSleepMinutes) => (m < 60 ? `${m} min` : `${m / 60} hr`);
const ARCHIVE_LABEL: Record<string, string> = { '12': '12 hours', '24': '1 day', '168': '7 days', '720': '30 days', never: 'Never' };
const SCALES = [
  { value: 0.9, label: 'Smaller' },
  { value: 1, label: 'Default' },
  { value: 1.15, label: 'Larger' },
  { value: 1.3, label: 'Largest' },
];
const nearestScale = (v: number) => SCALES.reduce((b, s) => (Math.abs(s.value - v) < Math.abs(b.value - v) ? s : b), SCALES[1]).value;

const SHORTCUTS: [string, string][] = [
  ['Ctrl T', 'New tab: type a place, or words to search'],
  ['Ctrl K or Ctrl L', 'The address bar, open tabs and commands (type >)'],
  ['Ctrl W', 'Close the tab'],
  ['Ctrl Shift T', 'Reopen the last closed tab'],
  ['Ctrl Tab / Ctrl Shift Tab', 'Next / previous tab'],
  ['Ctrl 1 … 9', 'Go to space 1 … 9'],
  ['Ctrl S or Ctrl E', 'Show or hide the sidebar'],
  ['Ctrl F', 'Find in page'],
  ['Ctrl R or F5', 'Reload (Ctrl Shift R ignores the cache)'],
  ['Alt ← / Alt →', 'Back / forward'],
  ['Alt Home', 'The new tab page'],
  ['Ctrl H', 'History'],
  ['Ctrl J', 'Downloads'],
  ['Ctrl O', 'Open a file'],
  ['Ctrl ,', 'Settings (Ctrl F here finds a setting)'],
  ['Ctrl + / Ctrl − / Ctrl 0', 'Zoom in / out / reset'],
  ['F12', 'Developer tools'],
];

/** Settings › Privacy › Site permissions: remembered answers, by site. */
const SitePermissions: React.FC<{ settings: BrowserSettings; onUpdateSettings: (p: Partial<BrowserSettings>) => void }> = ({ settings, onUpdateSettings }) => {
  const map = ((settings as any).sitePermissions ?? {}) as Record<string, Record<string, 'allow' | 'block'>>;
  const sites = Object.keys(map).sort();
  const save = (next: Record<string, Record<string, string>>) => onUpdateSettings({ sitePermissions: next } as any);
  if (sites.length === 0) {
    return <p className="s2-empty">When you allow or block a site’s request for your camera, microphone, location and the like, it shows up here.</p>;
  }
  return (
    <>
      {sites.map((origin) => (
        <div key={origin} className="s2-row perm-row">
          <span className="s2-row-icon" aria-hidden="true">
            <img src={`https://www.google.com/s2/favicons?domain=${encodeURIComponent(new URL(origin).hostname)}&sz=32`} alt="" width={16} height={16} />
          </span>
          <div className="s2-row-text">
            <span className="s2-row-title">{origin.replace(/^https?:\/\//, '')}</span>
            <span className="perm-kinds">
              {Object.entries(map[origin]).map(([kind, state]) => {
                const meta = PERMISSION_COPY[kind];
                const Icon = meta?.Icon ?? Globe;
                return (
                  <button
                    key={kind}
                    type="button"
                    className={`perm-kind ${state}`}
                    title={`${meta?.label ?? kind}: ${state === 'allow' ? 'allowed' : 'blocked'}. Click to ${state === 'allow' ? 'block' : 'allow'}.`}
                    onClick={() => save({ ...map, [origin]: { ...map[origin], [kind]: state === 'allow' ? 'block' : 'allow' } })}
                  >
                    <Icon size={12} aria-hidden="true" />
                    {meta?.label ?? kind}
                    <span className="perm-state">{state === 'allow' ? 'Allowed' : 'Blocked'}</span>
                  </button>
                );
              })}
            </span>
          </div>
          <div className="s2-row-control">
            <button
              type="button"
              className="btn btn-quiet btn-icon"
              aria-label={`Forget ${origin}`}
              title="Forget: the site asks again next time"
              onClick={() => {
                const next = { ...map };
                delete next[origin];
                save(next);
              }}
            >
              <Trash2 size={14} />
            </button>
          </div>
        </div>
      ))}
    </>
  );
};

/** The default-browser row: status, and the way to change it. */
const DefaultBrowserRow: React.FC = () => {
  const [status, setStatus] = useState<{ isDefault: boolean; supported: boolean } | null>(null);
  const [asked, setAsked] = useState(false);
  useEffect(() => {
    let alive = true;
    const check = () => window.serica?.defaultBrowserStatus?.().then((s: any) => alive && setStatus(s));
    void check();
    // After asking, Windows Settings is where the choice is made: follow it.
    const t = asked ? window.setInterval(check, 1500) : 0;
    return () => {
      alive = false;
      if (t) window.clearInterval(t);
    };
  }, [asked]);
  return (
    <Row id="general.default" icon={Link} title="Default browser" detail={status?.isDefault ? 'Links from other apps open in Serica' : 'Links from Mail, Slack, Teams and the rest open in your default browser'}>
      {status?.isDefault ? (
        <span className="s2-ok">
          <ShieldCheck size={13} aria-hidden="true" /> Serica is the default
        </span>
      ) : (
        <button
          type="button"
          className="btn btn-primary"
          disabled={status?.supported === false}
          onClick={() => {
            setAsked(true);
            void window.serica?.makeDefaultBrowser?.().then((s: any) => s && setStatus(s));
          }}
        >
          Make Serica the default
        </button>
      )}
    </Row>
  );
};

export interface Settings2Props {
  onClose: () => void;
  settings: BrowserSettings;
  onUpdateSettings: (patch: Partial<BrowserSettings>) => void;
  /** Light, dark or follow Windows, for the current space. */
  mode: 'light' | 'dark' | 'auto';
  onSetMode: (mode: 'light' | 'dark' | 'auto') => void;
  onOpenThemeStudio: () => void;
  tabs?: TabState[];
  initialPage?: SettingsPage;
  toolActions?: ToolActions;
  onRunSetup: () => void;
  onStartTour: () => void;
}

/**
 * Settings: one window, a page per area, and "Find a setting" across all of
 * them. Each row is an icon, a title, one short line and its control.
 * Only what works on Windows is here.
 */
export const Settings2: React.FC<Settings2Props> = ({ onClose, settings, onUpdateSettings, mode, onSetMode, onOpenThemeStudio, tabs, initialPage = 'general', toolActions, onRunSetup, onStartTour }) => {
  const [page, setPage] = useState<SettingsPage>(initialPage);
  const [query, setQuery] = useState('');
  const findRef = useRef<HTMLInputElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const results = useMemo(() => searchSettings(query), [query]);

  useEffect(() => setPage(initialPage), [initialPage]);
  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
  }, [page]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f') {
        e.preventDefault();
        e.stopPropagation();
        findRef.current?.focus();
        findRef.current?.select();
      } else if (e.key === 'Escape' && !e.defaultPrevented) {
        if (document.querySelector('.wed-overlay')) return; // a sheet inside handles it
        e.preventDefault();
        if (query) setQuery('');
        else onClose();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [onClose, query]);

  const go = (id: string) => {
    const entry = SETTINGS_INDEX.find((e) => e.id === id);
    if (!entry) return;
    setQuery('');
    setPage(entry.page);
    // The page renders first, then the row is brought into view and lit.
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        const el = scrollRef.current?.querySelector(`[data-setting-id="${id}"]`) as HTMLElement | null;
        if (!el) return;
        el.scrollIntoView({ block: 'center', behavior: 'smooth' });
        el.classList.remove('s2-flash');
        void el.offsetWidth;
        el.classList.add('s2-flash');
        window.setTimeout(() => el.classList.remove('s2-flash'), 1800);
      }),
    );
  };

  const close = () => onClose();
  const openTool = (k: keyof ToolActions) => {
    close();
    toolActions?.[k]?.();
  };
  const topTabs = settings.tabLayout === 'top';
  const uiScale: UiScale = settings.uiScale ?? DEFAULT_UI_SCALE;
  const widgets = settings.homeWidgets ?? DEFAULT_HOME_WIDGETS;
  const pinnedTools = normalizePinnedTools(settings.pinnedTools);
  const current = PAGES.find((p) => p.id === page)!;

  const body = (() => {
    switch (page) {
      case 'general':
        return (
          <>
            <Group title="Your browser" icon={Globe}>
              <DefaultBrowserRow />
              <Row id="general.import" icon={Upload} title="Bring things over" detail="Bookmarks, history, passwords and profiles from another browser on this PC" wide>
                <ImportPanel />
              </Row>
              <Row id="general.setup" icon={RotateCcw} title="Run setup again" detail="The welcome screen, from the start. Nothing you have is changed">
                <button type="button" className="btn" onClick={() => { close(); onRunSetup(); }}>
                  Run setup
                </button>
              </Row>
            </Group>
            <Group title="Searching" icon={Search}>
              <Row id="general.search" icon={Search} title="Search engine" detail="For words typed in the address bar">
                <Segmented label="Search engine" value={settings.searchEngine} options={ENGINE_IDS.map((id) => ({ value: id, label: SEARCH_ENGINES[id].label }))} onChange={(searchEngine) => onUpdateSettings({ searchEngine })} />
              </Row>
            </Group>
            <Group title="Downloads" icon={ArrowDownToLine}>
              <Row id="general.downloads" icon={Download} title="Downloads folder" detail="Files go to your Windows Downloads folder">
                <button type="button" className="btn" onClick={() => void window.serica?.openDownloadsFolder?.()}>
                  Open folder
                </button>
              </Row>
            </Group>
          </>
        );
      case 'appearance':
        return (
          <>
            <Group title="Light and dark" icon={Moon} note="Each space keeps its own appearance and colours.">
              <Row id="appearance.theme" icon={Sun} title="Appearance" detail="For the space you’re in">
                <Segmented
                  label="Appearance"
                  value={mode}
                  options={[
                    { value: 'light', label: 'Light' },
                    { value: 'dark', label: 'Dark' },
                    { value: 'auto', label: 'Match Windows' },
                  ]}
                  onChange={onSetMode}
                />
              </Row>
              <Row id="appearance.studio" icon={Palette} title="Theme Studio" detail="Curated palettes, your own colour, and how strongly it shows">
                <span className="s2-swatches" aria-hidden="true">
                  <span style={{ background: 'var(--theme-backdrop-c1)' }} />
                  <span style={{ background: 'var(--theme-backdrop-c2)' }} />
                  <span style={{ background: 'var(--theme-surface-solid)' }} />
                </span>
                <button type="button" className="btn" onClick={() => { close(); onOpenThemeStudio(); }}>
                  Open Theme Studio
                </button>
              </Row>
            </Group>
            <Group title="Window" icon={Monitor}>
              <Row id="appearance.layout" icon={topTabs ? PanelTop : PanelLeft} title="Tabs" detail="Down the side, or across the top">
                <Segmented label="Tabs" value={topTabs ? 'top' : 'sidebar'} options={[{ value: 'sidebar', label: 'Sidebar' }, { value: 'top', label: 'Top' }]} onChange={(tabLayout) => onUpdateSettings({ tabLayout })} />
              </Row>
              <Row id="appearance.topbar" icon={PanelTop} title="Top bar buttons" detail="Flat, or outlined">
                <Segmented label="Top bar buttons" value={settings.topBarStyle === 'pill' ? 'pill' : 'flat'} options={[{ value: 'flat', label: 'Flat' }, { value: 'pill', label: 'Outlined' }]} onChange={(topBarStyle) => onUpdateSettings({ topBarStyle })} />
              </Row>
              <Row id="appearance.glass" icon={Layers} title="Glass panels" detail="Settings, menus and panels show what is behind them, blurred">
                <Switch checked={settings.glassPanels !== false} label="Glass panels" onChange={(glassPanels) => onUpdateSettings({ glassPanels })} />
              </Row>
              <Row id="appearance.autohide" icon={PanelTop} title="Hide the top bar on websites" detail="It slides in when the pointer reaches the top edge">
                <Switch checked={settings.topBarAutoHide === true} label="Hide the top bar on websites" onChange={(topBarAutoHide) => onUpdateSettings({ topBarAutoHide })} />
              </Row>
              <Row id="appearance.dock" icon={LayoutGrid} title="Website dock" detail="One-click sites on the right edge; on websites it waits at the edge">
                <Switch checked={settings.rightPinsDockVisible} label="Website dock" onChange={(rightPinsDockVisible) => onUpdateSettings({ rightPinsDockVisible })} />
              </Row>
            </Group>
            <Group title="Text and sizes" icon={Type}>
              <Row id="appearance.textsize" icon={Type} title="Text size" detail="The interface and newly opened pages">
                <Segmented
                  label="Text size"
                  value={TEXT_SCALE_OPTIONS.find((o) => Math.abs((settings.textScale ?? 1) - o.value) < 0.001)?.value ?? 1}
                  options={TEXT_SCALE_OPTIONS.map((o) => ({ value: o.value, label: o.label }))}
                  onChange={(textScale) => onUpdateSettings({ textScale })}
                />
              </Row>
              {(
                [
                  ['sidebar', 'Sidebar'],
                  ['dock', 'Website dock'],
                  ['widgets', 'New-tab widgets'],
                  ['hero', 'New-tab clock'],
                ] as const
              ).map(([key, label]) => (
                <Row key={key} id={key === 'sidebar' ? 'appearance.sizes' : `appearance.sizes.${key}`} icon={Gauge} title={label}>
                  <Segmented label={`${label} size`} value={nearestScale(uiScale[key])} options={SCALES} onChange={(v) => onUpdateSettings({ uiScale: { ...uiScale, [key]: v } })} />
                </Row>
              ))}
            </Group>
            <Group title="Top bar menu" icon={MousePointer2} note="The ⋯ menu always has light/dark and Settings.">
              {TOOLS.map((tool) => {
                const isPinned = pinnedTools.includes(tool.key);
                return (
                  <Row key={tool.key} id={tool.key === TOOLS[0].key ? 'appearance.toolsmenu' : `appearance.toolsmenu.${tool.key}`} icon={tool.icon} title={tool.name} detail={tool.hint}>
                    <Switch checked={isPinned} label={`Show ${tool.name} in the menu`} onChange={(next) => onUpdateSettings({ pinnedTools: next ? [...pinnedTools, tool.key] : pinnedTools.filter((k) => k !== tool.key) })} />
                  </Row>
                );
              })}
            </Group>
          </>
        );
      case 'home':
        return (
          <>
            <Group title="Background" icon={ImageIcon}>
              <div className="s2-row wide" data-setting-id="home.background">
                <HomeBackgroundSettings value={settings.homeBackground ?? DEFAULT_HOME_BACKGROUND} onChange={(homeBackground) => onUpdateSettings({ homeBackground })} />
              </div>
            </Group>
            <Group title="Clock" icon={Clock} note="The clock sits in a bottom corner of every new tab.">
              <Row id="home.clock.greeting" icon={Clock} title="Greeting" detail="“Good evening” and your name, above the clock.">
                <Switch checked={widgets.clock.greeting} label="Greeting" onChange={(greeting) => onUpdateSettings({ homeWidgets: { ...widgets, clock: { ...widgets.clock, greeting } } })} />
              </Row>
              <Row id="home.clock" icon={Clock} title="24-hour clock">
                <Switch checked={!widgets.clock.hour12} label="24-hour clock" onChange={(on) => onUpdateSettings({ homeWidgets: { ...widgets, clock: { ...widgets.clock, hour12: !on } } })} />
              </Row>
              <Row id="home.clock.seconds" icon={Clock} title="Seconds">
                <Switch checked={widgets.clock.seconds} label="Seconds" onChange={(seconds) => onUpdateSettings({ homeWidgets: { ...widgets, clock: { ...widgets.clock, seconds } } })} />
              </Row>
              <Row id="home.clock.side" icon={Clock} title="Corner">
                <div className="seg" role="radiogroup" aria-label="Clock corner">
                  {(['left', 'right'] as const).map((side) => (
                    <button key={side} type="button" role="radio" aria-checked={widgets.clock.side === side} className={widgets.clock.side === side ? 'on' : ''} onClick={() => onUpdateSettings({ homeWidgets: { ...widgets, clock: { ...widgets.clock, side } } })}>
                      {side === 'left' ? 'Bottom left' : 'Bottom right'}
                    </button>
                  ))}
                </div>
              </Row>
            </Group>
            <Group title="Widgets" icon={LayoutGrid}>
              <Row id="home.widgets" icon={LayoutGrid} title={`${widgets.items.length || 'No'} widget${widgets.items.length === 1 ? '' : 's'}`} detail="On a new tab, press and hold a widget, or use Customize at the bottom right, to add, move, resize and set them up" />
            </Group>
          </>
        );
      case 'tabs':
        return (
          <>
            <Group title="Memory" icon={Gauge}>
              <Row id="tabs.memory" icon={Gauge} title="Memory saver" detail="Frees memory from background tabs you haven’t used. They reload when you return. Pinned and playing tabs stay awake">
                <Switch checked={settings.zenSleepEnabled} label="Memory saver" onChange={(zenSleepEnabled) => onUpdateSettings({ zenSleepEnabled })} />
              </Row>
              {settings.zenSleepEnabled && (
                <Row id="tabs.memory.after" icon={Clock} title="After">
                  <Segmented label="Free memory after" value={settings.tabSleepMinutes} options={TAB_SLEEP_OPTIONS.map((v) => ({ value: v, label: tabSleepLabel(v) }))} onChange={(tabSleepMinutes) => onUpdateSettings({ tabSleepMinutes })} />
                </Row>
              )}
            </Group>
            <Group title="Closing and archiving" icon={History}>
              <Row id="tabs.archive" icon={History} title="Archive Today tabs after" detail="Inactive Today tabs close and wait in the archive">
                <Segmented label="Archive after" value={settings.archiveIntervalHours} options={ARCHIVE_INTERVAL_OPTIONS.map((v: ArchiveIntervalHours) => ({ value: v, label: ARCHIVE_LABEL[String(v)] }))} onChange={(archiveIntervalHours) => onUpdateSettings({ archiveIntervalHours })} />
              </Row>
            </Group>
            <Group title="Video and documents" icon={AppWindow}>
              <Row id="tabs.pip" icon={AppWindow} title="Video mini player" detail="Keep watching in a floating window when you switch away from a playing video">
                <Switch checked={settings.autoPictureInPicture} label="Video mini player" onChange={(autoPictureInPicture) => onUpdateSettings({ autoPictureInPicture })} />
              </Row>
              <Row id="tabs.pdf" icon={Moon} title="Dark PDFs" detail="In dark mode, dark pages and light text; chart colours stay. Each PDF can switch back from the address bar">
                <Switch checked={settings.pdfDarkMode} label="Dark PDFs" onChange={(pdfDarkMode) => onUpdateSettings({ pdfDarkMode })} />
              </Row>
            </Group>
          </>
        );
      case 'focus':
        return <FocusSettings />;
      case 'spaces':
        return (
          <div data-setting-id="spaces.profiles">
            <ProfilesSettings tabs={tabs} />
          </div>
        );
      case 'passwords':
        return (
          <Group title="Your passwords" icon={KeyRound} note="Passwords are encrypted by Windows on this PC and kept separately for each profile.">
            <Row id="passwords.saving" icon={KeyRound} title="Saving and filling" detail="When you sign in, Serica offers to save the password, and fills it in next time. Addresses and other form entries too" />
            <Row id="passwords.import" icon={Upload} title="Import passwords" detail="From another browser on this PC, or from a password export file" wide>
              <ImportPanel compact />
            </Row>
          </Group>
        );
      case 'privacy':
        return (
          <>
            <Group title="Blocking" icon={Shield}>
              <Row id="privacy.shields" icon={Shield} title="Shields" detail="Blocks ads, trackers and fingerprinting on every site">
                <Switch checked={settings.adblockEnabled} label="Shields" onChange={(adblockEnabled) => onUpdateSettings({ adblockEnabled })} />
              </Row>
            </Group>
            <Group title="Site permissions" icon={ShieldCheck} note="Sites ask before using these. “This time” answers aren’t kept.">
              <div data-setting-id="privacy.permissions">
                <SitePermissions settings={settings} onUpdateSettings={onUpdateSettings} />
              </div>
            </Group>
            <section className="s2-group" data-setting-id="privacy.cookies">
              <h3 className="s2-group-title">
                <Cookie size={12} aria-hidden="true" />
                Cookies
              </h3>
              <div className="s2-legacy">
                <CookieSettings settings={settings} onUpdateSettings={onUpdateSettings} />
              </div>
            </section>
            <Group title="Clear" icon={Trash2}>
              <Row id="privacy.history" icon={History} title="Browsing history" detail="See, search or clear what you’ve visited">
                <button type="button" className="btn" onClick={() => openTool('history')}>
                  Open history
                </button>
              </Row>
            </Group>
            <Group title="Connection" icon={Globe}>
              <Row id="privacy.tunnel" icon={Globe} title="Privacy tunnel" detail="DNS over HTTPS, Tor, or your own proxy, with a kill switch">
                <button type="button" className="btn" onClick={() => openTool('vpn')}>
                  Open
                </button>
              </Row>
            </Group>
          </>
        );
      case 'ai':
        return (
          <>
            <Group title="Assistants" icon={Sparkles}>
              <Row id="ai.assistants" icon={Sparkles} title="Assistants" detail="Claude, ChatGPT, Gemini and others in a panel beside the page, signed in as you">
                <button type="button" className="btn" onClick={() => openTool('ai')}>
                  Open
                </button>
              </Row>
            </Group>
            <Group title="Agents" icon={Bot} note="Off unless you turn it on. Only this PC can connect, with a private token.">
              <Row id="ai.mcp" icon={Bot} title="Let AI agents use Serica" detail="A local MCP endpoint, for Claude Code, Claude Desktop and other agents">
                <button
                  type="button"
                  className="btn"
                  onClick={() => {
                    close();
                    window.dispatchEvent(new CustomEvent('serica:open-mcp', { detail: 'tools' }));
                  }}
                >
                  Set up
                </button>
              </Row>
            </Group>
          </>
        );
      case 'extensions':
        return (
          <Group title="Installed" icon={Puzzle} note="Open an extension’s page in the Chrome Web Store and press “Add to Serica”.">
            <Row id="extensions.manage" icon={Puzzle} title="Extensions" detail="Add, turn off or remove Chrome Web Store extensions">
              <button
                type="button"
                className="btn"
                onClick={() => {
                  close();
                  window.dispatchEvent(new CustomEvent('serica:open-extensions'));
                }}
              >
                Manage
              </button>
            </Row>
          </Group>
        );
      case 'keyboard':
        return (
          <>
            <Group title="Shortcuts" icon={Keyboard}>
              <div className="s2-shortcuts" data-setting-id="keyboard.shortcuts">
                {SHORTCUTS.map(([k, d]) => (
                  <div key={k} className="s2-shortcut">
                    <span className="s2-keys">
                      {k.split(' or ').map((part, i) => (
                        <React.Fragment key={part}>
                          {i > 0 && <span className="s2-or">or</span>}
                          <kbd>{part}</kbd>
                        </React.Fragment>
                      ))}
                    </span>
                    <span className="s2-shortcut-text">{d}</span>
                  </div>
                ))}
              </div>
            </Group>
            <Group title="Mouse and touchpad" icon={MousePointer2}>
              <Row id="keyboard.gestures" icon={MousePointer2} title="Swipe with two fingers" detail="Sideways on a page goes back and forward; on the sidebar it switches spaces" />
              <Row id="keyboard.drag" icon={MousePointer2} title="Drag and drop" detail="Drop files, links or text on a page as usual; drop them on the sidebar, the tab strip or a new tab to open them" />
            </Group>
          </>
        );
      case 'about':
        return (
          <>
            <div data-setting-id="about.version" className="s2-legacy">
              <AboutSettings active />
            </div>
            <Group title="Guides" icon={Info}>
              <Row id="about.tour" icon={Info} title="Take the tour again" detail="The address bar, the sidebar, widgets, Theme Studio and Focus">
                <button type="button" className="btn" onClick={() => { close(); onStartTour(); }}>
                  Start the tour
                </button>
              </Row>
            </Group>
            <Group title="Credits" icon={ImageIcon}>
              <div className="s2-credits" data-setting-id="about.credits">
                <p>Serica is open source (MIT). It uses Microsoft WebView2, Tauri, Brave’s adblock-rust engine with EasyList, EasyPrivacy and uBlock Origin’s lists, Inter by Rasmus Andersson (SIL Open Font License) and Lucide icons.</p>
                <p>Serica Special re-paints three public-domain works in oil pastel:</p>
                <ul>
                  {SPECIAL_ART.map((a) => (
                    <li key={a.src}>
                      {a.title}, <span className="wed-hint">{a.after}</span>
                    </li>
                  ))}
                </ul>
                <button type="button" className="btn btn-quiet" onClick={() => void window.serica?.openExternalUrl?.('https://github.com/icesucksatguitar/Serica')}>
                  <ExternalLink size={13} aria-hidden="true" /> Source code
                </button>
              </div>
            </Group>
          </>
        );
    }
  })();

  return (
    <div className="modal-overlay settings2-overlay" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div className="settings2" role="dialog" aria-modal="true" aria-label="Settings">
        <aside className="s2-nav">
          <div className="s2-nav-title">Settings</div>
          <label className="s2-find">
            <Search size={13} aria-hidden="true" />
            <input ref={findRef} value={query} placeholder="Find a setting" aria-label="Find a setting" spellCheck={false} onChange={(e) => setQuery(e.target.value)} />
            {query ? (
              <button type="button" className="s2-find-clear" aria-label="Clear" onClick={() => setQuery('')}>
                <X size={12} />
              </button>
            ) : (
              <kbd>Ctrl F</kbd>
            )}
          </label>
          <nav className="s2-pages" aria-label="Settings pages">
            {PAGES.map((p) => (
              <button key={p.id} type="button" className={`s2-page${p.id === page && !query ? ' on' : ''}`} aria-current={p.id === page && !query ? 'page' : undefined} onClick={() => { setQuery(''); setPage(p.id); }}>
                <p.icon size={15} aria-hidden="true" />
                <span>{p.label}</span>
              </button>
            ))}
          </nav>
        </aside>
        <main className="s2-main">
          <header className="s2-head">
            <h2>{query ? 'Find a setting' : current.label}</h2>
            <button type="button" className="btn btn-quiet btn-icon" aria-label="Close settings" title="Close (Esc)" onClick={close}>
              <X size={16} />
            </button>
          </header>
          <div className="s2-scroll" ref={scrollRef}>
            {query ? (
              results.length ? (
                <div className="s2-results" role="list">
                  {results.map((r) => (
                    <button key={r.id} type="button" role="listitem" className="s2-result" onClick={() => go(r.id)}>
                      <span className="s2-result-path">
                        {PAGES.find((p) => p.id === r.page)?.label} › {r.group}
                      </span>
                      <span className="s2-result-title">{r.title}</span>
                      <span className="s2-result-detail">{r.detail}</span>
                    </button>
                  ))}
                </div>
              ) : (
                <p className="s2-empty">No setting matches “{query}”.</p>
              )
            ) : (
              <div className="s2-page-body" key={page}>
                {body}
              </div>
            )}
          </div>
        </main>
      </div>
    </div>
  );
};

export default Settings2;
