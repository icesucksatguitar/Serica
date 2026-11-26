import React, { useEffect, useMemo, useState } from 'react';
import { CalendarClock, Clock, Lock, Pencil, Play, Plus, ShieldCheck, Square, Target, Trash2, Unlock, X } from 'lucide-react';
import type { FocusBlock, FocusSlot, FocusView } from '../../adapter/tauriBridge';
import { focusError, formatRemaining, remainingNow, setFocusView, useFocusView, useTicker } from '../../hooks/useFocus';
import { usePresence } from '../../hooks/usePresence';
import { Segmented } from '../home/WidgetEditor';
import { Switch } from '../Switch';

const DAYS = ['S', 'M', 'T', 'W', 'T', 'F', 'S'];
const DAY_NAMES = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];

const toTime = (m: number) => `${String(Math.floor(m / 60)).padStart(2, '0')}:${String(m % 60).padStart(2, '0')}`;
const fromTime = (s: string) => {
  const [h, m] = s.split(':').map(Number);
  return Number.isFinite(h) && Number.isFinite(m) ? Math.max(0, Math.min(1439, h * 60 + m)) : 0;
};

function scheduleSummary(b: FocusBlock): string {
  if (!b.scheduleOn || b.schedule.length === 0) return 'No schedule';
  return b.schedule
    .map((s) => {
      const days = [1, 2, 3, 4, 5].every((d) => s.days & (1 << d)) && !(s.days & 1) && !(s.days & 64) ? 'Weekdays' : s.days === 127 ? 'Every day' : DAY_NAMES.filter((_, i) => s.days & (1 << i)).map((n) => n.slice(0, 3)).join(' ');
      return `${days} ${toTime(s.start)}–${toTime(s.end)}`;
    })
    .join(' · ');
}

function rulesSummary(b: FocusBlock, view: FocusView): string {
  const cats = b.categories.map((c) => view.categories.find((x) => x.id === c)?.label ?? c);
  const parts = [...cats, ...b.sites.slice(0, 3)];
  const more = b.sites.length > 3 ? ` and ${b.sites.length - 3} more` : '';
  return parts.length ? parts.join(', ') + more : 'Nothing blocked yet';
}

const EMPTY: FocusBlock = { id: '', name: 'Deep work', sites: [], allow: [], categories: ['social', 'video'], schedule: [], scheduleOn: false, scheduleLocked: false, unlock: 'strict' };

/** A list's settings. A locked list can only become stricter (the backend enforces it too). */
const BlockEditor: React.FC<{ block: FocusBlock; view: FocusView; locked: boolean; onSave: (b: FocusBlock) => Promise<string | null>; onClose: () => void }> = ({ block, view, locked, onSave, onClose }) => {
  const [b, setB] = useState<FocusBlock>(block);
  const [sitesText, setSitesText] = useState(block.sites.join('\n'));
  const [allowText, setAllowText] = useState(block.allow.join('\n'));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const lines = (t: string) => t.split(/[\n,]+/).map((s) => s.trim()).filter(Boolean);

  const save = async () => {
    setBusy(true);
    setError(null);
    const err = await onSave({ ...b, sites: lines(sitesText), allow: lines(allowText) });
    setBusy(false);
    if (err) setError(err);
    else onClose();
  };
  const setSlot = (i: number, patch: Partial<FocusSlot>) => setB({ ...b, schedule: b.schedule.map((s, k) => (k === i ? { ...s, ...patch } : s)) });

  return (
    <div className="modal-overlay wed-overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()} onKeyDown={(e) => e.key === 'Escape' && (e.stopPropagation(), onClose())}>
      <div className="sheet focus-editor" role="dialog" aria-modal="true" aria-label="Focus list">
        <header className="wed-head">
          <span className="wed-icon">
            <Target size={17} />
          </span>
          <div className="wed-titles">
            <h2 className="wed-title">{block.id ? 'Edit list' : 'New Focus list'}</h2>
            <p className="wed-sub">{locked ? 'Locked: you can block more, not less, until it ends.' : 'Sites to block, and when.'}</p>
          </div>
          <button type="button" className="btn btn-quiet btn-icon" aria-label="Close" onClick={onClose}>
            <X size={16} />
          </button>
        </header>
        <div className="wed-body">
          <div className="wed-field">
            <div className="wed-label">Name</div>
            <input className="input" value={b.name} maxLength={60} onChange={(e) => setB({ ...b, name: e.target.value })} onKeyDown={(e) => e.stopPropagation()} aria-label="List name" />
          </div>
          <div className="wed-field">
            <div className="wed-label">
              <span>Categories</span>
              <span className="wed-hint">Popular sites of each kind</span>
            </div>
            <div className="focus-chips">
              {view.categories.map((c) => {
                const on = b.categories.includes(c.id);
                const lockedOn = locked && block.categories.includes(c.id);
                return (
                  <button
                    key={c.id}
                    type="button"
                    className={`focus-chip${on ? ' on' : ''}`}
                    aria-pressed={on}
                    disabled={lockedOn}
                    title={c.sites.join(', ')}
                    onClick={() => setB({ ...b, categories: on ? b.categories.filter((x) => x !== c.id) : [...b.categories, c.id] })}
                  >
                    {c.label}
                  </button>
                );
              })}
            </div>
          </div>
          <div className="wed-field">
            <div className="wed-label">
              <span>Block these sites</span>
              <span className="wed-hint">One per line: reddit.com · *.reddit.com · youtube.com/shorts · *casino*</span>
            </div>
            <textarea className="input focus-textarea" value={sitesText} spellCheck={false} onChange={(e) => setSitesText(e.target.value)} onKeyDown={(e) => e.stopPropagation()} aria-label="Sites to block" placeholder={'reddit.com\n*.twitter.com\nyoutube.com/shorts'} />
          </div>
          <div className="wed-field">
            <div className="wed-label">
              <span>Always allow</span>
              <span className="wed-hint">Exceptions win over everything above</span>
            </div>
            <textarea className="input focus-textarea small" value={allowText} spellCheck={false} disabled={locked} onChange={(e) => setAllowText(e.target.value)} onKeyDown={(e) => e.stopPropagation()} aria-label="Always allow" placeholder="reddit.com/r/rust" />
          </div>
          <div className="wed-field">
            <div className="wed-label">
              <span>Schedule</span>
              <Switch checked={b.scheduleOn} label="Use a schedule" disabled={locked && block.scheduleOn} onChange={(scheduleOn) => setB({ ...b, scheduleOn, schedule: scheduleOn && b.schedule.length === 0 ? [{ days: 0b0111110, start: 9 * 60, end: 17 * 60 }] : b.schedule })} />
            </div>
            {b.scheduleOn && (
              <>
                {b.schedule.map((s, i) => (
                  <div key={i} className="focus-slot">
                    <div className="focus-days" role="group" aria-label="Days">
                      {DAYS.map((d, k) => (
                        <button key={k} type="button" className={s.days & (1 << k) ? 'on' : ''} aria-pressed={!!(s.days & (1 << k))} title={DAY_NAMES[k]} disabled={locked} onClick={() => setSlot(i, { days: s.days ^ (1 << k) })}>
                          {d}
                        </button>
                      ))}
                    </div>
                    <input className="input time" type="time" value={toTime(s.start)} disabled={locked} onChange={(e) => setSlot(i, { start: fromTime(e.target.value) })} aria-label="From" />
                    <span className="wed-hint">to</span>
                    <input className="input time" type="time" value={toTime(s.end)} disabled={locked} onChange={(e) => setSlot(i, { end: fromTime(e.target.value) })} aria-label="Until" />
                    {!locked && (
                      <button type="button" className="btn btn-quiet btn-icon" aria-label="Remove this time" onClick={() => setB({ ...b, schedule: b.schedule.filter((_, k) => k !== i) })}>
                        <X size={14} />
                      </button>
                    )}
                  </div>
                ))}
                {!locked && (
                  <button type="button" className="btn btn-quiet focus-add-slot" onClick={() => setB({ ...b, schedule: [...b.schedule, { days: 0b1000001, start: 10 * 60, end: 12 * 60 }] })}>
                    <Plus size={13} /> Add a time
                  </button>
                )}
                <div className="wed-row focus-toggle-row">
                  <span>Lock while the schedule is on</span>
                  <Switch checked={b.scheduleLocked} label="Lock while the schedule is on" disabled={locked && block.scheduleLocked} onChange={(scheduleLocked) => setB({ ...b, scheduleLocked })} />
                </div>
              </>
            )}
          </div>
          <div className="wed-field">
            <div className="wed-label">
              <span>When locked</span>
              <span className="wed-hint">How a locked list can end early</span>
            </div>
            <Segmented
              label="When locked"
              value={b.unlock}
              options={[
                { value: 'strict', label: 'Nothing ends it early' },
                { value: 'randomText', label: 'Type 64 random characters', disabled: locked && block.unlock === 'strict' },
              ]}
              onChange={(unlock) => setB({ ...b, unlock })}
            />
          </div>
          {error && <p className="wed-error">{error}</p>}
        </div>
        <footer className="wed-foot">
          <span className="wed-spacer" />
          <button type="button" className="btn btn-quiet" onClick={onClose}>
            Cancel
          </button>
          <button type="button" className="btn btn-primary" disabled={busy || !b.name.trim()} onClick={save}>
            Save list
          </button>
        </footer>
      </div>
    </div>
  );
};

/** Starting a session: how long, and whether to lock it. */
const StartSheet: React.FC<{ block: FocusBlock; onStart: (minutes: number | null, lock: boolean) => Promise<string | null>; onClose: () => void }> = ({ block, onStart, onClose }) => {
  const [minutes, setMinutes] = useState<number | null>(25);
  const [custom, setCustom] = useState('45');
  const [lock, setLock] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [useCustom, setUseCustom] = useState(false);
  const length = useCustom ? Math.max(1, Math.min(10080, Number(custom) || 0)) : minutes;
  return (
    <div className="modal-overlay wed-overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()} onKeyDown={(e) => e.key === 'Escape' && (e.stopPropagation(), onClose())}>
      <div className="sheet focus-start" role="dialog" aria-modal="true" aria-label={`Start ${block.name}`}>
        <header className="wed-head">
          <span className="wed-icon">
            <Play size={16} />
          </span>
          <div className="wed-titles">
            <h2 className="wed-title">Start {block.name}</h2>
            <p className="wed-sub">Every tab, profile and window follows it, after restarts too.</p>
          </div>
          <button type="button" className="btn btn-quiet btn-icon" aria-label="Close" onClick={onClose}>
            <X size={16} />
          </button>
        </header>
        <div className="wed-body">
          <div className="wed-field">
            <div className="wed-label">For</div>
            <Segmented
              label="Length"
              value={useCustom ? -1 : minutes ?? 0}
              options={[
                { value: 15, label: '15 min' },
                { value: 25, label: '25 min' },
                { value: 50, label: '50 min' },
                { value: 90, label: '90 min' },
                { value: 240, label: '4 h' },
                { value: -1, label: 'Custom' },
                { value: 0, label: 'Until I stop', disabled: lock },
              ]}
              onChange={(v) => {
                setUseCustom(v === -1);
                if (v !== -1) setMinutes(v === 0 ? null : v);
              }}
            />
            {useCustom && (
              <div className="wed-row">
                <input className="input" type="number" min={1} max={10080} value={custom} onChange={(e) => setCustom(e.target.value)} onKeyDown={(e) => e.stopPropagation()} aria-label="Minutes" style={{ maxWidth: 120 }} />
                <span className="wed-hint">minutes</span>
              </div>
            )}
          </div>
          <div className="wed-field">
            <div className="wed-label">
              <span>Lock it</span>
              <Switch checked={lock} label="Lock it" onChange={(v) => { setLock(v); if (v && minutes === null && !useCustom) setMinutes(25); }} />
            </div>
            <span className="wed-hint">
              {lock
                ? block.unlock === 'randomText'
                  ? 'It can only end early by typing 64 random characters. Changing the clock won’t end it.'
                  : 'Nothing ends it early: not stopping it, not restarting Serica, not changing the clock.'
                : 'You can stop it any time.'}
            </span>
          </div>
          {error && <p className="wed-error">{error}</p>}
        </div>
        <footer className="wed-foot">
          <span className="wed-spacer" />
          <button type="button" className="btn btn-quiet" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={useCustom && !(Number(custom) > 0)}
            onClick={async () => {
              const err = await onStart(length, lock);
              if (err) setError(err);
              else onClose();
            }}
          >
            {lock ? <Lock size={13} aria-hidden="true" /> : <Play size={13} aria-hidden="true" />}
            {lock ? 'Start locked' : 'Start'}
          </button>
        </footer>
      </div>
    </div>
  );
};

/** Ending a locked list early by typing its random text. */
const UnlockSheet: React.FC<{ block: FocusBlock; onClose: () => void }> = ({ block, onClose }) => {
  const [challenge, setChallenge] = useState<string | null>(null);
  const [typed, setTyped] = useState('');
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    window.serica?.focusChallenge?.(block.id).then(setChallenge, (e: unknown) => setError(focusError(e)));
  }, [block.id]);
  return (
    <div className="modal-overlay wed-overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()} onKeyDown={(e) => e.key === 'Escape' && (e.stopPropagation(), onClose())}>
      <div className="sheet" role="dialog" aria-modal="true" aria-label="End early">
        <header className="wed-head">
          <span className="wed-icon">
            <Unlock size={16} />
          </span>
          <div className="wed-titles">
            <h2 className="wed-title">End {block.name} early</h2>
            <p className="wed-sub">Type the text below exactly. It can’t be pasted.</p>
          </div>
          <button type="button" className="btn btn-quiet btn-icon" aria-label="Close" onClick={onClose}>
            <X size={16} />
          </button>
        </header>
        <div className="wed-body">
          {challenge && (
            <p className="focus-challenge" aria-label="Text to type" onCopy={(e) => e.preventDefault()}>
              {challenge}
            </p>
          )}
          <textarea
            className="input focus-textarea small"
            value={typed}
            spellCheck={false}
            onPaste={(e) => e.preventDefault()}
            onDrop={(e) => e.preventDefault()}
            onChange={(e) => setTyped(e.target.value)}
            onKeyDown={(e) => e.stopPropagation()}
            aria-label="Type the text"
          />
          {error && <p className="wed-error">{error}</p>}
        </div>
        <footer className="wed-foot">
          <span className="wed-hint tnum">
            {typed.length}/{challenge?.length ?? 64}
          </span>
          <span className="wed-spacer" />
          <button
            type="button"
            className="btn btn-primary"
            disabled={!challenge || typed !== challenge}
            onClick={async () => {
              try {
                const v = await window.serica?.focusStop?.(block.id, typed);
                if (v) setFocusView(v);
                onClose();
              } catch (e) {
                setError(focusError(e));
              }
            }}
          >
            End early
          </button>
        </footer>
      </div>
    </div>
  );
};

/** Settings › Focus. */
export const FocusSettings: React.FC = () => {
  const view = useFocusView();
  const [editing, setEditing] = useState<FocusBlock | null>(null);
  const [starting, setStarting] = useState<FocusBlock | null>(null);
  const [unlocking, setUnlocking] = useState<FocusBlock | null>(null);
  const [error, setError] = useState<string | null>(null);
  const anyActive = !!view?.active.length;
  useTicker(anyActive);
  const editorP = usePresence(!!editing, 150);
  const startP = usePresence(!!starting, 150);
  const unlockP = usePresence(!!unlocking, 150);
  const [lastEditing, setLastEditing] = useState<FocusBlock | null>(null);
  const [lastStarting, setLastStarting] = useState<FocusBlock | null>(null);
  const [lastUnlocking, setLastUnlocking] = useState<FocusBlock | null>(null);
  useEffect(() => void (editing && setLastEditing(editing)), [editing]);
  useEffect(() => void (starting && setLastStarting(starting)), [starting]);
  useEffect(() => void (unlocking && setLastUnlocking(unlocking)), [unlocking]);

  const activeOf = useMemo(() => new Map((view?.active ?? []).map((a) => [a.blockId, a])), [view]);
  if (!view) return <p className="s2-note">Loading…</p>;

  const run = async (p: Promise<FocusView> | undefined): Promise<string | null> => {
    try {
      const v = await p;
      if (v) setFocusView(v);
      setError(null);
      return null;
    } catch (e) {
      const msg = focusError(e);
      setError(msg);
      return msg;
    }
  };

  return (
    <>
      <div className="focus-hero">
        <div className="focus-hero-text">
          <h3>Block what distracts you, for as long as you choose.</h3>
          <p>Lists block sites in every tab, profile and window, after restarts too. A locked list can’t be stopped or loosened until its time is up, and moving the clock doesn’t end it.</p>
        </div>
        <span className={`focus-clock${view.clockVerified ? ' ok' : ''}`} title={view.clockVerified ? 'Time checked against the network' : 'Waiting to check the time against the network: time Serica is closed won’t count until then'}>
          <ShieldCheck size={13} aria-hidden="true" />
          {view.clockVerified ? 'Clock verified' : 'Clock not verified yet'}
        </span>
      </div>

      <section className="s2-group">
        <h3 className="s2-group-title">
          <Target size={12} aria-hidden="true" />
          Focus lists
        </h3>
        <div className="focus-lists" data-setting-id="focus.lists">
          {view.blocks.map((b) => {
            const a = activeOf.get(b.id);
            const left = a ? remainingNow(a.remainingMs) : null;
            return (
              <div key={b.id} className={`focus-list${a ? ' active' : ''}${a?.locked ? ' locked' : ''}`}>
                <div className="focus-list-main">
                  <div className="focus-list-title">
                    {a?.locked ? <Lock size={13} aria-hidden="true" /> : null}
                    <span>{b.name}</span>
                    {a && <span className="focus-badge tnum">{a.source === 'schedule' ? 'Scheduled · ' : ''}{formatRemaining(left)}</span>}
                  </div>
                  <div className="focus-list-detail">{rulesSummary(b, view)}</div>
                  <div className="focus-list-detail faint">
                    <CalendarClock size={11} aria-hidden="true" /> {scheduleSummary(b)}
                    {b.scheduleOn && b.scheduleLocked ? ' · locked' : ''}
                  </div>
                </div>
                <div className="focus-list-actions">
                  {a ? (
                    a.locked ? (
                      b.unlock === 'randomText' ? (
                        <button type="button" className="btn" onClick={() => setUnlocking(b)}>
                          <Unlock size={13} aria-hidden="true" /> End early…
                        </button>
                      ) : (
                        <span className="focus-locked-note">Locked until it ends</span>
                      )
                    ) : (
                      <button type="button" className="btn" onClick={() => run(window.serica?.focusStop?.(b.id))}>
                        <Square size={12} aria-hidden="true" /> Stop
                      </button>
                    )
                  ) : (
                    <button type="button" className="btn btn-primary" onClick={() => setStarting(b)}>
                      <Play size={12} aria-hidden="true" /> Start
                    </button>
                  )}
                  {a?.locked && (
                    <button type="button" className="btn btn-quiet" title="Add more time or more sites" onClick={() => setStarting(b)}>
                      <Clock size={13} aria-hidden="true" />
                    </button>
                  )}
                  <button type="button" className="btn btn-quiet btn-icon" aria-label={`Edit ${b.name}`} onClick={() => setEditing(b)}>
                    <Pencil size={14} />
                  </button>
                  <button type="button" className="btn btn-quiet btn-icon" aria-label={`Delete ${b.name}`} disabled={!!a?.locked} title={a?.locked ? 'Locked lists can’t be deleted' : undefined} onClick={() => run(window.serica?.focusDeleteBlock?.(b.id))}>
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>
            );
          })}
          <button type="button" className="focus-new" onClick={() => setEditing({ ...EMPTY })}>
            <Plus size={14} aria-hidden="true" /> New list
          </button>
        </div>
        {error && <p className="wed-error">{error}</p>}
      </section>

      {editorP.mounted && (editing ?? lastEditing) && (
        <div className="presence" data-state={editorP.state}>
          <BlockEditor
            key={(editing ?? lastEditing)!.id || 'new'}
            block={(editing ?? lastEditing)!}
            view={view}
            locked={!!activeOf.get((editing ?? lastEditing)!.id)?.locked}
            onSave={(b) => run(window.serica?.focusSaveBlock?.(b))}
            onClose={() => setEditing(null)}
          />
        </div>
      )}
      {startP.mounted && (starting ?? lastStarting) && (
        <div className="presence" data-state={startP.state}>
          <StartSheet block={(starting ?? lastStarting)!} onStart={(m, lock) => run(window.serica?.focusStart?.((starting ?? lastStarting)!.id, m, lock))} onClose={() => setStarting(null)} />
        </div>
      )}
      {unlockP.mounted && (unlocking ?? lastUnlocking) && (
        <div className="presence" data-state={unlockP.state}>
          <UnlockSheet block={(unlocking ?? lastUnlocking)!} onClose={() => setUnlocking(null)} />
        </div>
      )}
    </>
  );
};

export default FocusSettings;
