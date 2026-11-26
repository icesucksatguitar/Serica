import React from 'react';
import type { LucideIcon } from 'lucide-react';

/** A titled group: small heading with its icon, then its rows in one card. */
export const Group: React.FC<{ title: string; icon?: LucideIcon; children: React.ReactNode; note?: React.ReactNode }> = ({ title, icon: Icon, children, note }) => (
  <section className="s2-group">
    <h3 className="s2-group-title">
      {Icon && <Icon size={12} aria-hidden="true" />}
      {title}
    </h3>
    <div className="s2-card">{children}</div>
    {note && <p className="s2-note">{note}</p>}
  </section>
);

/**
 * One setting: icon, title, one short line, and its control on the right
 * (or below, for wide controls). `id` lets "Find a setting" jump to it.
 */
export const Row: React.FC<{
  id: string;
  icon?: LucideIcon;
  title: React.ReactNode;
  detail?: React.ReactNode;
  children?: React.ReactNode;
  wide?: boolean;
  disabled?: boolean;
}> = ({ id, icon: Icon, title, detail, children, wide, disabled }) => (
  <div className={`s2-row${wide ? ' wide' : ''}${disabled ? ' disabled' : ''}`} data-setting-id={id}>
    {Icon && (
      <span className="s2-row-icon" aria-hidden="true">
        <Icon size={15} />
      </span>
    )}
    <div className="s2-row-text">
      <span className="s2-row-title">{title}</span>
      {detail && <span className="s2-row-detail">{detail}</span>}
    </div>
    {children !== undefined && <div className="s2-row-control">{children}</div>}
  </div>
);

export type SettingsPage =
  | 'general'
  | 'appearance'
  | 'home'
  | 'tabs'
  | 'focus'
  | 'spaces'
  | 'passwords'
  | 'privacy'
  | 'ai'
  | 'extensions'
  | 'keyboard'
  | 'about';

/** Every setting, written down once: the pages draw rows with these ids and
 *  "Find a setting" searches this list. */
export interface IndexEntry {
  id: string;
  page: SettingsPage;
  group: string;
  title: string;
  detail: string;
  words?: string[];
}

export const SETTINGS_INDEX: IndexEntry[] = [
  { id: 'general.default', page: 'general', group: 'Your browser', title: 'Default browser', detail: 'Open links from other apps in Serica', words: ['default', 'windows', 'links', 'mail'] },
  { id: 'general.import', page: 'general', group: 'Your browser', title: 'Bring things over', detail: 'Bookmarks, history, passwords and profiles from another browser', words: ['import', 'chrome', 'edge', 'firefox', 'brave', 'arc', 'migrate', 'bookmarks', 'history'] },
  { id: 'general.search', page: 'general', group: 'Searching', title: 'Search engine', detail: 'For searches typed in the address bar', words: ['google', 'duckduckgo', 'bing', 'brave'] },
  { id: 'general.downloads', page: 'general', group: 'Downloads', title: 'Downloads folder', detail: 'Where downloaded files go', words: ['download', 'folder'] },
  { id: 'general.setup', page: 'general', group: 'Your browser', title: 'Run setup again', detail: 'The welcome screen, from the start', words: ['setup', 'welcome', 'onboarding'] },

  { id: 'appearance.theme', page: 'appearance', group: 'Light and dark', title: 'Appearance', detail: 'Light, dark, or match Windows, for this space', words: ['dark', 'light', 'mode', 'theme'] },
  { id: 'appearance.studio', page: 'appearance', group: 'Light and dark', title: 'Theme Studio', detail: 'Colours for each space', words: ['colour', 'color', 'palette', 'theme'] },
  { id: 'appearance.layout', page: 'appearance', group: 'Window', title: 'Tabs', detail: 'Down the side, or across the top', words: ['sidebar', 'top tabs', 'layout'] },
  { id: 'appearance.topbar', page: 'appearance', group: 'Window', title: 'Top bar buttons', detail: 'Flat, or outlined', words: ['toolbar', 'buttons'] },
  { id: 'appearance.autohide', page: 'appearance', group: 'Window', title: 'Hide the top bar on websites', detail: 'It slides in when the pointer reaches the top edge', words: ['autohide', 'fullscreen'] },
  { id: 'appearance.dock', page: 'appearance', group: 'Window', title: 'Website dock', detail: 'One-click sites on the right edge', words: ['dock', 'pins'] },
  { id: 'appearance.textsize', page: 'appearance', group: 'Text and sizes', title: 'Text size', detail: 'The interface and newly opened pages', words: ['zoom', 'font', 'scale'] },
  { id: 'appearance.sizes', page: 'appearance', group: 'Text and sizes', title: 'Sizes', detail: 'Sidebar, dock and widgets, each on its own', words: ['scale', 'size'] },
  { id: 'appearance.toolsmenu', page: 'appearance', group: 'Top bar menu', title: 'Tools in the menu', detail: 'Which tools the ⋯ menu shows', words: ['menu', 'tools', 'history', 'downloads', 'vpn'] },

  { id: 'home.background', page: 'home', group: 'Background', title: 'New tab background', detail: 'Colour, gradient, your picture, or Serica Special', words: ['wallpaper', 'image', 'picture', 'gradient', 'colour', 'color', 'special', 'art', 'slideshow'] },
  { id: 'home.clock', page: 'home', group: 'Clock', title: 'Clock', detail: '24-hour time and seconds', words: ['clock', 'time', '24', 'seconds'] },
  { id: 'home.widgets', page: 'home', group: 'Widgets', title: 'Widgets', detail: 'Add, move and resize from the new tab', words: ['widgets', 'weather', 'notes', 'calendar'] },

  { id: 'tabs.memory', page: 'tabs', group: 'Memory', title: 'Memory saver', detail: 'Frees memory from background tabs you haven’t used for a while', words: ['sleep', 'ram', 'memory', 'hibernate'] },
  { id: 'tabs.archive', page: 'tabs', group: 'Closing and archiving', title: 'Archive Today tabs after', detail: 'Inactive Today tabs move to the archive', words: ['archive', 'close'] },
  { id: 'tabs.pip', page: 'tabs', group: 'Video and documents', title: 'Video mini player', detail: 'Keep watching in a floating window when you switch tabs', words: ['picture in picture', 'pip', 'video'] },
  { id: 'tabs.pdf', page: 'tabs', group: 'Video and documents', title: 'Dark PDFs', detail: 'In dark mode, PDFs have dark pages and light text', words: ['pdf', 'dark'] },

  { id: 'focus.lists', page: 'focus', group: 'Focus lists', title: 'Focus Mode', detail: 'Block distracting sites for a while', words: ['block', 'blocker', 'cold turkey', 'productivity', 'focus', 'distraction', 'schedule', 'lock'] },
  { id: 'focus.stats', page: 'focus', group: 'Focus', title: 'Focus statistics', detail: 'Time focused, blocked attempts, streaks, time on sites', words: ['stats', 'statistics', 'usage', 'screen time', 'report', 'streak', 'history', 'time spent', 'export', 'csv', 'cold turkey'] },

  { id: 'spaces.profiles', page: 'spaces', group: 'Spaces and profiles', title: 'Profiles and spaces', detail: 'Separate sign-ins and sets of tabs', words: ['profile', 'space', 'workspace'] },

  { id: 'passwords.saving', page: 'passwords', group: 'Your passwords', title: 'Saving and filling', detail: 'Serica offers to save passwords and fills them in', words: ['password', 'autofill', 'login'] },
  { id: 'passwords.import', page: 'passwords', group: 'Your passwords', title: 'Import passwords', detail: 'From another browser, or a password export file', words: ['csv', 'import', 'chrome', 'bitwarden', '1password'] },

  { id: 'privacy.shields', page: 'privacy', group: 'Blocking', title: 'Shields', detail: 'Blocks ads, trackers and fingerprinting', words: ['adblock', 'ads', 'trackers'] },
  { id: 'privacy.permissions', page: 'privacy', group: 'Site permissions', title: 'Site permissions', detail: 'Camera, microphone, location and more, by site', words: ['camera', 'microphone', 'mic', 'location', 'notifications', 'clipboard', 'permission'] },
  { id: 'privacy.cookies', page: 'privacy', group: 'Cookies', title: 'Cookies', detail: 'Third-party cookies, clearing on exit, sites to keep', words: ['cookies', 'sign out', 'third party'] },
  { id: 'privacy.history', page: 'privacy', group: 'Clear', title: 'Browsing history', detail: 'See or clear what you’ve visited', words: ['history', 'clear', 'delete'] },
  { id: 'privacy.tunnel', page: 'privacy', group: 'Connection', title: 'Privacy tunnel', detail: 'DNS over HTTPS, Tor, or your own proxy', words: ['vpn', 'proxy', 'tor', 'dns'] },

  { id: 'ai.assistants', page: 'ai', group: 'Assistants', title: 'Assistants', detail: 'Claude and others in a panel beside the page', words: ['ai', 'claude', 'chatgpt', 'assistant'] },
  { id: 'ai.mcp', page: 'ai', group: 'Agents', title: 'Let AI agents use Serica', detail: 'A local MCP endpoint for Claude and other agents', words: ['mcp', 'agent', 'automation'] },

  { id: 'extensions.manage', page: 'extensions', group: 'Installed', title: 'Extensions', detail: 'Add, turn off or remove Chrome Web Store extensions', words: ['extension', 'addon', 'plugin', 'chrome web store'] },

  { id: 'keyboard.shortcuts', page: 'keyboard', group: 'Shortcuts', title: 'Keyboard shortcuts', detail: 'Every shortcut Serica knows', words: ['keys', 'hotkeys', 'shortcuts'] },
  { id: 'keyboard.gestures', page: 'keyboard', group: 'Mouse and touchpad', title: 'Swipe gestures', detail: 'Two fingers sideways to go back and forward or switch spaces', words: ['touchpad', 'trackpad', 'swipe', 'gesture'] },

  { id: 'about.version', page: 'about', group: 'Updates', title: 'Version and updates', detail: 'Serica updates itself', words: ['update', 'version'] },
  { id: 'about.tour', page: 'about', group: 'Guides', title: 'Take the tour again', detail: 'A short walk through Serica', words: ['tour', 'help', 'guide'] },
  { id: 'about.credits', page: 'about', group: 'Credits', title: 'Credits', detail: 'Open-source software and the Serica Special artworks', words: ['license', 'credits', 'art'] },
];

export function searchSettings(q: string): IndexEntry[] {
  const words = q.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return [];
  return SETTINGS_INDEX.map((e) => {
    const hay = `${e.title} ${e.detail} ${e.group} ${(e.words ?? []).join(' ')}`.toLowerCase();
    if (!words.every((w) => hay.includes(w))) return null;
    const score = words.reduce((s, w) => s + (e.title.toLowerCase().includes(w) ? 3 : 0) + ((e.words ?? []).some((x) => x.startsWith(w)) ? 2 : 0), 0);
    return { e, score };
  })
    .filter((x): x is { e: IndexEntry; score: number } => !!x)
    .sort((a, b) => b.score - a.score)
    .map((x) => x.e);
}
