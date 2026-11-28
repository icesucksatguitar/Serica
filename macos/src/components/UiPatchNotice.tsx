import React, { useEffect, useState } from 'react';
import { Sparkles, X } from 'lucide-react';
import { tauriBridge, type UiPatchStatus } from '../adapter/tauriBridge';
import { useElementHole } from '../hooks/useViewportOverlay';
import { usePresence } from '../hooks/usePresence';

/**
 * UI fixes ship without an installer (see src-tauri/src/ui_patch.rs). Once
 * one is downloaded this offers "Reload", which swaps it in within a second;
 * dismissed, it is used on the next launch instead. Mounting this also tells
 * the backend the UI rendered, so a broken fix is rolled back automatically.
 */
export const UiPatchNotice: React.FC = () => {
  const [status, setStatus] = useState<UiPatchStatus | null>(null);
  const [dismissed, setDismissed] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [el, setEl] = useState<HTMLDivElement | null>(null);

  useEffect(() => {
    void tauriBridge.confirmUiPatch().catch(() => {});
    void tauriBridge.getUiPatchStatus().then(setStatus).catch(() => {});
    return tauriBridge.onUiPatch(setStatus);
  }, []);

  const ready = status?.readyBuild ?? null;
  const visible = ready != null && dismissed !== ready;
  const presence = usePresence(visible, 180);
  useElementHole('ui-patch-notice', presence.mounted ? el : null, 2);
  if (!presence.mounted) return null;

  const reload = async () => {
    setBusy(true);
    try {
      await tauriBridge.applyUiPatch();
      window.location.reload();
    } catch {
      setBusy(false);
    }
  };

  return (
    <div className="presence" data-state={presence.state}>
      <div ref={setEl} className="update-notice" role="alertdialog" aria-labelledby="ui-patch-title" aria-describedby="ui-patch-text">
        <div className="update-notice-head">
          <span className="update-notice-icon" aria-hidden="true">
            <Sparkles size={15} />
          </span>
          <div className="update-notice-body">
            <p className="update-notice-title" id="ui-patch-title">
              A fix for Serica is ready
            </p>
            <p className="update-notice-text" id="ui-patch-text">
              {status?.notes || 'Reload to apply it now. Your tabs stay open.'}
            </p>
          </div>
          <button type="button" className="update-notice-close" aria-label="Apply on next launch" title="Apply on next launch" onClick={() => setDismissed(ready)}>
            <X size={14} />
          </button>
        </div>
        <div className="update-notice-actions">
          <button type="button" className="update-notice-secondary" onClick={() => setDismissed(ready)}>
            Next launch
          </button>
          <button type="button" className="update-notice-primary" disabled={busy} onClick={() => void reload()}>
            {busy ? 'Reloading…' : 'Reload now'}
          </button>
        </div>
      </div>
    </div>
  );
};
