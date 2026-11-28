import React, { useEffect, useRef, useState } from 'react';
import { Trash2, X } from 'lucide-react';
import type { WidgetItem, WidgetSize } from '../../shared/settings';
import { NOTE_TINTS, SIZE_LABEL, WORLD_ZONES, widgetMeta } from './widgets';
import { useFocusView } from '../../hooks/useFocus';

/** Segmented control: one of a few choices. */
export const Segmented = <T extends string | number>({
  value,
  options,
  onChange,
  label,
  disabled,
}: {
  value: T;
  options: { value: T; label: string; disabled?: boolean; title?: string }[];
  onChange: (v: T) => void;
  label: string;
  disabled?: boolean;
}) => (
  <div className="seg" role="radiogroup" aria-label={label}>
    {options.map((o) => (
      <button
        key={String(o.value)}
        type="button"
        role="radio"
        aria-checked={o.value === value}
        className={o.value === value ? 'on' : ''}
        disabled={disabled || o.disabled}
        title={o.title}
        onClick={() => onChange(o.value)}
      >
        {o.label}
      </button>
    ))}
  </div>
);

const Field: React.FC<{ label: string; hint?: string; children: React.ReactNode }> = ({ label, hint, children }) => (
  <div className="wed-field">
    <div className="wed-label">
      <span>{label}</span>
      {hint && <span className="wed-hint">{hint}</span>}
    </div>
    {children}
  </div>
);

/** Places a city for the weather widget (Open-Meteo geocoding). */
const PlaceSearch: React.FC<{ initial: string; onPick: (p: { name: string; lat: number; lon: number }) => void }> = ({ initial, onPick }) => {
  const [q, setQ] = useState(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <form
      className="wed-row"
      onSubmit={async (e) => {
        e.preventDefault();
        if (!q.trim()) return;
        setBusy(true);
        setError(null);
        try {
          const found = await window.serica?.widgetGeocode?.(q.trim());
          if (!found) throw new Error();
          onPick({ name: found.name, lat: found.lat, lon: found.lon });
          setQ(found.name);
        } catch {
          setError('No place found by that name.');
        } finally {
          setBusy(false);
        }
      }}
    >
      <input className="input" value={q} placeholder="City" spellCheck={false} disabled={busy} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.stopPropagation()} aria-label="City" />
      <button type="submit" className="btn" disabled={busy || !q.trim()}>
        {busy ? 'Finding…' : 'Set'}
      </button>
      {error && <span className="wed-error">{error}</span>}
    </form>
  );
};

/** What a widget shows and how: its own settings. */
const KindFields: React.FC<{ item: WidgetItem; setConfig: (patch: Record<string, unknown>) => void }> = ({ item, setConfig }) => {
  const c = item.config;
  const focus = useFocusView();
  const [user, setUser] = useState(typeof c.user === 'string' ? c.user : '');
  switch (item.kind) {
    case 'weather':
      return (
        <>
          <Field label="City" hint={typeof c.place === 'string' && c.place ? `Showing ${c.place}` : undefined}>
            <PlaceSearch initial={typeof c.place === 'string' ? c.place : ''} onPick={(p) => setConfig({ place: p.name, lat: p.lat, lon: p.lon })} />
          </Field>
          <Field label="Units">
            <Segmented label="Units" value={c.fahrenheit === true ? 'f' : 'c'} options={[{ value: 'c', label: '°C' }, { value: 'f', label: '°F' }]} onChange={(v) => setConfig({ fahrenheit: v === 'f' })} />
          </Field>
        </>
      );
    case 'github':
      return (
        <Field label="GitHub username">
          <form
            className="wed-row"
            onSubmit={(e) => {
              e.preventDefault();
              setConfig({ user: user.trim().replace(/^@/, '').slice(0, 39) });
            }}
          >
            <input className="input" value={user} placeholder="octocat" spellCheck={false} onChange={(e) => setUser(e.target.value)} onKeyDown={(e) => e.stopPropagation()} aria-label="GitHub username" />
            <button type="submit" className="btn" disabled={!user.trim() || user.trim() === c.user}>
              Set
            </button>
          </form>
        </Field>
      );
    case 'notes':
      return (
        <Field label="Paper">
          <div className="wed-swatches" role="radiogroup" aria-label="Paper">
            {NOTE_TINTS.map((t) => (
              <button
                key={t.id}
                type="button"
                role="radio"
                aria-checked={(c.tint ?? 'plain') === t.id}
                className={`wed-swatch${(c.tint ?? 'plain') === t.id ? ' on' : ''}`}
                style={{ '--sw': t.color } as React.CSSProperties}
                title={t.label}
                onClick={() => setConfig({ tint: t.id })}
              >
                <span className="sr-only">{t.label}</span>
              </button>
            ))}
          </div>
        </Field>
      );
    case 'calendar':
      return (
        <Field label="Weeks start on">
          <Segmented label="Weeks start on" value={c.weekStart === 0 ? 0 : 1} options={[{ value: 1, label: 'Monday' }, { value: 0, label: 'Sunday' }]} onChange={(v) => setConfig({ weekStart: v })} />
        </Field>
      );
    case 'topsites':
      return (
        <Field label="Sites shown" hint={item.size === 'm' ? 'Medium shows 4; Large shows more' : undefined}>
          <Segmented label="Sites shown" value={typeof c.count === 'number' ? c.count : 8} disabled={item.size !== 'l'} options={[{ value: 4, label: '4' }, { value: 8, label: '8' }, { value: 12, label: '12' }]} onChange={(v) => setConfig({ count: v })} />
        </Field>
      );
    case 'worldclock':
      return (
        <Field label="City">
          <select
            className="input select"
            value={typeof c.zone === 'string' ? c.zone : 'Europe/London'}
            onChange={(e) => {
              const z = WORLD_ZONES.find((w) => w.zone === e.target.value);
              setConfig({ zone: e.target.value, label: z?.label ?? e.target.value });
            }}
            aria-label="City"
          >
            {WORLD_ZONES.map((z) => (
              <option key={z.zone} value={z.zone}>
                {z.label}
              </option>
            ))}
          </select>
        </Field>
      );
    case 'focus':
      return (
        <>
          <Field label="List">
            {focus && focus.blocks.length > 0 ? (
              <select className="input select" value={typeof c.listId === 'string' && c.listId ? c.listId : focus.blocks[0].id} onChange={(e) => setConfig({ listId: e.target.value })} aria-label="List">
                {focus.blocks.map((b) => (
                  <option key={b.id} value={b.id}>
                    {b.name}
                  </option>
                ))}
              </select>
            ) : (
              <span className="wed-hint">No lists yet: make one in Settings › Focus.</span>
            )}
          </Field>
          <Field label="Session length">
            <Segmented
              label="Session length"
              value={typeof c.minutes === 'number' ? c.minutes : 25}
              options={[15, 25, 50, 90].map((m) => ({ value: m, label: `${m} min` }))}
              onChange={(v) => setConfig({ minutes: v })}
            />
          </Field>
        </>
      );
    default:
      return null;
  }
};

/**
 * A widget's settings, in a sheet: its size (each size shows more), what it
 * shows, and removing it. Changes apply as they are made.
 */
export const WidgetEditor: React.FC<{
  item: WidgetItem;
  canResize: (size: WidgetSize) => boolean;
  onResize: (size: WidgetSize) => void;
  setConfig: (patch: Record<string, unknown>) => void;
  onRemove: () => void;
  onClose: () => void;
}> = ({ item, canResize, onResize, setConfig, onRemove, onClose }) => {
  const meta = widgetMeta(item.kind);
  const closeRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    closeRef.current?.focus({ preventScroll: true });
  }, []);
  if (!meta) return null;
  return (
    <div
      className="modal-overlay wed-overlay"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation();
          onClose();
        }
      }}
    >
      <div className="sheet wed" role="dialog" aria-modal="true" aria-label={`${meta.name} settings`}>
        <header className="wed-head">
          <span className="wed-icon">{meta.icon}</span>
          <div className="wed-titles">
            <h2 className="wed-title">{meta.name}</h2>
            <p className="wed-sub">{meta.description}</p>
          </div>
          <button ref={closeRef} type="button" className="btn btn-quiet btn-icon" aria-label="Close" onClick={onClose}>
            <X size={16} />
          </button>
        </header>
        <div className="wed-body">
          {meta.sizes.length > 1 && (
            <Field label="Size">
              <Segmented
                label="Size"
                value={item.size}
                options={meta.sizes.map((s) => ({
                  value: s,
                  label: SIZE_LABEL[s],
                  disabled: s !== item.size && !canResize(s),
                  title: s !== item.size && !canResize(s) ? 'No room for this size: move or remove a widget' : undefined,
                }))}
                onChange={onResize}
              />
            </Field>
          )}
          <KindFields item={item} setConfig={setConfig} />
        </div>
        <footer className="wed-foot">
          <button type="button" className="btn btn-quiet btn-danger-text" onClick={onRemove}>
            <Trash2 size={14} />
            Remove widget
          </button>
          <span className="wed-spacer" />
          <button type="button" className="btn btn-primary" onClick={onClose}>
            Done
          </button>
        </footer>
      </div>
    </div>
  );
};
