import React, { useEffect, useState } from 'react';
import { Sparkles, X } from 'lucide-react';
import { tauriBridge } from '../adapter/tauriBridge';
import { useElementHole } from '../hooks/useViewportOverlay';

/** What changed in each version, shown once after Serica updates to it. */
export const RELEASE_NOTES: Record<string, { title: string; text: string }[]> = {
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
 * A fresh install (onboarding not finished) only records the version.
 */
export const WhatsNew: React.FC<{ onboarded: boolean }> = ({ onboarded }) => {
  const [version, setVersion] = useState<string | null>(null);
  const [el, setEl] = useState<HTMLDivElement | null>(null);

  useEffect(() => {
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
