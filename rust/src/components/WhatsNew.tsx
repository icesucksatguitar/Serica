import React, { useEffect, useState } from 'react';
import { Sparkles, X } from 'lucide-react';
import { tauriBridge } from '../adapter/tauriBridge';
import { useElementHole } from '../hooks/useViewportOverlay';

/** What changed in each version, shown once after Serica updates to it. */
export const RELEASE_NOTES: Record<string, { title: string; text: string }[]> = {
  '0.5.0': [
    {
      title: 'Focus Mode, with statistics',
      text: 'Block distracting sites for a while or on a schedule, in every tab and profile. Lock it and nothing ends it early, not even a restart. Statistics show your focus time, streak, what you reached for and where your time went.',
    },
    {
      title: 'A quieter new tab',
      text: 'Only your widgets, now liquid glass, and the clock. Press and hold to move a widget anywhere. Pick a background: a colour, a gradient, your own picture, or Serica Special, three oil-pastel paintings.',
    },
    {
      title: 'Theme Studio and Settings, rebuilt',
      text: 'Curated palettes with a live preview, settings regrouped with search (Ctrl+F), and glass panels that let what is behind them show through. Bring bookmarks, history and passwords over from other browsers, and make Serica your default browser.',
    },
    {
      title: 'Smoother everywhere',
      text: 'Panels open and close with motion, the cursor stays smooth, the sidebar takes in the site controls when it opens, and the window controls are one line that turns into the control you point at.',
    },
    {
      title: 'Drag and drop, and permission prompts',
      text: 'Drop links, text and files onto Serica to open them. Sites asking for your camera, microphone or location now ask in Serica, and it remembers your answer per site.',
    },
  ],
  '0.4.7': [
    {
      title: 'A new look for Serica',
      text: 'A new icon, and a redesigned sidebar: calmer rows, your space shown up top, and actions that appear when you need them.',
    },
    {
      title: 'Widgets on the new tab',
      text: 'Add GitHub contributions, weather, notes, a calendar, your top sites, or quick widgets for Claude, Notion, Gmail and more. Use Customize > Edit widgets to move and resize them.',
    },
    {
      title: 'Faster space swipes',
      text: 'Swiping sideways on the sidebar switches spaces as soon as you swipe, instead of waiting for the trackpad to settle.',
    },
    {
      title: 'Sizes your way',
      text: 'Make the sidebar, dock, clock, search bar and widgets bigger or smaller, each on its own, in Settings > Appearance > Sizes.',
    },
    {
      title: 'Little things',
      text: 'The top bar is back to its slim size, the dock magnifies again, and short guides show you around (replay them in Settings > Appearance).',
    },
  ],
  '0.4.6': [
    {
      title: 'Instant space switching',
      text: 'Switching spaces, by click, swipe or Ctrl+1-9, now happens at once, and the sidebar never shows one space while you are still in another.',
    },
    {
      title: 'Manage spaces from the sidebar',
      text: 'Add a space with +, double-click its name to rename it, or use ⋯ or a right-click to change its icon and color, move it, add a folder or delete it.',
    },
    {
      title: 'Folders that take drops',
      text: 'Drag any tab into a folder, between folders or back out. The folder lights up when a drop will land in it.',
    },
    {
      title: 'Serica’s own right-click menu',
      text: 'Right-clicking a page shows Serica’s menu, with copy, paste, spelling, Inspect and new-tab options for links and images.',
    },
    {
      title: 'Better dark PDFs',
      text: 'Pages turn dark and text light, while chart colors stay true. PDFs follow light and dark mode, and the address bar has a button to show a PDF in its own colors.',
    },
    {
      title: 'New tab widgets',
      text: 'Add your GitHub contributions, the weather or a notes pad to the new tab page with Customize.',
    },
    {
      title: 'A calmer, steadier look',
      text: 'Settings no longer shifts around, and Appearance, the VPN panel and the website dock are simpler and flatter. The top bar has more room and stays put on websites.',
    },
  ],
  '0.4.5': [
    {
      title: 'A slimmer top bar',
      text: 'The top bar is thinner and quieter, and on websites it hides until you move the pointer to the top edge. Choose Flat or Pills, or turn auto-hide off, in Settings > Appearance.',
    },
    {
      title: 'Your tools, your way',
      text: 'The ⋯ menu now holds just light/dark and Settings. Every tool is in Settings > Appearance > Top bar tools, where you can pin the ones you want back on the top bar.',
    },
    {
      title: 'Cookie controls',
      text: 'Block third-party cookies, clear cookies when Serica closes (keeping the sites you choose), and see or delete every site’s cookies in Settings > Privacy.',
    },
    {
      title: 'Passwords fill in',
      text: 'Serica offers to save passwords when you sign in and fills them in next time. Addresses and other forms fill in too.',
    },
    {
      title: 'Zoom is remembered per site',
      text: 'Zoom YouTube (or any site) once and it stays that size in every tab, even after a restart.',
    },
    {
      title: 'Links open again',
      text: 'Links that open in a new tab work with a normal click, and shortcuts like Ctrl+T work right after you close a tab.',
    },
    {
      title: 'Smoother and calmer',
      text: 'Spaces slide in without a dark flash or a name popping up, recently used tabs stay ready so switching back is instant, the sidebar is a solid color, and tabs free memory quietly in the background.',
    },
    {
      title: 'Small touches',
      text: 'A Serica right-click menu replaces the Edge one, scrollbars are slim, and the address bar stays put when the sidebar opens. Four-finger swipes are gone.',
    },
  ],
  '0.4.4': [
    {
      title: 'Open PDFs in Serica',
      text: 'Press Ctrl+O (or Open file in the tools menu), click a downloaded PDF, paste a file path into the address bar, or choose Serica under Open with in Windows.',
    },
    {
      title: 'Dark PDFs',
      text: 'PDFs open with black pages and white text. Websites are not affected. You can turn it off in Settings.',
    },
    {
      title: 'Smoother browsing',
      text: 'Pages full of ads and trackers no longer make Serica stutter while they load, and saving your tabs no longer holds up the window.',
    },
    {
      title: 'Smoother space switching',
      text: 'Switching spaces no longer flashes an old page or a New Tab page first, and the tab you just left stays awake.',
    },
  ],
};

const SEEN_KEY = 'serica_last_seen_version';

function readSeen(): string | null {
  try {
    return localStorage.getItem(SEEN_KEY);
  } catch {
    return null;
  }
}

function writeSeen(v: string) {
  try {
    localStorage.setItem(SEEN_KEY, v);
  } catch {
    /* private storage unavailable: the card may show again, harmless */
  }
}

/**
 * A card listing what's new, shown once on the first launch after an update.
 * A fresh install, or an update that shows the setup screen (which already
 * says what's new, and is followed by the tour), only records the version.
 */
export const WhatsNew: React.FC<{ onboarded: boolean | null }> = ({ onboarded }) => {
  const [version, setVersion] = useState<string | null>(null);
  const [el, setEl] = useState<HTMLDivElement | null>(null);

  useEffect(() => {
    // Not known yet (settings loading): decide once it is.
    if (onboarded === null) return;
    let alive = true;
    void tauriBridge
      .getUpdateStatus()
      .then((s) => {
        const current = s?.currentVersion;
        if (!alive || !current) return;
        if (!onboarded) {
          writeSeen(current);
          return;
        }
        if (readSeen() !== current && RELEASE_NOTES[current]) setVersion(current);
        else writeSeen(current);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [onboarded]);

  // Shows through the live page (tab views paint above the HTML UI).
  useElementHole('whats-new', version ? el : null);

  if (!version) return null;
  const close = () => {
    writeSeen(version);
    setVersion(null);
  };

  return (
    <div ref={setEl} className="whats-new" role="dialog" aria-labelledby="whats-new-title">
      <div className="whats-new-head">
        <Sparkles size={16} className="update-notice-icon" aria-hidden="true" />
        <p id="whats-new-title" className="update-notice-title">
          Serica updated to {version}
        </p>
        <button type="button" className="update-notice-close" aria-label="Close" title="Close" onClick={close}>
          <X size={14} />
        </button>
      </div>
      <ul className="whats-new-list">
        {RELEASE_NOTES[version].map((n) => (
          <li key={n.title}>
            <span className="whats-new-item-title">{n.title}</span>
            <span className="whats-new-item-text">{n.text}</span>
          </li>
        ))}
      </ul>
      <div className="whats-new-actions">
        <button type="button" className="update-notice-primary" onClick={close}>
          Got it
        </button>
      </div>
    </div>
  );
};
