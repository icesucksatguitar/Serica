import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Cloud, CloudDrizzle, CloudFog, CloudLightning, CloudRain, CloudSnow, CloudSun, Moon, StickyNote, Sun } from 'lucide-react';
import type { HomeWidgets } from '../shared/settings';
import type { GithubContributions, WeatherNow } from '../types';

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

const ERRORS: Record<string, string> = {
  'not-found': 'No GitHub user by that name.',
  'invalid-user': 'That is not a GitHub username.',
  offline: 'Could not reach GitHub. Check your connection.',
  unreadable: 'GitHub sent a page Serica could not read.',
};

// ---------------------------------------------------------------------------
// GitHub contributions

/** GitHub's mark (lucide has no brand icons). */
const GithubMark: React.FC<{ size?: number }> = ({ size = 14 }) => (
  <svg width={size} height={size} viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
    <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
  </svg>
);

const WEEKDAY_LABELS = ['', 'Mon', '', 'Wed', '', 'Fri', ''];

export const GithubWidget: React.FC<{ user: string; onOpen: (url: string) => void }> = ({ user, onOpen }) => {
  const key = user ? `github:${user.toLowerCase()}` : null;
  const res = useLoaded<GithubContributions>(key, 30 * 60_000, () => window.serica?.widgetGithub?.(user));

  // Columns of weeks (Sunday first), like GitHub's own calendar.
  const weeks = useMemo(() => {
    if (res.state !== 'ok') return [];
    const out: (GithubContributions['days'][number] | null)[][] = [];
    let week: (GithubContributions['days'][number] | null)[] = [];
    const first = res.data.days[0];
    if (first) {
      const pad = new Date(`${first.date}T00:00:00`).getDay();
      for (let i = 0; i < pad; i++) week.push(null);
    }
    for (const d of res.data.days) {
      week.push(d);
      if (week.length === 7) {
        out.push(week);
        week = [];
      }
    }
    if (week.length) out.push(week);
    return out;
  }, [res]);

  const CELL = 10;
  const GAP = 3;
  return (
    <section className="home-widget home-widget-github" aria-label={`GitHub contributions of ${user}`}>
      <header className="home-widget-head">
        <GithubMark size={14} />
        <button type="button" className="home-widget-title link" onClick={() => onOpen(`https://github.com/${user}`)}>
          {user}
        </button>
        {res.state === 'ok' && (
          <span className="home-widget-meta">
            {res.data.total.toLocaleString()} contributions in the last year
          </span>
        )}
      </header>
      {res.state === 'loading' && <div className="home-widget-skeleton" style={{ height: 7 * (CELL + GAP) }} />}
      {res.state === 'error' && <p className="home-widget-empty">{ERRORS[res.error] ?? 'Could not load contributions.'}</p>}
      {res.state === 'ok' && (
        <div className="gh-calendar">
          <div className="gh-weekdays" aria-hidden="true">
            {WEEKDAY_LABELS.map((l, i) => (
              <span key={i} style={{ height: CELL, marginBottom: GAP }}>
                {l}
              </span>
            ))}
          </div>
          <svg
            className="gh-grid"
            width={weeks.length * (CELL + GAP) - GAP}
            height={7 * (CELL + GAP) - GAP}
            role="img"
            aria-label={`${res.data.total} contributions in the last year`}
          >
            {weeks.map((week, w) =>
              week.map((d, i) =>
                d ? (
                  <rect
                    key={d.date}
                    x={w * (CELL + GAP)}
                    y={i * (CELL + GAP)}
                    width={CELL}
                    height={CELL}
                    rx={2}
                    className={`gh-l${d.level}`}
                  >
                    <title>{`${d.count === 0 ? 'No' : d.count} contribution${d.count === 1 ? '' : 's'} on ${new Date(`${d.date}T00:00:00`).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })}`}</title>
                  </rect>
                ) : null,
              ),
            )}
          </svg>
        </div>
      )}
    </section>
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

export const WeatherWidget: React.FC<{ widgets: HomeWidgets['weather'] }> = ({ widgets }) => {
  const { lat, lon, fahrenheit, place } = widgets;
  const key = lat !== null && lon !== null ? `weather:${lat.toFixed(2)},${lon.toFixed(2)},${fahrenheit}` : null;
  const res = useLoaded<WeatherNow>(key, 15 * 60_000, () =>
    lat !== null && lon !== null ? window.serica?.widgetWeather?.(lat, lon, fahrenheit) : undefined,
  );
  const unit = fahrenheit ? '°F' : '°C';
  return (
    <section className="home-widget home-widget-weather" aria-label={`Weather in ${place || 'your place'}`}>
      {!key ? (
        <p className="home-widget-empty">Set a place in Customize to see the weather.</p>
      ) : res.state === 'ok' ? (
        (() => {
          const { label, Icon } = describeWeather(res.data.code, res.data.isDay);
          return (
            <>
              <div className="weather-now">
                <Icon size={22} strokeWidth={1.6} aria-hidden="true" />
                <span className="weather-temp">
                  {Math.round(res.data.temperature)}
                  <span className="weather-unit">{unit}</span>
                </span>
              </div>
              <div className="weather-detail">
                <span className="home-widget-title">{place}</span>
                <span className="home-widget-meta">
                  {label}
                  {res.data.max !== null && res.data.min !== null && ` · ${Math.round(res.data.max)}° / ${Math.round(res.data.min)}°`}
                </span>
              </div>
            </>
          );
        })()
      ) : res.state === 'error' ? (
        <p className="home-widget-empty">Could not load the weather.</p>
      ) : (
        <div className="home-widget-skeleton" style={{ height: 40 }} />
      )}
    </section>
  );
};

// ---------------------------------------------------------------------------
// Notes: a scratchpad saved with the settings (on this device only).

export const NotesWidget: React.FC<{ text: string; onChange: (text: string) => void }> = ({ text, onChange }) => {
  const [draft, setDraft] = useState(text);
  const timer = useRef<number | null>(null);
  const editing = useRef(false);
  useEffect(() => {
    if (!editing.current) setDraft(text);
  }, [text]);
  useEffect(
    () => () => {
      if (timer.current) window.clearTimeout(timer.current);
    },
    [],
  );
  const save = (next: string) => {
    if (timer.current) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => onChange(next), 500);
  };
  return (
    <section className="home-widget home-widget-notes" aria-label="Notes">
      <header className="home-widget-head">
        <StickyNote size={14} aria-hidden="true" />
        <span className="home-widget-title">Notes</span>
      </header>
      <textarea
        className="notes-text"
        value={draft}
        placeholder="Write something down…"
        maxLength={4000}
        spellCheck
        onFocus={() => (editing.current = true)}
        onBlur={() => {
          editing.current = false;
          if (timer.current) window.clearTimeout(timer.current);
          if (draft !== text) onChange(draft);
        }}
        onChange={(e) => {
          setDraft(e.target.value);
          save(e.target.value);
        }}
        onKeyDown={(e) => e.stopPropagation()}
      />
    </section>
  );
};

// ---------------------------------------------------------------------------
// Customize: which widgets show, and their settings. Used on the new-tab page
// and in Settings > Appearance.

export const WidgetSettings: React.FC<{
  widgets: HomeWidgets;
  onChange: (next: HomeWidgets) => void;
  renderSwitch: (checked: boolean, label: string, onChange: (next: boolean) => void) => React.ReactNode;
}> = ({ widgets, onChange, renderSwitch }) => {
  const [user, setUser] = useState(widgets.github.user);
  const [place, setPlace] = useState(widgets.weather.place);
  const [placeError, setPlaceError] = useState<string | null>(null);
  const [placeBusy, setPlaceBusy] = useState(false);
  useEffect(() => setUser(widgets.github.user), [widgets.github.user]);
  useEffect(() => setPlace(widgets.weather.place), [widgets.weather.place]);

  const commitUser = () => {
    const u = user.trim().replace(/^@/, '');
    if (u !== widgets.github.user) onChange({ ...widgets, github: { enabled: true, user: u } });
  };
  const commitPlace = async () => {
    const q = place.trim();
    if (!q || q === widgets.weather.place) return;
    setPlaceBusy(true);
    setPlaceError(null);
    try {
      const found = await window.serica?.widgetGeocode?.(q);
      if (!found) throw new Error('not-found');
      onChange({
        ...widgets,
        weather: { ...widgets.weather, enabled: true, place: found.name, lat: found.lat, lon: found.lon },
      });
    } catch {
      setPlaceError('No place found by that name.');
    } finally {
      setPlaceBusy(false);
    }
  };

  return (
    <div className="widget-settings">
      <div className="settings-row">
        <div className="settings-row-text">
          <span className="settings-row-label">Greeting and clock</span>
        </div>
        {renderSwitch(widgets.clock, 'Greeting and clock', (clock) => onChange({ ...widgets, clock }))}
      </div>

      <div className="settings-row">
        <div className="settings-row-text">
          <span className="settings-row-label">GitHub contributions</span>
          <span className="settings-hint">Your contribution calendar, read from your public GitHub profile</span>
        </div>
        {renderSwitch(widgets.github.enabled, 'GitHub contributions', (enabled) =>
          onChange({ ...widgets, github: { ...widgets.github, enabled } }),
        )}
      </div>
      {widgets.github.enabled && (
        <div className="settings-subrow">
          <label className="settings-sublabel" htmlFor="widget-gh-user">
            Username
          </label>
          <input
            id="widget-gh-user"
            className="settings-input"
            value={user}
            placeholder="octocat"
            maxLength={39}
            spellCheck={false}
            onChange={(e) => setUser(e.target.value)}
            onBlur={commitUser}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === 'Enter') commitUser();
            }}
          />
        </div>
      )}

      <div className="settings-row">
        <div className="settings-row-text">
          <span className="settings-row-label">Weather</span>
          <span className="settings-hint">Current conditions from Open-Meteo</span>
        </div>
        {renderSwitch(widgets.weather.enabled, 'Weather', (enabled) =>
          onChange({ ...widgets, weather: { ...widgets.weather, enabled } }),
        )}
      </div>
      {widgets.weather.enabled && (
        <div className="settings-subrow">
          <label className="settings-sublabel" htmlFor="widget-weather-place">
            Place
          </label>
          <input
            id="widget-weather-place"
            className={`settings-input${placeError ? ' invalid' : ''}`}
            value={place}
            placeholder="City"
            maxLength={80}
            disabled={placeBusy}
            onChange={(e) => {
              setPlace(e.target.value);
              setPlaceError(null);
            }}
            onBlur={() => void commitPlace()}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === 'Enter') void commitPlace();
            }}
          />
          <div className="settings-segmented" role="radiogroup" aria-label="Temperature unit">
            {([false, true] as const).map((f) => (
              <button
                key={String(f)}
                type="button"
                role="radio"
                aria-checked={widgets.weather.fahrenheit === f}
                className={`settings-segment${widgets.weather.fahrenheit === f ? ' active' : ''}`}
                onClick={() => onChange({ ...widgets, weather: { ...widgets.weather, fahrenheit: f } })}
              >
                {f ? '°F' : '°C'}
              </button>
            ))}
          </div>
          {placeError && <span className="settings-error">{placeError}</span>}
        </div>
      )}

      <div className="settings-row">
        <div className="settings-row-text">
          <span className="settings-row-label">Notes</span>
          <span className="settings-hint">A scratchpad kept on this device</span>
        </div>
        {renderSwitch(widgets.notes.enabled, 'Notes', (enabled) => onChange({ ...widgets, notes: { ...widgets.notes, enabled } }))}
      </div>
    </div>
  );
};
