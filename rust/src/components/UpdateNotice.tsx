import React, { useCallback, useEffect, useState } from 'react';
import { Download, X } from 'lucide-react';
import { tauriBridge, type UpdateStatus } from '../adapter/tauriBridge';
import { useElementHole } from '../hooks/useViewportOverlay';
import { usePresence } from '../hooks/usePresence';

/** "Remind me later" hides the reminder this long. */
const SNOOZE_MS = 3 * 60 * 60 * 1000;
const SNOOZE_KEY = 'serica_update_snooze';

interface Snooze {
  version: string;
  until: number;
}

function readSnooze(): Snooze | null {
  try {
    const v = JSON.parse(localStorage.getItem(SNOOZE_KEY) || 'null');
    return v && typeof v.version === 'string' && typeof v.until === 'number' ? v : null;
  } catch {
    return null;
  }
}

/** The first few lines of the release notes, without Markdown marks. */
function summary(notes: string | null | undefined): string[] {
  if (!notes) return [];
  return notes
    .split(/\r?\n/)
    .filter((l) => !/^\s*#/.test(l)) // headings repeat the title
    .map((l) => l.replace(/^\s*[-*]\s+/, '').replace(/\*\*|__|`/g, '').trim())
    .filter(Boolean)
    .slice(0, 5);
}

/**
 * Reminds to update once a new version has been downloaded in the background
 * (checked shortly after launch and every two hours). "Remind me later"
 * hides it for a few hours; it comes back the next time the window is used
 * after that, until Serica is restarted into the new version. The update is
 * also offered in Settings → About.
 */
export const UpdateNotice: React.FC = () => {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [snooze, setSnooze] = useState<Snooze | null>(readSnooze);
  const [now, setNow] = useState(() => Date.now());
  const [el, setEl] = useState<HTMLDivElement | null>(null);

  useEffect(() => {
    void tauriBridge.getUpdateStatus().then(setStatus).catch(() => {});
    return tauriBridge.onUpdateStatus(setStatus);
  }, []);

  const version = status?.newVersion ?? '';
  const ready = !!status && (status.state === 'ready' || status.state === 'installing');
  const snoozed = !!snooze && snooze.version === version && snooze.until > now;

  // While snoozed, look again when the window is used (and once a minute).
  useEffect(() => {
    if (!ready || !snoozed) return;
    const tick = () => setNow(Date.now());
    window.addEventListener('focus', tick);
    const t = window.setInterval(tick, 60_000);
    return () => {
      window.removeEventListener('focus', tick);
      window.clearInterval(t);
    };
  }, [ready, snoozed]);

  const later = useCallback(() => {
    const s = { version, until: Date.now() + SNOOZE_MS };
    try {
      localStorage.setItem(SNOOZE_KEY, JSON.stringify(s));
    } catch {}
    setSnooze(s);
    setNow(Date.now());
  }, [version]);

  const visible = ready && (!snoozed || status?.state === 'installing');
  const presence = usePresence(visible, 180);
  // Shows through the live page (tab views paint above the HTML UI).
  useElementHole('update-notice', presence.mounted ? el : null, 2);

  if (!presence.mounted || !status) return null;
  const installing = status.state === 'installing';
  const lines = summary(status.notes);

  return (
    <div className="presence" data-state={presence.state}>
    <div ref={setEl} className="update-notice" role="alertdialog" aria-labelledby="update-notice-title" aria-describedby="update-notice-text">
      <div className="update-notice-head">
        <span className="update-notice-icon" aria-hidden="true">
          <Download size={15} />
        </span>
        <div className="update-notice-body">
          <p className="update-notice-title" id="update-notice-title">
            Serica {version} is ready
          </p>
          <p className="update-notice-text" id="update-notice-text">
            {installing ? 'Installing… Serica will reopen when it’s done.' : 'Restart to update. Your tabs will be right where you left them.'}
          </p>
        </div>
        {!installing && (
          <button type="button" className="update-notice-close" aria-label="Remind me later" title="Remind me later" onClick={later}>
            <X size={14} />
          </button>
        )}
      </div>
      {lines.length > 0 && !installing && (
        <ul className="update-notice-notes" aria-label="What’s new">
          {lines.map((l, i) => (
            <li key={i}>{l}</li>
          ))}
        </ul>
      )}
      <div className="update-notice-actions">
        {!installing && (
          <button type="button" className="update-notice-secondary" onClick={later}>
            Remind me later
          </button>
        )}
        <button type="button" className="update-notice-primary" disabled={installing} onClick={() => void tauriBridge.installUpdate().catch(() => {})}>
          {installing ? 'Restarting…' : 'Restart now'}
        </button>
      </div>
    </div>
    </div>
  );
};
