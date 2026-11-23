import React, { useEffect, useState } from 'react';
import { RefreshCw, X } from 'lucide-react';
import { tauriBridge, type UpdateStatus } from '../adapter/tauriBridge';
import { useElementHole } from '../hooks/useViewportOverlay';

/**
 * Appears once a new version has been downloaded in the background and offers
 * to restart into it. "Later" hides it until the next launch; the update stays
 * downloaded and is also offered in Settings → About.
 */
export const UpdateNotice: React.FC = () => {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const [el, setEl] = useState<HTMLDivElement | null>(null);

  useEffect(() => {
    void tauriBridge.getUpdateStatus().then(setStatus).catch(() => {});
    return tauriBridge.onUpdateStatus(setStatus);
  }, []);

  const version = status?.newVersion ?? '';
  const visible =
    !!status && (status.state === 'ready' || status.state === 'installing') && dismissed !== version;
  // Shows through the live page (tab views paint above the HTML UI).
  useElementHole('update-notice', visible ? el : null);

  if (!visible) return null;
  const installing = status.state === 'installing';

  return (
    <div ref={setEl} className="update-notice" role="status" aria-live="polite">
      <RefreshCw size={16} className="update-notice-icon" aria-hidden="true" />
      <div className="update-notice-body">
        <p className="update-notice-title">Serica {version} is ready</p>
        <p className="update-notice-text">
          {installing ? 'Installing… Serica will reopen when it’s done.' : 'Restart to update. Your tabs will be right where you left them.'}
        </p>
      </div>
      <div className="update-notice-actions">
        <button
          type="button"
          className="update-notice-primary"
          disabled={installing}
          onClick={() => void tauriBridge.installUpdate().catch(() => {})}
        >
          {installing ? 'Restarting…' : 'Restart'}
        </button>
        {!installing && (
          <button
            type="button"
            className="update-notice-close"
            aria-label="Later"
            title="Later"
            onClick={() => setDismissed(version)}
          >
            <X size={14} />
          </button>
        )}
      </div>
    </div>
  );
};
