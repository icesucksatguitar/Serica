import React, { useCallback, useState } from 'react';
import { Cookie, Trash2, X } from 'lucide-react';
import type { BrowserSettings } from '../shared/settings';
import { Switch as SettingsSwitch } from './Switch';

interface CookieSettingsProps {
  settings: BrowserSettings;
  onUpdateSettings: (patch: Partial<BrowserSettings>) => void;
}

type SiteList =
  | { state: 'idle' }
  | { state: 'loading' }
  | { state: 'error'; text: string }
  | { state: 'ready'; sites: { site: string; count: number }[] };

/** Turns what a user types ("https://www.Google.com/x") into a site ("google.com"). */
function siteFromInput(input: string): string | null {
  const t = input.trim().toLowerCase();
  if (!t) return null;
  try {
    const host = new URL(t.includes('://') ? t : `https://${t}`).hostname;
    const site = host.replace(/^www\./, '');
    return /^[a-z0-9.-]+\.[a-z0-9-]+$/.test(site) || site === 'localhost' ? site : null;
  } catch {
    return null;
  }
}

/** Cookies and site data: the controls every browser has. */
export const CookieSettings: React.FC<CookieSettingsProps> = ({ settings, onUpdateSettings }) => {
  const keep = settings.cookieKeepSites ?? [];
  const [draft, setDraft] = useState('');
  const [list, setList] = useState<SiteList>({ state: 'idle' });
  const [confirmAll, setConfirmAll] = useState(false);
  const draftSite = siteFromInput(draft);

  const load = useCallback(async () => {
    setList({ state: 'loading' });
    const res = await window.serica?.cookieSites?.().catch(() => null);
    if (!res) {
      setList({ state: 'error', text: 'Cookies could not be read.' });
      return;
    }
    if (!res.ok) {
      setList({
        state: 'error',
        text: res.error === 'no-page' ? 'Open a website in this profile to see its cookies.' : 'Cookies could not be read.',
      });
      return;
    }
    setList({ state: 'ready', sites: res.sites ?? [] });
  }, []);

  const remove = async (site?: string) => {
    await window.serica?.deleteCookies?.(site).catch(() => null);
    setConfirmAll(false);
    await load();
  };

  const addKeep = () => {
    if (!draftSite || keep.includes(draftSite)) return;
    onUpdateSettings({ cookieKeepSites: [...keep, draftSite].sort() });
    setDraft('');
  };

  return (
    <div className="settings-card">
      <div className="settings-row">
        <div className="settings-row-text">
          <span className="settings-row-label">Block third-party cookies</span>
          <span className="settings-hint">
            Sites can't use cookies to follow you across other sites. Sign-ins embedded in other sites may stop working.
            Open pages reload once
          </span>
        </div>
        <SettingsSwitch
          checked={settings.blockThirdPartyCookies === true}
          label="Block third-party cookies"
          onChange={(next) => onUpdateSettings({ blockThirdPartyCookies: next })}
        />
      </div>

      <div className="settings-divider" />

      <div className="settings-row">
        <div className="settings-row-text">
          <span className="settings-row-label">Clear cookies when Serica closes</span>
          <span className="settings-hint">You're signed out of sites each time, except the sites you keep below</span>
        </div>
        <SettingsSwitch
          checked={settings.clearCookiesOnExit === true}
          label="Clear cookies when Serica closes"
          onChange={(next) => onUpdateSettings({ clearCookiesOnExit: next })}
        />
      </div>

      {settings.clearCookiesOnExit && (
        <div className="cookie-keep">
          <span className="settings-row-label">Always keep cookies for</span>
          {keep.length > 0 && (
            <ul className="settings-chip-list">
              {keep.map((site) => (
                <li key={site} className="settings-chip">
                  <span className="settings-chip-label">{site}</span>
                  <button
                    type="button"
                    className="settings-chip-remove"
                    aria-label={`Stop keeping cookies for ${site}`}
                    onClick={() => onUpdateSettings({ cookieKeepSites: keep.filter((s) => s !== site) })}
                  >
                    <X size={11} />
                  </button>
                </li>
              ))}
            </ul>
          )}
          <form
            className="cookie-keep-add"
            onSubmit={(e) => {
              e.preventDefault();
              addKeep();
            }}
          >
            <input
              className="settings-input"
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              placeholder="example.com"
              aria-label="Site to keep cookies for"
              spellCheck={false}
            />
            <button type="submit" className="settings-button" disabled={!draftSite || keep.includes(draftSite)}>
              Add
            </button>
          </form>
        </div>
      )}

      <div className="settings-divider" />

      <div className="settings-row">
        <div className="settings-row-text">
          <span className="settings-row-label">Cookies and site data</span>
          <span className="settings-hint">See which sites store cookies in this profile and delete them</span>
        </div>
        <button type="button" className="settings-button" onClick={() => void load()}>
          <Cookie size={13} aria-hidden="true" />
          {list.state === 'idle' ? 'See all' : 'Refresh'}
        </button>
      </div>

      {list.state === 'loading' && <p className="settings-hint cookie-status">Reading cookies…</p>}
      {list.state === 'error' && <p className="settings-hint cookie-status">{list.text}</p>}
      {list.state === 'ready' &&
        (list.sites.length === 0 ? (
          <p className="settings-hint cookie-status">No cookies stored.</p>
        ) : (
          <>
            <ul className="cookie-site-list">
              {list.sites.map(({ site, count }) => (
                <li key={site} className="cookie-site">
                  <span className="cookie-site-name">{site}</span>
                  <span className="cookie-site-count">
                    {count} {count === 1 ? 'cookie' : 'cookies'}
                  </span>
                  <button
                    type="button"
                    className="cookie-site-delete"
                    aria-label={`Delete cookies for ${site}`}
                    title="Delete"
                    onClick={() => void remove(site)}
                  >
                    <Trash2 size={13} />
                  </button>
                </li>
              ))}
            </ul>
            <div className="cookie-actions">
              <button
                type="button"
                className={`settings-button${confirmAll ? ' settings-button-danger' : ''}`}
                onClick={() => (confirmAll ? void remove() : setConfirmAll(true))}
                onBlur={() => setConfirmAll(false)}
              >
                <Trash2 size={13} aria-hidden="true" />
                {confirmAll ? 'Click again to delete all' : 'Delete all cookies'}
              </button>
            </div>
          </>
        ))}
    </div>
  );
};
