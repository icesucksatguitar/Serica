import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Download, Flame, Timer, Trash2, TrendingDown, TrendingUp } from 'lucide-react';
import type { FocusRange, FocusStats as Stats, FocusTotals } from '../../adapter/tauriBridge';
import { Switch } from '../Switch';
import { fmtDuration, useFocusView } from '../../hooks/useFocus';
import { appIcon } from '../home/apps';

// Focus statistics, after Cold Turkey Pro's: how long you focused, the
// sessions you kept, what you reached for while it was blocked and when,
// and where your browsing time went. Read from the backend every 15 seconds
// while shown; everything stays on this PC.

const RANGES: { id: FocusRange; label: string; prev: string }[] = [
  { id: 'day', label: 'Today', prev: 'yesterday' },
  { id: 'week', label: '7 days', prev: 'the 7 days before' },
  { id: 'month', label: '30 days', prev: 'the 30 days before' },
  { id: 'year', label: 'Year', prev: 'the year before' },
];

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

function parseDay(key: string): Date {
  const [y, m, d] = key.split('-').map(Number);
  return new Date(y, m - 1, d);
}

function hourLabel(h: number): string {
  return `${String(h).padStart(2, '0')}:00`;
}

/** Axis label under a bar, or '' to skip it (labels stay sparse). */
function tickLabel(range: FocusRange, key: string, i: number, n: number): string {
  if (range === 'day') return i % 6 === 0 ? hourLabel(i) : '';
  if (range === 'year') return MONTHS[Number(key.slice(5, 7)) - 1] ?? '';
  const d = parseDay(key);
  if (range === 'week') return WEEKDAYS[d.getDay()];
  return i % 5 === 0 || i === n - 1 ? String(d.getDate()) : '';
}

function bucketTitle(range: FocusRange, key: string): string {
  if (range === 'day') {
    const h = Number(key);
    return `${hourLabel(h)}–${hourLabel((h + 1) % 24)}`;
  }
  if (range === 'year') return `${MONTHS[Number(key.slice(5, 7)) - 1]} ${key.slice(0, 4)}`;
  return parseDay(key).toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' });
}

/** Round axis steps in minutes, so ticks read 15m, 30m, 1h, 2h... */
const STEPS = [1, 2, 5, 10, 15, 30, 60, 120, 180, 240, 360, 600, 1200, 3000, 6000, 12000];
function niceMax(maxMs: number): { top: number; step: number } {
  const maxMin = Math.max(1, maxMs / 60_000);
  const step = STEPS.find((s) => s * 3 >= maxMin) ?? STEPS[STEPS.length - 1];
  return { top: step * Math.max(1, Math.ceil(maxMin / step)) * 60_000, step: step * 60_000 };
}

/** Round count steps (1, 2, 5, 10, 20, 25, 50...) with at most three gridlines. */
function niceCount(max: number): { top: number; step: number } {
  const target = Math.max(1, max) / 3;
  const mag = 10 ** Math.floor(Math.log10(target));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * mag).find((s) => s >= target && Number.isInteger(s)) ?? 10 * mag;
  return { top: step * Math.max(1, Math.ceil(max / step)), step };
}

function delta(now: number, before: number): number | null {
  if (before <= 0) return null;
  return Math.round(((now - before) / before) * 100);
}

const Delta: React.FC<{ now: number; before: number; upIsGood: boolean; vs: string }> = ({ now, before, upIsGood, vs }) => {
  const d = delta(now, before);
  if (d === null || d === 0) return <span className="fs-delta">{before <= 0 ? `Nothing in ${vs}` : `Same as ${vs}`}</span>;
  const good = d > 0 === upIsGood;
  const Icon = d > 0 ? TrendingUp : TrendingDown;
  return (
    <span className={`fs-delta ${good ? 'good' : 'bad'}`}>
      <Icon size={12} aria-hidden="true" />
      {d > 0 ? '+' : '−'}
      {Math.abs(d)}% vs {vs}
    </span>
  );
};

/** A column chart of one measure, with a tooltip per column. */
const Columns: React.FC<{
  label: string;
  bars: { key: string; value: number; title: string; tick: string; extra?: string }[];
  format: (v: number) => string;
  axis: 'time' | 'count';
  height?: number;
}> = ({ label, bars, format, axis, height = 150 }) => {
  const max = Math.max(0, ...bars.map((b) => b.value));
  const scale = axis === 'time' ? niceMax(max) : niceCount(max);
  const ticks: number[] = [];
  for (let v = scale.step; v <= scale.top + 1e-6; v += scale.step) ticks.push(v);
  const [hover, setHover] = useState<number | null>(null);
  const plot = useRef<HTMLDivElement>(null);
  const onMove = (e: React.PointerEvent) => {
    const el = plot.current;
    if (!el || bars.length === 0) return;
    const r = el.getBoundingClientRect();
    const i = Math.floor(((e.clientX - r.left) / r.width) * bars.length);
    setHover(i >= 0 && i < bars.length ? i : null);
  };
  const h = hover !== null ? bars[hover] : null;
  return (
    <figure className="fs-chart" aria-label={label}>
      <div className="fs-plot-wrap" style={{ height }}>
        <div className="fs-yaxis" aria-hidden="true">
          {ticks.map((t) => (
            <span key={t} style={{ bottom: `${(t / scale.top) * 100}%` }}>
              {axis === 'time' ? fmtDuration(t).replace(' 00m', '') : t.toLocaleString()}
            </span>
          ))}
        </div>
        <div className="fs-plot" ref={plot} onPointerMove={onMove} onPointerLeave={() => setHover(null)}>
          {ticks.map((t) => (
            <i key={t} className="fs-grid" style={{ bottom: `${(t / scale.top) * 100}%` }} aria-hidden="true" />
          ))}
          <div className="fs-cols" style={{ gridTemplateColumns: `repeat(${bars.length}, minmax(0, 1fr))` }}>
            {bars.map((b, i) => (
              <div key={b.key} className={`fs-col${hover === i ? ' hot' : ''}`}>
                {b.value > 0 && <span className="fs-bar" style={{ height: `${Math.max(1.5, (b.value / scale.top) * 100)}%` }} />}
              </div>
            ))}
          </div>
          {h && hover !== null && (
            <div className="fs-tip" style={{ left: `${((hover + 0.5) / bars.length) * 100}%` }} role="status">
              <strong className="tnum">{format(h.value)}</strong>
              <span>{h.title}</span>
              {h.extra && <span className="fs-tip-extra">{h.extra}</span>}
            </div>
          )}
        </div>
      </div>
      <div className="fs-xaxis" style={{ gridTemplateColumns: `repeat(${bars.length}, minmax(0, 1fr))` }} aria-hidden="true">
        {bars.map((b) => (
          <span key={b.key}>{b.tick}</span>
        ))}
      </div>
    </figure>
  );
};

/** Ranked rows: a name, a thin bar for its share of the first, the value. */
const Ranked: React.FC<{
  rows: { key: string; name: string; value: number; listed?: boolean; tag?: string }[];
  format: (v: number) => string;
  empty: string;
  favicons?: boolean;
}> = ({ rows, format, empty, favicons }) => {
  if (rows.length === 0) return <p className="fs-empty-line">{empty}</p>;
  const max = Math.max(...rows.map((r) => r.value), 1);
  return (
    <ol className="fs-ranked">
      {rows.map((r) => (
        <li key={r.key} className={r.listed ? 'listed' : ''}>
          <div className="fs-ranked-line">
            {favicons && r.key !== '(other)' && <img src={appIcon(r.key)} alt="" width={14} height={14} loading="lazy" onError={(e) => ((e.currentTarget.style.visibility = 'hidden'))} />}
            <span className="fs-ranked-name">{r.name}</span>
            {r.tag && <span className="fs-tag">{r.tag}</span>}
            <span className="fs-ranked-value tnum">{format(r.value)}</span>
          </div>
          <span className="fs-track" aria-hidden="true">
            <span className="fs-fill" style={{ width: `${Math.max(1.5, (r.value / max) * 100)}%` }} />
          </span>
        </li>
      ))}
    </ol>
  );
};

const Tile: React.FC<{ label: string; value: React.ReactNode; sub?: React.ReactNode; icon?: React.ReactNode; hero?: boolean }> = ({ label, value, sub, icon, hero }) => (
  <div className={`fs-tile${hero ? ' hero' : ''}`}>
    <span className="fs-tile-label">
      {icon}
      {label}
    </span>
    <span className="fs-tile-value">{value}</span>
    {sub && <span className="fs-tile-sub">{sub}</span>}
  </div>
);

export const FocusStatsPanel: React.FC = () => {
  const [range, setRange] = useState<FocusRange>(() => {
    try {
      return (localStorage.getItem('serica_focus_stats_range') as FocusRange) || 'week';
    } catch {
      return 'week';
    }
  });
  const [stats, setStats] = useState<Stats | null>(null);
  const [table, setTable] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);
  const [exported, setExported] = useState<string | null>(null);
  const focusView = useFocusView();
  const load = useCallback(() => void window.serica?.focusStats?.(range).then((s: Stats) => s && setStats(s)), [range]);
  useEffect(() => {
    load();
    const t = window.setInterval(load, 15_000);
    return () => window.clearInterval(t);
  }, [load, focusView]);
  const pick = (r: FocusRange) => {
    setRange(r);
    try {
      localStorage.setItem('serica_focus_stats_range', r);
    } catch {
      // Not remembered; the default is fine.
    }
  };

  const vs = RANGES.find((r) => r.id === range)!.prev;
  const focusBars = useMemo(
    () =>
      (stats?.buckets ?? []).map((b, i, all) => ({
        key: b.key,
        value: b.focusMs,
        title: bucketTitle(stats!.range, b.key),
        tick: tickLabel(stats!.range, b.key, i, all.length),
        extra: b.blocked ? `${b.blocked.toLocaleString()} blocked ${b.blocked === 1 ? 'attempt' : 'attempts'}` : undefined,
      })),
    [stats],
  );
  const hourBars = useMemo(
    () =>
      (stats?.blockedHours ?? []).map((n, h) => ({
        key: String(h),
        value: n,
        title: `${hourLabel(h)}–${hourLabel((h + 1) % 24)}`,
        tick: h % 6 === 0 ? hourLabel(h) : '',
      })),
    [stats],
  );

  if (!stats) return <p className="s2-note">Loading…</p>;
  const t: FocusTotals = stats.totals;
  const p: FocusTotals = stats.previous;
  const sessionsDone = t.completed + t.stopped;
  const peakHour = stats.blockedHours.reduce((best, n, h, all) => (n > all[best] ? h : best), 0);
  const listedShare = t.browseMs > 0 ? Math.round((t.distractMs / t.browseMs) * 100) : 0;

  return (
    <div className="fs">
      <div className="fs-head">
        <div className="seg" role="radiogroup" aria-label="Period">
          {RANGES.map((r) => (
            <button key={r.id} type="button" role="radio" aria-checked={range === r.id} className={range === r.id ? 'on' : ''} onClick={() => pick(r.id)}>
              {r.label}
            </button>
          ))}
        </div>
      </div>

      <div className="fs-tiles">
        <Tile hero label="Focused" icon={<Timer size={12} aria-hidden="true" />} value={fmtDuration(t.focusMs)} sub={<Delta now={t.focusMs} before={p.focusMs} upIsGood vs={vs} />} />
        <Tile
          label="Blocked attempts"
          value={t.blocked.toLocaleString()}
          sub={t.blocked > 0 ? <>Most around {hourLabel(peakHour)}</> : <Delta now={t.blocked} before={p.blocked} upIsGood={false} vs={vs} />}
        />
        <Tile
          label="Sessions kept"
          value={
            <>
              {t.completed.toLocaleString()}
              {sessionsDone > 0 && <small> of {sessionsDone.toLocaleString()}</small>}
            </>
          }
          sub={t.stopped > 0 ? `${t.stopped.toLocaleString()} ended early` : t.started > 0 ? `${t.started.toLocaleString()} started, none ended early` : 'No sessions yet'}
        />
        <Tile
          label="Streak"
          icon={<Flame size={12} aria-hidden="true" />}
          value={
            <>
              {stats.streak}
              <small> {stats.streak === 1 ? 'day' : 'days'}</small>
            </>
          }
          sub={`Best ${stats.bestStreak} · ${Math.round(stats.streakGoalMs / 60_000)}m a day counts`}
        />
        <Tile label="Longest stretch" value={fmtDuration(t.longestMs)} sub="Unbroken focus" />
        <Tile
          label="On listed sites"
          value={stats.tracking && t.browseMs > 0 ? `${listedShare}%` : '—'}
          sub={!stats.tracking ? 'Site time is off' : t.browseMs > 0 ? `${fmtDuration(t.distractMs)} of ${fmtDuration(t.browseMs)} browsing` : 'No browsing counted yet'}
        />
      </div>

      <section className="s2-group">
        <div className="fs-section-head">
          <h3 className="s2-group-title">{range === 'day' ? 'Focus by hour' : range === 'year' ? 'Focus by month' : 'Focus by day'}</h3>
          <div className="seg fs-small-seg" role="radiogroup" aria-label="Show as">
            <button type="button" role="radio" aria-checked={!table} className={!table ? 'on' : ''} onClick={() => setTable(false)}>
              Chart
            </button>
            <button type="button" role="radio" aria-checked={table} className={table ? 'on' : ''} onClick={() => setTable(true)}>
              Table
            </button>
          </div>
        </div>
        <div className="s2-card fs-card">
          {table ? (
            <div className="fs-table-wrap">
              <table className="fs-table">
                <thead>
                  <tr>
                    <th scope="col">{range === 'day' ? 'Hour' : range === 'year' ? 'Month' : 'Day'}</th>
                    <th scope="col">Focused</th>
                    <th scope="col">Blocked</th>
                    <th scope="col">Browsing</th>
                  </tr>
                </thead>
                <tbody>
                  {[...stats.buckets].reverse().map((b) => (
                    <tr key={b.key}>
                      <th scope="row">{bucketTitle(stats.range, b.key)}</th>
                      <td className="tnum">{fmtDuration(b.focusMs)}</td>
                      <td className="tnum">{b.blocked.toLocaleString()}</td>
                      <td className="tnum">{fmtDuration(b.browseMs)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : (
            <Columns label="Focused time" bars={focusBars} format={fmtDuration} axis="time" />
          )}
        </div>
      </section>

      <div className="fs-split">
        <section className="s2-group">
          <h3 className="s2-group-title">Where your time went</h3>
          <div className="s2-card fs-card">
            {stats.tracking ? (
              <>
                {stats.sites.length > 0 && (
                <div className="fs-legend" aria-hidden="true">
                  <span>
                    <i className="fs-key" /> Other sites
                  </span>
                  <span>
                    <i className="fs-key listed" /> In a Focus list
                  </span>
                </div>
                )}
                <Ranked
                  favicons
                  rows={stats.sites.map((s) => ({ key: s.host, name: s.host === '(other)' ? 'Other sites' : s.host, value: s.ms, listed: s.listed, tag: s.listed ? 'In a list' : undefined }))}
                  format={fmtDuration}
                  empty="Time on sites shows up here as you browse."
                />
              </>
            ) : (
              <p className="fs-empty-line">Site time is off. Switch it on below to see where your browsing time goes.</p>
            )}
          </div>
        </section>
        <section className="s2-group">
          <h3 className="s2-group-title">What you reached for</h3>
          <div className="s2-card fs-card">
            <Ranked
              favicons
              rows={stats.blockedSites.map((s) => ({ key: s.host, name: s.host === '(other)' ? 'Other sites' : s.host, value: s.count }))}
              format={(v) => `${v.toLocaleString()}×`}
              empty="Nothing blocked in this period."
            />
          </div>
        </section>
      </div>

      <div className="fs-split">
        <section className="s2-group">
          <h3 className="s2-group-title">When you reach for blocked sites</h3>
          <div className="s2-card fs-card">
            {t.blocked > 0 ? (
              <Columns label="Blocked attempts by hour of the day" bars={hourBars} format={(v) => `${v.toLocaleString()} ${v === 1 ? 'attempt' : 'attempts'}`} axis="count" height={110} />
            ) : (
              <p className="fs-empty-line">No blocked attempts in this period.</p>
            )}
          </div>
        </section>
        <section className="s2-group">
          <h3 className="s2-group-title">Focus by list</h3>
          <div className="s2-card fs-card">
            <Ranked rows={stats.lists.map((l) => ({ key: l.id, name: l.name, value: l.ms }))} format={fmtDuration} empty="Start a Focus list to see its time here." />
          </div>
        </section>
      </div>

      <section className="s2-group">
        <h3 className="s2-group-title">Statistics</h3>
        <div className="s2-card">
          <div className="s2-row">
            <div className="s2-row-text">
              <span className="s2-row-title">Count time on sites</span>
              <span className="s2-row-detail">While Serica is in front and you’re using it. Kept only on this PC.</span>
            </div>
            <div className="s2-row-control">
              <Switch
                checked={stats.tracking}
                label="Count time on sites"
                onChange={(on) => {
                  setStats({ ...stats, tracking: on });
                  void window.serica?.focusStatsTracking?.(on).then(load);
                }}
              />
            </div>
          </div>
          <div className="s2-row">
            <div className="s2-row-text">
              <span className="s2-row-title">Export</span>
              <span className="s2-row-detail">{exported ? `Saved to ${exported}` : 'Every day as a CSV file, for a spreadsheet.'}</span>
            </div>
            <div className="s2-row-control">
              <button type="button" className="btn" disabled={!stats.hasData} onClick={() => void window.serica?.focusStatsExport?.().then((path: string | null) => path && setExported(path))}>
                <Download size={13} aria-hidden="true" /> Export CSV…
              </button>
            </div>
          </div>
          <div className="s2-row">
            <div className="s2-row-text">
              <span className="s2-row-title">Clear statistics</span>
              <span className="s2-row-detail">Removes every day recorded. Your lists and sessions stay.</span>
            </div>
            <div className="s2-row-control">
              {confirmClear ? (
                <>
                  <button type="button" className="btn btn-quiet" onClick={() => setConfirmClear(false)}>
                    Keep them
                  </button>
                  <button
                    type="button"
                    className="btn btn-danger"
                    onClick={() => {
                      setConfirmClear(false);
                      void window.serica?.focusStatsClear?.().then(load);
                    }}
                  >
                    Clear statistics
                  </button>
                </>
              ) : (
                <button type="button" className="btn" disabled={!stats.hasData} onClick={() => setConfirmClear(true)}>
                  <Trash2 size={13} aria-hidden="true" /> Clear…
                </button>
              )}
            </div>
          </div>
        </div>
      </section>
    </div>
  );
};

export default FocusStatsPanel;
