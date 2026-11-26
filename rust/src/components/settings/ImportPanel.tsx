import React, { useEffect, useState } from 'react';
import { Check, FileUp, Loader2 } from 'lucide-react';
import type { ImportReport, ImportSource } from '../../adapter/tauriBridge';
import { Switch } from '../Switch';
import { Segmented } from '../home/WidgetEditor';

const BROWSER_ICON: Record<string, string> = {
  chrome: 'google.com/chrome',
  edge: 'microsoft.com/edge',
  brave: 'brave.com',
  vivaldi: 'vivaldi.com',
  opera: 'opera.com',
  operagx: 'opera.com/gx',
  arc: 'arc.net',
  chromium: 'chromium.org',
  firefox: 'firefox.com',
  zen: 'zen-browser.app',
  librewolf: 'librewolf.net',
  waterfox: 'waterfox.net',
};

function summary(r: ImportReport): string[] {
  const out: string[] = [];
  if (r.profileName) out.push(`New profile “${r.profileName}”`);
  if (r.bookmarks) out.push(`${r.bookmarks.toLocaleString()} bookmarks${r.pinned ? `, ${r.pinned} pinned in the sidebar` : ''}`);
  if (r.history) out.push(`${r.history.toLocaleString()} history entries`);
  if (r.passwords) out.push(`${r.passwords.toLocaleString()} passwords`);
  return out;
}

/**
 * "Bring things over": pick a browser on this PC and one of its profiles,
 * then what to bring. A browser profile can come over as a Serica profile
 * of its own. Nothing in the other browser changes.
 */
export const ImportPanel: React.FC<{ compact?: boolean; onDone?: (r: ImportReport) => void }> = ({ compact, onDone }) => {
  const [sources, setSources] = useState<ImportSource[] | null>(null);
  const [sourceId, setSourceId] = useState<string>('');
  const [profileId, setProfileId] = useState<string>('');
  const [what, setWhat] = useState({ bookmarks: true, history: true, passwords: true });
  const [asNew, setAsNew] = useState(false);
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<ImportReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void window.serica?.importSources?.().then((list: ImportSource[]) => {
      setSources(list ?? []);
      if (list?.length) {
        setSourceId(list[0].id);
        setProfileId(list[0].profiles[0]?.id ?? '');
      }
    });
  }, []);

  const source = sources?.find((s) => s.id === sourceId);
  const run = async () => {
    if (!source) return;
    setBusy(true);
    setError(null);
    setReport(null);
    try {
      const r: ImportReport = await window.serica!.importFromBrowser!({ source: source.id, profile: profileId, ...what, asNewProfile: asNew });
      setReport(r);
      onDone?.(r);
    } catch (e) {
      setError(typeof e === 'string' ? (e === 'browser-not-found' || e === 'profile-not-found' ? 'That browser profile is no longer there.' : e) : 'The import didn’t finish.');
    } finally {
      setBusy(false);
    }
  };
  const importFile = async () => {
    setBusy(true);
    setError(null);
    try {
      const r: ImportReport | null = await window.serica!.importPasswordsFile!();
      if (r) {
        setReport(r);
        onDone?.(r);
      }
    } catch (e) {
      setError(e === 'no-passwords-in-file' ? 'That file has no passwords Serica can read.' : 'The passwords couldn’t be imported.');
    } finally {
      setBusy(false);
    }
  };

  if (sources === null) {
    return (
      <div className="import-panel loading">
        <Loader2 size={16} className="spin" aria-hidden="true" /> Looking for other browsers…
      </div>
    );
  }

  return (
    <div className={`import-panel${compact ? ' compact' : ''}`}>
      {sources.length === 0 ? (
        <p className="s2-note">No other browser was found on this PC. You can still import a password export file.</p>
      ) : (
        <>
          <div className="import-sources" role="radiogroup" aria-label="Browser">
            {sources.map((s) => (
              <button
                key={s.id}
                type="button"
                role="radio"
                aria-checked={s.id === sourceId}
                className={`import-source${s.id === sourceId ? ' on' : ''}`}
                onClick={() => {
                  setSourceId(s.id);
                  setProfileId(s.profiles[0]?.id ?? '');
                  setReport(null);
                }}
              >
                <img src={`https://www.google.com/s2/favicons?domain=${encodeURIComponent(BROWSER_ICON[s.id] ?? s.id)}&sz=64`} alt="" width={22} height={22} />
                <span>{s.name}</span>
                {s.id === sourceId && <Check size={13} className="import-check" aria-hidden="true" />}
              </button>
            ))}
          </div>
          {source && source.profiles.length > 1 && (
            <label className="import-row">
              <span>Profile</span>
              <select className="input select" value={profileId} onChange={(e) => setProfileId(e.target.value)}>
                {source.profiles.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </select>
            </label>
          )}
          <div className="import-what">
            {(
              [
                ['bookmarks', 'Bookmarks', 'The bookmarks bar becomes a pinned folder'],
                ['history', 'History', 'So the address bar knows where you go'],
                ['passwords', 'Passwords', 'Filled in on sign-in pages'],
              ] as const
            ).map(([k, label, hint]) => (
              <div key={k} className="import-row">
                <div className="import-row-text">
                  <span>{label}</span>
                  <span className="import-hint">{hint}</span>
                </div>
                <Switch checked={what[k]} label={label} onChange={(v) => setWhat({ ...what, [k]: v })} />
              </div>
            ))}
          </div>
          <div className="import-row">
            <div className="import-row-text">
              <span>Bring it into</span>
              <span className="import-hint">{asNew ? 'A Serica profile of its own, with separate sign-ins' : 'The profile you’re using now'}</span>
            </div>
            <Segmented label="Bring it into" value={asNew ? 'new' : 'current'} options={[{ value: 'current', label: 'This profile' }, { value: 'new', label: 'A new profile' }]} onChange={(v) => setAsNew(v === 'new')} />
          </div>
          <div className="import-actions">
            <button type="button" className="btn btn-primary" disabled={busy || !source || !(what.bookmarks || what.history || what.passwords)} onClick={run}>
              {busy ? <Loader2 size={13} className="spin" aria-hidden="true" /> : null}
              {busy ? 'Bringing it over…' : `Import from ${source?.name ?? ''}`}
            </button>
            {what.passwords && <span className="import-hint">Pages of that profile reload once while passwords are saved.</span>}
          </div>
        </>
      )}
      <div className="import-file">
        <button type="button" className="btn btn-quiet" disabled={busy} onClick={importFile}>
          <FileUp size={13} aria-hidden="true" /> Import passwords from a file…
        </button>
        <span className="import-hint">A CSV export from Chrome, Edge, Firefox, Bitwarden or 1Password</span>
      </div>
      {error && <p className="wed-error">{error}</p>}
      {report && (
        <div className="import-report" role="status">
          <Check size={14} aria-hidden="true" />
          <div>
            <strong>{summary(report).length ? 'Brought over' : 'Nothing new to bring over'}</strong>
            {summary(report).length > 0 && <span>{summary(report).join(' · ')}</span>}
            {report.passwordsLocked > 0 && (
              <span className="import-warn">
                {report.passwordsLocked} passwords are locked to {source?.name ?? 'that browser'}. Export them there (Settings › Passwords › Export) and use “Import passwords from a file”.
              </span>
            )}
            {report.primaryPassword && <span className="import-warn">{source?.name ?? 'That browser'} has a primary password. Export your passwords from it and import the file instead.</span>}
            {report.errors.length > 0 && <span className="import-warn">Some things couldn’t be read. If {source?.name ?? 'the browser'} is open, close it and try again.</span>}
          </div>
        </div>
      )}
    </div>
  );
};

export default ImportPanel;
