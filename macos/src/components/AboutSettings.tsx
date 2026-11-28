import React, { useEffect, useState } from 'react';
import { tauriBridge, type UpdateStatus } from '../adapter/tauriBridge';

function checkedAgo(ms?: number | null): string {
  if (!ms) return '';
  const mins = Math.round((Date.now() - ms) / 60000);
  if (mins < 1) return 'Checked just now';
  if (mins < 60) return `Checked ${mins} min ago`;
  const hours = Math.round(mins / 60);
  return `Checked ${hours} hour${hours === 1 ? '' : 's'} ago`;
}

function describe(s: UpdateStatus): string {
  switch (s.state) {
    case 'checking':
      return 'Checking for updates…';
    case 'downloading':
      return s.progress != null
        ? `Downloading Serica ${s.newVersion}… ${Math.round(s.progress * 100)}%`
        : `Downloading Serica ${s.newVersion}…`;
    case 'ready':
      return `Serica ${s.newVersion} is ready. Restart to finish updating.`;
    case 'installing':
      return 'Installing… Serica will reopen when it’s done.';
    case 'up-to-date':
      return `You’re up to date. ${checkedAgo(s.lastChecked)}`.trim();
    case 'error':
      return s.error ?? 'Couldn’t check for updates.';
    default:
      return 'Serica checks for updates automatically and installs them when you restart.';
  }
}

/** Settings → About: current version and update controls. */
export const AboutSettings: React.FC<{ active: boolean }> = ({ active }) => {
  const [status, setStatus] = useState<UpdateStatus | null>(null);

  useEffect(() => {
    if (!active) return;
    void tauriBridge.getUpdateStatus().then(setStatus).catch(() => {});
    return tauriBridge.onUpdateStatus(setStatus);
  }, [active]);

  const busy = status?.state === 'checking' || status?.state === 'downloading' || status?.state === 'installing';
  const ready = status?.state === 'ready';

  return (
    <section className="settings-section" aria-label="About">
      <h3 className="settings-section-title">Updates</h3>
      <div className="settings-card">
        <div className="settings-row">
          <div className="settings-row-text">
            <span className="settings-row-label">Serica {status?.currentVersion ?? ''}</span>
            <span className="settings-hint" role="status" aria-live="polite">
              {status ? describe(status) : ' '}
            </span>
          </div>
          {ready ? (
            <button type="button" className="settings-button" onClick={() => void tauriBridge.installUpdate().catch(() => {})}>
              Restart to update
            </button>
          ) : (
            <button
              type="button"
              className="settings-button"
              disabled={busy}
              onClick={() => void tauriBridge.checkForUpdate().then(setStatus).catch(() => {})}
            >
              Check for updates
            </button>
          )}
        </div>
      </div>
    </section>
  );
};
