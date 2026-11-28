import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
  ArrowUpRight,
  CalendarDays,
  Clock3,
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
  Target,
} from 'lucide-react';
import type { WidgetItem, WidgetKind, WidgetSize } from '../../shared/settings';
import type { GithubContributions, HistoryEntry, WeatherNow } from '../../types';
import { APP_WIDGETS, appIcon, appQueryUrl, appWidget } from './apps';
import type { FocusStats } from '../../adapter/tauriBridge';
import { fmtDuration, focusError, formatRemaining, remainingNow, useFocusView, useTicker } from '../../hooks/useFocus';

// ---------------------------------------------------------------------------
// Catalog

export interface WidgetMeta {
  name: string;
  description: string;
  sizes: readonly WidgetSize[];
  defaultSize: WidgetSize;
  defaultConfig: () => Record<string, unknown>;
  /** Mark shown in the gallery and the editor. */
  icon: React.ReactNode;
  /** Has settings of its own (besides size). */
  configurable: boolean;
}

export const SIZE_LABEL: Record<WidgetSize, string> = { s: 'Small', m: 'Medium', l: 'Large', w: 'Wide' };

/** GitHub's mark (lucide has no brand icons). */
export const GithubMark: React.FC<{ size?: number }> = ({ size = 14 }) => (
  <svg width={size} height={size} viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
    <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
  </svg>
);

const BUILTIN: Record<string, WidgetMeta> = {
  focus: {
    name: 'Focus',
    description: 'Start Focus Mode, or see how long it has left',
    sizes: ['s', 'm'],
    defaultSize: 's',
    defaultConfig: () => ({ minutes: 25, listId: '' }),
    icon: <Target size={18} />,
    configurable: true,
  },
  weather: {
    name: 'Weather',
    description: 'Conditions in a city you choose',
    sizes: ['s', 'm'],
    defaultSize: 's',
    defaultConfig: () => ({ place: '', lat: null, lon: null, fahrenheit: false }),
    icon: <CloudSun size={18} />,
    configurable: true,
  },
  worldclock: {
    name: 'World clock',
    description: 'The time in another city',
    sizes: ['s', 'm'],
    defaultSize: 's',
    defaultConfig: () => ({ zone: 'Europe/London', label: 'London' }),
    icon: <Clock3 size={18} />,
    configurable: true,
  },
  calendar: {
    name: 'Calendar',
    description: 'Today’s date, or the whole month',
    sizes: ['s', 'l'],
    defaultSize: 's',
    defaultConfig: () => ({ weekStart: 1 }),
    icon: <CalendarDays size={18} />,
    configurable: true,
  },
  notes: {
    name: 'Notes',
    description: 'A scratchpad kept on this device',
    sizes: ['s', 'm', 'l'],
    defaultSize: 'm',
    defaultConfig: () => ({ tint: 'plain' }),
    icon: <StickyNote size={18} />,
    configurable: true,
  },
  topsites: {
    name: 'Top sites',
    description: 'The sites you visit most, from your history',
    sizes: ['m', 'l'],
    defaultSize: 'm',
    defaultConfig: () => ({ count: 8 }),
    icon: <LayoutGrid size={18} />,
    configurable: true,
  },
  github: {
    name: 'GitHub contributions',
    description: 'A contribution calendar and totals',
    sizes: ['s', 'm', 'w'],
    defaultSize: 'w',
    defaultConfig: () => ({ user: '' }),
    icon: <GithubMark size={18} />,
    configurable: true,
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
      defaultConfig: () => ({}),
      icon: <img src={appIcon(app.domain)} alt="" width={18} height={18} />,
      configurable: false,
    };
  }
  return BUILTIN[kind];
}

export const BUILTIN_KINDS: WidgetKind[] = ['focus', 'weather', 'worldclock', 'calendar', 'notes', 'topsites', 'github'];
export const APP_KINDS: WidgetKind[] = APP_WIDGETS.map((a) => `app:${a.id}` as WidgetKind);

// ---------------------------------------------------------------------------
// Data: fetched by the backend, cached for the session.

type Loaded<T> = { state: 'loading' } | { state: 'ok'; data: T } | { state: 'error'; error: string };

const cache = new Map<string, { at: number; value: Promise<unknown> }>();

function cached<T>(key: string, maxAgeMs: number, load: () => Promise<T> | undefined): Promise<T> {
  const hit = cache.get(key);
  if (hit && Date.now() - hit.at < maxAgeMs) return hit.value as Promise<T>;
  const value = Promise.resolve(load()).then((v) => {
    if (v === undefined) throw new Error('unavailable');
    return v;
  });
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
// Context handed to every widget

export interface WidgetContext {
  item: WidgetItem;
  /** Merges into this widget's own settings. */
  setConfig: (patch: Record<string, unknown>) => void;
  note: string;
  setNote: (text: string) => void;
  open: (url: string) => void;
  editing: boolean;
  /** Opens this widget's settings (from an empty widget's prompt). */
  edit: () => void;
}

const str = (v: unknown, d = '') => (typeof v === 'string' ? v : d);
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);

/** An empty widget that needs setting up. */
const SetupPrompt: React.FC<{ icon: React.ReactNode; title: string; text: string; ctx: WidgetContext }> = ({ icon, title, text, ctx }) => (
  <button type="button" className="w-body w-setup-prompt" onClick={ctx.edit} disabled={ctx.editing}>
    <div className="w-head">
      {icon}
      <span className="w-title">{title}</span>
    </div>
    <span className="w-muted">{text}</span>
  </button>
);

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
    else if (i === days.length - 1) continue;
    else break;
  }
  return n;
}

const GithubWidget: React.FC<{ ctx: WidgetContext }> = ({ ctx }) => {
  const size = ctx.item.size;
  const user = str(ctx.item.config.user);
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

  if (!user) return <SetupPrompt icon={<GithubMark size={14} />} title="GitHub" text="Choose whose contributions to show" ctx={ctx} />;
  const head = (
    <div className="w-head">
      <GithubMark size={14} />
      <button type="button" className="w-title link" onClick={() => ctx.open(`https://github.com/${user}`)} disabled={ctx.editing}>
        {user}
      </button>
      {res.state === 'ok' && size !== 's' && <span className="w-meta tnum">{res.data.total.toLocaleString()} this year</span>}
    </div>
  );
  if (res.state === 'loading') return <div className="w-body">{head}<div className="w-skeleton" /></div>;
  if (res.state === 'error') return <div className="w-body">{head}<p className="w-muted">{GH_ERRORS[res.error] ?? 'Could not load contributions.'}</p></div>;
  if (size === 's') {
    return (
      <div className="w-body">
        {head}
        <div className="w-big tnum">{res.data.total.toLocaleString()}</div>
        <div className="w-muted">contributions this year</div>
        <div className="w-foot tnum">{streak(res.data.days)} day streak</div>
      </div>
    );
  }
  const CELL = size === 'w' ? 9 : 8;
  const GAP = 3;
  return (
    <div className="w-body">
      {head}
      <svg className="gh-grid" viewBox={`0 0 ${weeks.length * (CELL + GAP) - GAP} ${7 * (CELL + GAP) - GAP}`} preserveAspectRatio="xMidYMid meet" role="img" aria-label={`${res.data.total} contributions in the last year`}>
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

const WeatherWidget: React.FC<{ ctx: WidgetContext }> = ({ ctx }) => {
  const c = ctx.item.config;
  const lat = num(c.lat);
  const lon = num(c.lon);
  const fahrenheit = c.fahrenheit === true;
  const place = str(c.place);
  const size = ctx.item.size;
  const key = lat !== null && lon !== null ? `weather:${lat.toFixed(2)},${lon.toFixed(2)},${fahrenheit}` : null;
  const res = useLoaded<WeatherNow>(key, 15 * 60_000, () => (lat !== null && lon !== null ? window.serica?.widgetWeather?.(lat, lon, fahrenheit) : undefined));
  if (!key) return <SetupPrompt icon={<CloudSun size={14} />} title="Weather" text="Choose a city" ctx={ctx} />;
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
      </div>
      <div className="weather-main">
        <span className="w-big tnum">{Math.round(res.data.temperature)}°</span>
        <Icon size={size === 's' ? 22 : 30} strokeWidth={1.5} aria-hidden="true" />
      </div>
      <div className="w-foot">
        <span>{label}</span>
        {size !== 's' && range && <span className="w-muted tnum">{range}</span>}
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Notes

export const NOTE_TINTS: { id: string; label: string; color: string }[] = [
  { id: 'plain', label: 'Plain', color: 'transparent' },
  { id: 'butter', label: 'Butter', color: '#F2D675' },
  { id: 'rose', label: 'Rose', color: '#EBA3B0' },
  { id: 'sage', label: 'Sage', color: '#A8C7A0' },
  { id: 'sky', label: 'Sky', color: '#9EC3E6' },
];

const NotesWidget: React.FC<{ ctx: WidgetContext }> = ({ ctx }) => {
  const text = ctx.note;
  const [draft, setDraft] = useState(text);
  const timer = useRef<number | null>(null);
  const focused = useRef(false);
  const ctxRef = useRef(ctx);
  ctxRef.current = ctx;
  useEffect(() => {
    if (!focused.current) setDraft(text);
  }, [text]);
  useEffect(() => () => {
    if (timer.current) window.clearTimeout(timer.current);
  }, []);
  const tint = NOTE_TINTS.find((t) => t.id === ctx.item.config.tint) ?? NOTE_TINTS[0];
  return (
    <div className={`w-body notes tint-${tint.id}`} style={{ '--note-tint': tint.color } as React.CSSProperties}>
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
        onFocus={() => (focused.current = true)}
        onBlur={() => {
          focused.current = false;
          if (timer.current) window.clearTimeout(timer.current);
          if (draft !== text) ctxRef.current.setNote(draft);
        }}
        onChange={(e) => {
          setDraft(e.target.value);
          if (timer.current) window.clearTimeout(timer.current);
          const v = e.target.value;
          timer.current = window.setTimeout(() => ctxRef.current.setNote(v), 500);
        }}
        onKeyDown={(e) => e.stopPropagation()}
      />
    </div>
  );
};

// ---------------------------------------------------------------------------
// Calendar

function useToday(): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const ms = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1).getTime() - Date.now() + 500;
    const t = window.setTimeout(() => setNow(new Date()), ms);
    return () => window.clearTimeout(t);
  }, [now]);
  return now;
}

const CalendarWidget: React.FC<{ ctx: WidgetContext }> = ({ ctx }) => {
  const today = useToday();
  const weekStart = ctx.item.config.weekStart === 0 ? 0 : 1;
  if (ctx.item.size === 's') {
    return (
      <div className="w-body cal-small">
        <span className="cal-weekday">{today.toLocaleDateString(undefined, { weekday: 'long' })}</span>
        <span className="w-big cal-day tnum">{today.getDate()}</span>
        <span className="w-muted">{today.toLocaleDateString(undefined, { month: 'long', year: 'numeric' })}</span>
      </div>
    );
  }
  const first = new Date(today.getFullYear(), today.getMonth(), 1);
  const daysIn = new Date(today.getFullYear(), today.getMonth() + 1, 0).getDate();
  const lead = (first.getDay() - weekStart + 7) % 7;
  const cells: (number | null)[] = [];
  for (let i = 0; i < lead; i++) cells.push(null);
  for (let d = 1; d <= daysIn; d++) cells.push(d);
  const names = Array.from({ length: 7 }, (_, i) => new Date(2023, 0, 1 + ((i + weekStart) % 7)).toLocaleDateString(undefined, { weekday: 'narrow' }));
  return (
    <div className="w-body">
      <div className="w-head">
        <span className="w-title">{today.toLocaleDateString(undefined, { month: 'long', year: 'numeric' })}</span>
      </div>
      <div className="cal-grid tnum" role="grid">
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
    if (cur) cur.count += e.visitCount ?? 1;
    else by.set(host, { host, url: `https://${host}`, title: e.title || host, count: e.visitCount ?? 1 });
  }
  return [...by.values()].sort((a, b) => b.count - a.count).slice(0, n);
}

const TopSitesWidget: React.FC<{ ctx: WidgetContext }> = ({ ctx }) => {
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
  const count = ctx.item.size === 'l' ? Math.max(4, Math.min(12, num(ctx.item.config.count) ?? 8)) : 4;
  const shown = (sites ?? []).slice(0, count);
  return (
    <div className="w-body">
      <div className="w-head">
        <LayoutGrid size={14} />
        <span className="w-title">Top sites</span>
      </div>
      {sites && sites.length === 0 ? (
        <p className="w-muted">Sites you visit often show up here.</p>
      ) : (
        <div className={`topsites ${ctx.item.size}`}>
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
// World clock

export const WORLD_ZONES: { zone: string; label: string }[] = [
  { zone: 'America/Los_Angeles', label: 'Los Angeles' },
  { zone: 'America/Denver', label: 'Denver' },
  { zone: 'America/Chicago', label: 'Chicago' },
  { zone: 'America/New_York', label: 'New York' },
  { zone: 'America/Toronto', label: 'Toronto' },
  { zone: 'America/Mexico_City', label: 'Mexico City' },
  { zone: 'America/Sao_Paulo', label: 'São Paulo' },
  { zone: 'America/Buenos_Aires', label: 'Buenos Aires' },
  { zone: 'Europe/London', label: 'London' },
  { zone: 'Europe/Dublin', label: 'Dublin' },
  { zone: 'Europe/Lisbon', label: 'Lisbon' },
  { zone: 'Europe/Paris', label: 'Paris' },
  { zone: 'Europe/Berlin', label: 'Berlin' },
  { zone: 'Europe/Madrid', label: 'Madrid' },
  { zone: 'Europe/Rome', label: 'Rome' },
  { zone: 'Europe/Amsterdam', label: 'Amsterdam' },
  { zone: 'Europe/Stockholm', label: 'Stockholm' },
  { zone: 'Europe/Athens', label: 'Athens' },
  { zone: 'Europe/Istanbul', label: 'Istanbul' },
  { zone: 'Europe/Moscow', label: 'Moscow' },
  { zone: 'Africa/Cairo', label: 'Cairo' },
  { zone: 'Africa/Lagos', label: 'Lagos' },
  { zone: 'Africa/Nairobi', label: 'Nairobi' },
  { zone: 'Africa/Johannesburg', label: 'Johannesburg' },
  { zone: 'Asia/Dubai', label: 'Dubai' },
  { zone: 'Asia/Karachi', label: 'Karachi' },
  { zone: 'Asia/Kolkata', label: 'Mumbai · Delhi' },
  { zone: 'Asia/Dhaka', label: 'Dhaka' },
  { zone: 'Asia/Bangkok', label: 'Bangkok' },
  { zone: 'Asia/Jakarta', label: 'Jakarta' },
  { zone: 'Asia/Singapore', label: 'Singapore' },
  { zone: 'Asia/Hong_Kong', label: 'Hong Kong' },
  { zone: 'Asia/Shanghai', label: 'Shanghai' },
  { zone: 'Asia/Taipei', label: 'Taipei' },
  { zone: 'Asia/Seoul', label: 'Seoul' },
  { zone: 'Asia/Tokyo', label: 'Tokyo' },
  { zone: 'Australia/Perth', label: 'Perth' },
  { zone: 'Australia/Sydney', label: 'Sydney' },
  { zone: 'Pacific/Auckland', label: 'Auckland' },
  { zone: 'Pacific/Honolulu', label: 'Honolulu' },
];

function useMinuteTick(): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    let t = 0;
    const schedule = () => {
      t = window.setTimeout(() => {
        setNow(new Date());
        schedule();
      }, 60_000 - (Date.now() % 60_000) + 50);
    };
    schedule();
    return () => window.clearTimeout(t);
  }, []);
  return now;
}

const WorldClockWidget: React.FC<{ ctx: WidgetContext; hour12: boolean }> = ({ ctx, hour12 }) => {
  const now = useMinuteTick();
  const zone = str(ctx.item.config.zone, 'Europe/London');
  const label = str(ctx.item.config.label) || zone.split('/').pop()!.replace(/_/g, ' ');
  let time = '';
  let day = '';
  let offset = '';
  try {
    time = new Intl.DateTimeFormat(undefined, { timeZone: zone, hour: '2-digit', minute: '2-digit', hour12 }).format(now);
    day = new Intl.DateTimeFormat(undefined, { timeZone: zone, weekday: 'short' }).format(now);
    const there = new Date(now.toLocaleString('en-US', { timeZone: zone }));
    const here = new Date(now.toLocaleString('en-US'));
    const diff = Math.round((there.getTime() - here.getTime()) / 3_600_000 * 2) / 2;
    offset = diff === 0 ? 'Same time' : `${diff > 0 ? '+' : '−'}${Math.abs(diff)}h`;
  } catch {
    time = '--:--';
  }
  return (
    <div className={`w-body worldclock ${ctx.item.size}`}>
      <div className="w-head">
        <Clock3 size={14} />
        <span className="w-title">{label}</span>
      </div>
      <span className="w-big tnum">{time}</span>
      <div className="w-foot">
        <span>{day}</span>
        <span className="w-muted tnum">{offset}</span>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Focus

const FocusWidget: React.FC<{ ctx: WidgetContext }> = ({ ctx }) => {
  const view = useFocusView();
  const [error, setError] = useState<string | null>(null);
  const active = view?.active[0];
  useTicker(!!active && active.remainingMs !== null);
  const minutes = num(ctx.item.config.minutes) ?? 25;
  const listId = str(ctx.item.config.listId);
  const list = view?.blocks.find((b) => b.id === listId) ?? view?.blocks[0];
  // Today's focused time, refreshed when a list starts or ends.
  const [today, setToday] = useState(0);
  useEffect(() => {
    void window.serica?.focusStats?.('day').then((s: FocusStats) => s && setToday(s.totals.focusMs));
  }, [view]);
  if (active) {
    const name = view?.blocks.find((b) => b.id === active.blockId)?.name ?? 'Focus';
    const left = remainingNow(active.remainingMs);
    const total = active.totalMs ?? null;
    const p = total && left !== null ? 1 - left / total : null;
    return (
      <div className={`w-body focusw on ${ctx.item.size}`}>
        <div className="w-head">
          <Target size={14} />
          <span className="w-title">{name}</span>
          {active.locked && <span className="w-meta">Locked</span>}
        </div>
        <span className="w-big tnum">{formatRemaining(left)}</span>
        {p !== null && (
          <div className="focusw-bar" aria-hidden="true">
            <span style={{ transform: `scaleX(${Math.max(0, Math.min(1, p))})` }} />
          </div>
        )}
        <div className="w-foot">
          <span className="w-muted">{active.source === 'schedule' ? 'Scheduled' : 'left'}</span>
        </div>
      </div>
    );
  }
  return (
    <div className={`w-body focusw ${ctx.item.size}`}>
      <div className="w-head">
        <Target size={14} />
        <span className="w-title">Focus</span>
        {today >= 60_000 && <span className="w-meta tnum" title="Focused today">{fmtDuration(today)} today</span>}
      </div>
      {list ? (
        <>
          <span className="w-muted">{list.name}</span>
          <button
            type="button"
            className="btn btn-primary focusw-start"
            disabled={ctx.editing}
            onClick={() => {
              setError(null);
              window.serica?.focusStart?.(list.id, minutes, false).catch((e: unknown) => setError(focusError(e)));
            }}
          >
            Focus for {minutes} min
          </button>
          {error && <span className="w-error">{error}</span>}
        </>
      ) : (
        <span className="w-muted">Make a Focus list in Settings › Focus first.</span>
      )}
    </div>
  );
};

// ---------------------------------------------------------------------------
// App widgets

const AppWidget: React.FC<{ ctx: WidgetContext }> = ({ ctx }) => {
  const def = appWidget(ctx.item.kind)!;
  const size = ctx.item.size;
  const [q, setQ] = useState('');
  const logo = <img className="app-logo" src={appIcon(def.domain)} alt="" width={size === 's' ? 28 : 18} height={size === 's' ? 28 : 18} />;
  if (size === 's') {
    return (
      <button type="button" className="w-body app-small" style={{ '--app-color': def.color } as React.CSSProperties} onClick={() => ctx.open(def.actions[0]?.url ?? def.home)} disabled={ctx.editing}>
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
          <input className="w-input" value={q} placeholder={def.placeholder} disabled={ctx.editing} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.stopPropagation()} />
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

export const WidgetView: React.FC<{ ctx: WidgetContext; hour12: boolean }> = ({ ctx, hour12 }) => {
  switch (ctx.item.kind) {
    case 'github':
      return <GithubWidget ctx={ctx} />;
    case 'weather':
      return <WeatherWidget ctx={ctx} />;
    case 'notes':
      return <NotesWidget ctx={ctx} />;
    case 'calendar':
      return <CalendarWidget ctx={ctx} />;
    case 'topsites':
      return <TopSitesWidget ctx={ctx} />;
    case 'worldclock':
      return <WorldClockWidget ctx={ctx} hour12={hour12} />;
    case 'focus':
      return <FocusWidget ctx={ctx} />;
    default:
      return appWidget(ctx.item.kind) ? <AppWidget ctx={ctx} /> : null;
  }
};
