import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
  ArrowUpRight,
  CalendarDays,
  Cloud,
  CloudDrizzle,
  CloudFog,
  CloudLightning,
  CloudRain,
  CloudSnow,
  CloudSun,
  Globe,
  LayoutGrid,
  Moon,
  StickyNote,
  Sun,
} from 'lucide-react';
import type { HomeWidgets, WidgetItem, WidgetKind, WidgetSize } from '../../shared/settings';
import type { GithubContributions, HistoryEntry, WeatherNow } from '../../types';
import { APP_WIDGETS, appIcon, appQueryUrl, appWidget } from './apps';

// ---------------------------------------------------------------------------
// Catalog: what can be added, at which sizes.

export interface WidgetMeta {
  name: string;
  description: string;
  sizes: readonly WidgetSize[];
  defaultSize: WidgetSize;
  /** Small mark shown in the gallery. */
  icon: React.ReactNode;
}

export const SIZE_LABEL: Record<WidgetSize, string> = { s: 'Small', m: 'Medium', l: 'Large', w: 'Wide' };

/** GitHub's mark (lucide has no brand icons). */
export const GithubMark: React.FC<{ size?: number }> = ({ size = 14 }) => (
  <svg width={size} height={size} viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
    <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
  </svg>
);

const BUILTIN: Record<string, WidgetMeta> = {
  github: {
    name: 'GitHub contributions',
    description: 'Your contribution calendar and totals',
    sizes: ['s', 'm', 'w'],
    defaultSize: 'w',
    icon: <GithubMark size={18} />,
  },
  weather: {
    name: 'Weather',
    description: 'Current conditions where you are',
    sizes: ['s', 'm'],
    defaultSize: 's',
    icon: <CloudSun size={18} />,
  },
  notes: {
    name: 'Notes',
    description: 'A scratchpad kept on this device',
    sizes: ['s', 'm', 'l'],
    defaultSize: 'm',
    icon: <StickyNote size={18} />,
  },
  calendar: {
    name: 'Calendar',
    description: 'Today’s date, or the whole month',
    sizes: ['s', 'l'],
    defaultSize: 's',
    icon: <CalendarDays size={18} />,
  },
  topsites: {
    name: 'Top sites',
    description: 'The sites you visit most, from your history',
    sizes: ['m', 'l'],
    defaultSize: 'm',
    icon: <LayoutGrid size={18} />,
  },
};

export function widgetMeta(kind: WidgetKind): WidgetMeta | undefined {
  const app = appWidget(kind);
  if (app) {
    return {
      name: app.name,
      description: app.description,
      sizes: ['s', 'm'],
      defaultSize: app.queryUrl ? 'm' : 's',
      icon: <img src={appIcon(app.domain)} alt="" width={18} height={18} />,
    };
  }
  return BUILTIN[kind];
}

export const BUILTIN_KINDS: WidgetKind[] = ['github', 'weather', 'notes', 'calendar', 'topsites'];
export const APP_KINDS: WidgetKind[] = APP_WIDGETS.map((a) => `app:${a.id}` as WidgetKind);

// ---------------------------------------------------------------------------
// Data: fetched by the backend, cached for the session so opening new tabs
// does not refetch (GitHub's calendar changes slowly, weather every ~15 min).

type Loaded<T> = { state: 'loading' } | { state: 'ok'; data: T } | { state: 'error'; error: string };

const cache = new Map<string, { at: number; value: Promise<unknown> }>();

function cached<T>(key: string, maxAgeMs: number, load: () => Promise<T> | undefined): Promise<T> {
  const hit = cache.get(key);
  if (hit && Date.now() - hit.at < maxAgeMs) return hit.value as Promise<T>;
  const value = Promise.resolve(load()).then((v) => {
    if (v === undefined) throw new Error('unavailable');
    return v;
  });
  // A failure is not kept: the next new tab tries again.
  value.catch(() => cache.delete(key));
  cache.set(key, { at: Date.now(), value });
  return value;
}

function useLoaded<T>(key: string | null, maxAgeMs: number, load: () => Promise<T> | undefined): Loaded<T> {
  const [state, setState] = useState<Loaded<T>>({ state: 'loading' });
  const loadRef = useRef(load);
  loadRef.current = load;
  useEffect(() => {
    if (!key) return;
    let alive = true;
    setState({ state: 'loading' });
    cached(key, maxAgeMs, () => loadRef.current()).then(
      (data) => alive && setState({ state: 'ok', data }),
      (e) => alive && setState({ state: 'error', error: typeof e === 'string' ? e : (e?.message ?? 'offline') }),
    );
    return () => {
      alive = false;
    };
  }, [key, maxAgeMs]);
  return state;
}

// ---------------------------------------------------------------------------
// Shared pieces

/** In-widget setup (username, place): the widget asks for what it needs. */
const InlineSetup: React.FC<{
  label: string;
  placeholder: string;
  initial?: string;
  error?: string | null;
  busy?: boolean;
  onSubmit: (value: string) => void;
}> = ({ label, placeholder, initial = '', error, busy, onSubmit }) => {
  const [value, setValue] = useState(initial);
  return (
    <form
      className="w-setup"
      onSubmit={(e) => {
        e.preventDefault();
        if (value.trim()) onSubmit(value.trim());
      }}
    >
      <label className="w-setup-label">{label}</label>
      <div className="w-setup-row">
        <input
          className="w-input"
          value={value}
          placeholder={placeholder}
          disabled={busy}
          spellCheck={false}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => e.stopPropagation()}
        />
        <button type="submit" className="btn btn-primary" disabled={busy || !value.trim()}>
          Set
        </button>
      </div>
      {error && <span className="w-error">{error}</span>}
    </form>
  );
};

export interface WidgetContext {
  widgets: HomeWidgets;
  update: (next: HomeWidgets) => void;
  open: (url: string) => void;
  editing: boolean;
}

// ---------------------------------------------------------------------------
// GitHub

const GH_ERRORS: Record<string, string> = {
  'not-found': 'No GitHub user by that name.',
  'invalid-user': 'That is not a GitHub username.',
  offline: 'Could not reach GitHub.',
  unreadable: 'GitHub sent a page Serica could not read.',
};

function streak(days: GithubContributions['days']): number {
  let n = 0;
  for (let i = days.length - 1; i >= 0; i--) {
    if (days[i].count > 0) n++;
    else if (i === days.length - 1) continue; // today may still be empty
    else break;
  }
  return n;
}

const GithubWidget: React.FC<{ size: WidgetSize; ctx: WidgetContext }> = ({ size, ctx }) => {
  const user = ctx.widgets.github.user;
  const key = user ? `github:${user.toLowerCase()}` : null;
  const res = useLoaded<GithubContributions>(key, 30 * 60_000, () => window.serica?.widgetGithub?.(user));
  const weeksShown = size === 'w' ? 53 : size === 'm' ? 26 : 0;

  const weeks = useMemo(() => {
    if (res.state !== 'ok') return [];
    const out: (GithubContributions['days'][number] | null)[][] = [];
    let week: (GithubContributions['days'][number] | null)[] = [];
    const first = res.data.days[0];
    if (first) for (let i = 0; i < new Date(`${first.date}T00:00:00`).getDay(); i++) week.push(null);
    for (const d of res.data.days) {
      week.push(d);
      if (week.length === 7) {
        out.push(week);
        week = [];
      }
    }
    if (week.length) out.push(week);
    return out.slice(-weeksShown);
  }, [res, weeksShown]);

  if (!user) {
    return (
      <div className="w-body">
        <div className="w-head">
          <GithubMark size={14} />
          <span className="w-title">GitHub</span>
        </div>
        <InlineSetup
          label="Your GitHub username"
          placeholder="octocat"
          onSubmit={(u) => ctx.update({ ...ctx.widgets, github: { user: u.replace(/^@/, '') } })}
        />
      </div>
    );
  }

  const headInner = (
    <>
      <GithubMark size={14} />
      <button type="button" className="w-title link" onClick={() => ctx.open(`https://github.com/${user}`)}>
        {user}
      </button>
    </>
  );
  const head = <div className="w-head">{headInner}</div>;
  if (res.state === 'loading') return <div className="w-body">{head}<div className="w-skeleton" /></div>;
  if (res.state === 'error')
    return (
      <div className="w-body">
        {head}
        <p className="w-muted">{GH_ERRORS[res.error] ?? 'Could not load contributions.'}</p>
        <button type="button" className="btn btn-quiet w-inline-btn" onClick={() => ctx.update({ ...ctx.widgets, github: { user: '' } })}>
          Change user
        </button>
      </div>
    );

  const total = res.data.total;
  if (size === 's') {
    return (
      <div className="w-body">
        {head}
        <div className="w-big">{total.toLocaleString()}</div>
        <div className="w-muted">contributions this year</div>
        <div className="w-foot">{streak(res.data.days)} day streak</div>
      </div>
    );
  }
  const CELL = size === 'w' ? 9 : 8;
  const GAP = 3;
  return (
    <div className="w-body">
      <div className="w-head">
        {headInner}
        <span className="w-meta">{total.toLocaleString()} this year</span>
      </div>
      <svg
        className="gh-grid"
        viewBox={`0 0 ${weeks.length * (CELL + GAP) - GAP} ${7 * (CELL + GAP) - GAP}`}
        preserveAspectRatio="xMidYMid meet"
        role="img"
        aria-label={`${total} contributions in the last year`}
      >
        {weeks.map((week, w) =>
          week.map((d, i) =>
            d ? (
              <rect key={d.date} x={w * (CELL + GAP)} y={i * (CELL + GAP)} width={CELL} height={CELL} rx={2} className={`gh-l${d.level}`}>
                <title>{`${d.count === 0 ? 'No' : d.count} contribution${d.count === 1 ? '' : 's'} on ${new Date(`${d.date}T00:00:00`).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })}`}</title>
              </rect>
            ) : null,
          ),
        )}
      </svg>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Weather (Open-Meteo)

function describeWeather(code: number, isDay: boolean): { label: string; Icon: typeof Sun } {
  if (code === 0) return { label: 'Clear', Icon: isDay ? Sun : Moon };
  if (code <= 2) return { label: 'Partly cloudy', Icon: CloudSun };
  if (code === 3) return { label: 'Overcast', Icon: Cloud };
  if (code === 45 || code === 48) return { label: 'Fog', Icon: CloudFog };
  if (code >= 51 && code <= 57) return { label: 'Drizzle', Icon: CloudDrizzle };
  if ((code >= 61 && code <= 67) || (code >= 80 && code <= 82)) return { label: 'Rain', Icon: CloudRain };
  if ((code >= 71 && code <= 77) || code === 85 || code === 86) return { label: 'Snow', Icon: CloudSnow };
  if (code >= 95) return { label: 'Thunderstorm', Icon: CloudLightning };
  return { label: 'Cloudy', Icon: Cloud };
}

const WeatherWidget: React.FC<{ size: WidgetSize; ctx: WidgetContext }> = ({ size, ctx }) => {
  const { lat, lon, fahrenheit, place } = ctx.widgets.weather;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const key = lat !== null && lon !== null ? `weather:${lat.toFixed(2)},${lon.toFixed(2)},${fahrenheit}` : null;
  const res = useLoaded<WeatherNow>(key, 15 * 60_000, () =>
    lat !== null && lon !== null ? window.serica?.widgetWeather?.(lat, lon, fahrenheit) : undefined,
  );
  if (!key) {
    return (
      <div className="w-body">
        <div className="w-head">
          <CloudSun size={14} />
          <span className="w-title">Weather</span>
        </div>
        <InlineSetup
          label="City"
          placeholder="London"
          busy={busy}
          error={error}
          onSubmit={async (q) => {
            setBusy(true);
            setError(null);
            try {
              const found = await window.serica?.widgetGeocode?.(q);
              if (!found) throw new Error();
              ctx.update({ ...ctx.widgets, weather: { ...ctx.widgets.weather, place: found.name, lat: found.lat, lon: found.lon } });
            } catch {
              setError('No place found by that name.');
            } finally {
              setBusy(false);
            }
          }}
        />
      </div>
    );
  }
  if (res.state !== 'ok') {
    return (
      <div className="w-body">
        <div className="w-head">
          <CloudSun size={14} />
          <span className="w-title">{place}</span>
        </div>
        {res.state === 'error' ? <p className="w-muted">Could not load the weather.</p> : <div className="w-skeleton" />}
      </div>
    );
  }
  const { label, Icon } = describeWeather(res.data.code, res.data.isDay);
  const range = res.data.max !== null && res.data.min !== null ? `H ${Math.round(res.data.max)}°  L ${Math.round(res.data.min)}°` : '';
  return (
    <div className={`w-body weather ${size}`}>
      <div className="w-head">
        <span className="w-title">{place}</span>
        <button
          type="button"
          className="w-unit"
          title="Switch units"
          onClick={() => ctx.update({ ...ctx.widgets, weather: { ...ctx.widgets.weather, fahrenheit: !fahrenheit } })}
        >
          °{fahrenheit ? 'F' : 'C'}
        </button>
      </div>
      <div className="weather-main">
        <span className="w-big">{Math.round(res.data.temperature)}°</span>
        <Icon size={size === 's' ? 22 : 30} strokeWidth={1.5} aria-hidden="true" />
      </div>
      <div className="w-foot">
        <span>{label}</span>
        {size !== 's' && range && <span className="w-muted">{range}</span>}
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Notes

const NotesWidget: React.FC<{ id: string; ctx: WidgetContext }> = ({ id, ctx }) => {
  const text = ctx.widgets.notes[id] ?? '';
  const [draft, setDraft] = useState(text);
  const timer = useRef<number | null>(null);
  const editing = useRef(false);
  const ctxRef = useRef(ctx);
  ctxRef.current = ctx;
  useEffect(() => {
    if (!editing.current) setDraft(text);
  }, [text]);
  const save = (next: string) => {
    const c = ctxRef.current;
    c.update({ ...c.widgets, notes: { ...c.widgets.notes, [id]: next } });
  };
  useEffect(
    () => () => {
      if (timer.current) window.clearTimeout(timer.current);
    },
    [],
  );
  return (
    <div className="w-body">
      <div className="w-head">
        <StickyNote size={14} />
        <span className="w-title">Notes</span>
      </div>
      <textarea
        className="w-notes"
        value={draft}
        placeholder="Write something down…"
        maxLength={4000}
        spellCheck
        disabled={ctx.editing}
        onFocus={() => (editing.current = true)}
        onBlur={() => {
          editing.current = false;
          if (timer.current) window.clearTimeout(timer.current);
          if (draft !== text) save(draft);
        }}
        onChange={(e) => {
          setDraft(e.target.value);
          if (timer.current) window.clearTimeout(timer.current);
          const v = e.target.value;
          timer.current = window.setTimeout(() => save(v), 500);
        }}
        onKeyDown={(e) => e.stopPropagation()}
      />
    </div>
  );
};

// ---------------------------------------------------------------------------
// Calendar (local)

function useToday(): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const ms = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1).getTime() - Date.now() + 500;
    const t = window.setTimeout(() => setNow(new Date()), ms);
    return () => window.clearTimeout(t);
  }, [now]);
  return now;
}

const CalendarWidget: React.FC<{ size: WidgetSize }> = ({ size }) => {
  const today = useToday();
  if (size === 's') {
    return (
      <div className="w-body cal-small">
        <span className="cal-weekday">{today.toLocaleDateString(undefined, { weekday: 'long' })}</span>
        <span className="w-big cal-day">{today.getDate()}</span>
        <span className="w-muted">{today.toLocaleDateString(undefined, { month: 'long', year: 'numeric' })}</span>
      </div>
    );
  }
  const first = new Date(today.getFullYear(), today.getMonth(), 1);
  const daysIn = new Date(today.getFullYear(), today.getMonth() + 1, 0).getDate();
  const cells: (number | null)[] = [];
  for (let i = 0; i < first.getDay(); i++) cells.push(null);
  for (let d = 1; d <= daysIn; d++) cells.push(d);
  const names = Array.from({ length: 7 }, (_, i) => new Date(2023, 0, 1 + i).toLocaleDateString(undefined, { weekday: 'narrow' }));
  return (
    <div className="w-body">
      <div className="w-head">
        <span className="w-title">{today.toLocaleDateString(undefined, { month: 'long', year: 'numeric' })}</span>
      </div>
      <div className="cal-grid" role="grid">
        {names.map((n, i) => (
          <span key={`h${i}`} className="cal-name">
            {n}
          </span>
        ))}
        {cells.map((d, i) => (
          <span key={i} className={`cal-cell${d === today.getDate() ? ' today' : ''}`}>
            {d ?? ''}
          </span>
        ))}
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Top sites (from local history)

function topSites(entries: HistoryEntry[], n: number): { host: string; url: string; title: string }[] {
  const by = new Map<string, { host: string; url: string; title: string; count: number }>();
  for (const e of entries) {
    let host = '';
    try {
      const u = new URL(e.url);
      if (u.protocol !== 'http:' && u.protocol !== 'https:') continue;
      host = u.hostname.replace(/^www\./, '');
    } catch {
      continue;
    }
    const cur = by.get(host);
    if (cur) cur.count++;
    else by.set(host, { host, url: `https://${host}`, title: e.title || host, count: 1 });
  }
  return [...by.values()].sort((a, b) => b.count - a.count).slice(0, n);
}

const TopSitesWidget: React.FC<{ size: WidgetSize; ctx: WidgetContext }> = ({ size, ctx }) => {
  const [sites, setSites] = useState<ReturnType<typeof topSites> | null>(null);
  useEffect(() => {
    let alive = true;
    void window.serica?.getHistory?.({ limit: 1500 }).then((list) => {
      if (alive) setSites(topSites(Array.isArray(list) ? list : [], 12));
    });
    return () => {
      alive = false;
    };
  }, []);
  const shown = (sites ?? []).slice(0, size === 'l' ? 12 : 4);
  return (
    <div className="w-body">
      <div className="w-head">
        <LayoutGrid size={14} />
        <span className="w-title">Top sites</span>
      </div>
      {sites && sites.length === 0 ? (
        <p className="w-muted">Sites you visit often show up here.</p>
      ) : (
        <div className={`topsites ${size}`}>
          {shown.map((s) => (
            <button key={s.host} type="button" className="topsite" onClick={() => ctx.open(s.url)} title={s.title} disabled={ctx.editing}>
              <span className="topsite-icon">
                <img src={appIcon(s.host)} alt="" width={20} height={20} onError={(e) => ((e.target as HTMLElement).style.display = 'none')} />
                <Globe size={16} aria-hidden="true" />
              </span>
              <span className="topsite-name">{s.host.split('.')[0]}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
};

// ---------------------------------------------------------------------------
// App widgets

const AppWidget: React.FC<{ kind: WidgetKind; size: WidgetSize; ctx: WidgetContext }> = ({ kind, size, ctx }) => {
  const def = appWidget(kind)!;
  const [q, setQ] = useState('');
  const logo = <img className="app-logo" src={appIcon(def.domain)} alt="" width={size === 's' ? 28 : 18} height={size === 's' ? 28 : 18} />;
  if (size === 's') {
    return (
      <button
        type="button"
        className="w-body app-small"
        style={{ '--app-color': def.color } as React.CSSProperties}
        onClick={() => ctx.open(def.actions[0]?.url ?? def.home)}
        disabled={ctx.editing}
      >
        {logo}
        <span className="w-title">{def.name}</span>
        <span className="w-muted">{def.actions[0]?.label ?? 'Open'}</span>
      </button>
    );
  }
  return (
    <div className="w-body app-medium" style={{ '--app-color': def.color } as React.CSSProperties}>
      <div className="w-head">
        {logo}
        <button type="button" className="w-title link" onClick={() => ctx.open(def.home)} disabled={ctx.editing}>
          {def.name}
        </button>
        <ArrowUpRight size={13} className="w-meta-icon" aria-hidden="true" />
      </div>
      {def.queryUrl ? (
        <form
          className="app-ask"
          onSubmit={(e) => {
            e.preventDefault();
            if (!q.trim()) return;
            ctx.open(appQueryUrl(def, q.trim()));
            setQ('');
          }}
        >
          <input
            className="w-input"
            value={q}
            placeholder={def.placeholder}
            disabled={ctx.editing}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => e.stopPropagation()}
          />
        </form>
      ) : null}
      <div className="app-actions">
        {def.actions.map((a) => (
          <button key={a.label} type="button" className="app-chip" onClick={() => ctx.open(a.url)} disabled={ctx.editing}>
            {a.label}
          </button>
        ))}
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------

export const WidgetView: React.FC<{ item: WidgetItem; ctx: WidgetContext }> = ({ item, ctx }) => {
  switch (item.kind) {
    case 'github':
      return <GithubWidget size={item.size} ctx={ctx} />;
    case 'weather':
      return <WeatherWidget size={item.size} ctx={ctx} />;
    case 'notes':
      return <NotesWidget id={item.id} ctx={ctx} />;
    case 'calendar':
      return <CalendarWidget size={item.size} />;
    case 'topsites':
      return <TopSitesWidget size={item.size} ctx={ctx} />;
    default:
      return appWidget(item.kind) ? <AppWidget kind={item.kind} size={item.size} ctx={ctx} /> : null;
  }
};
