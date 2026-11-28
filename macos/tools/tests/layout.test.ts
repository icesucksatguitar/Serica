// Tests for the new-tab widget layout and the settings it is stored in.
// Run: node tools/tests/run.mjs
import assert from 'node:assert/strict';
import { displayLayout, nearestFree, resolveMove, withCells } from '../../src/components/home/layout';
import { normalizeSettings, WIDGET_COLS, rectsOverlap, type WidgetItem } from '../../src/shared/settings';

const item = (id: string, size: WidgetItem['size'], x: number, y: number): WidgetItem => ({ id, kind: 'notes', size, x, y, config: {} });
const noOverlap = (cells: { x: number; y: number; w: number; h: number }[]) => {
  for (let i = 0; i < cells.length; i++) for (let j = i + 1; j < cells.length; j++) assert.ok(!rectsOverlap(cells[i], cells[j]), `overlap ${i}/${j}`);
};

let passed = 0;
const test = (name: string, fn: () => void) => {
  fn();
  passed++;
  console.log('ok -', name);
};

test('saved cells are kept when they fit', () => {
  const items = [item('a', 's', 0, 0), item('b', 'm', 4, 0), item('c', 'l', 8, 0)];
  const { placed, hidden } = displayLayout(items, WIDGET_COLS, 6);
  assert.deepEqual(hidden, []);
  assert.deepEqual(placed.map((p) => [p.id, p.x, p.y]), [['a', 0, 0], ['b', 4, 0], ['c', 8, 0]]);
});

test('a short window moves widgets up into free cells, or hides them', () => {
  const items = [item('a', 'm', 0, 0), item('b', 'm', 0, 4)];
  const short = displayLayout(items, WIDGET_COLS, 3);
  assert.deepEqual(short.hidden, []);
  const b = short.placed.find((p) => p.id === 'b')!;
  assert.ok(b.y + b.h <= 3, 'moved inside the grid');
  noOverlap(short.placed);
  const tiny = displayLayout([item('big', 'l', 0, 0)], WIDGET_COLS, 3);
  assert.deepEqual(tiny.hidden, ['big'], 'a 4-row widget can not show in 3 rows');
});

test('overlapping saved cells never overlap on screen', () => {
  const items = [item('a', 'm', 0, 0), item('b', 'm', 2, 0), item('c', 's', 1, 1)];
  const { placed } = displayLayout(items, WIDGET_COLS, 6);
  noOverlap(placed);
});

test('moving onto a widget pushes it to the nearest free cell', () => {
  const layout = displayLayout([item('a', 's', 0, 0), item('b', 's', 2, 0)], WIDGET_COLS, 4).placed;
  const next = resolveMove(layout, 'a', { x: 2, y: 0, w: 2, h: 2 }, WIDGET_COLS, 4)!;
  assert.ok(next);
  const a = next.find((p) => p.id === 'a')!;
  const b = next.find((p) => p.id === 'b')!;
  assert.deepEqual([a.x, a.y], [2, 0]);
  noOverlap(next);
  assert.ok(Math.abs(b.x - 2) + Math.abs(b.y - 0) <= 2, `b stays close, got ${b.x},${b.y}`);
});

test('moves are clamped to the grid', () => {
  const layout = displayLayout([item('a', 'm', 0, 0)], WIDGET_COLS, 4).placed;
  const next = resolveMove(layout, 'a', { x: 11, y: 9, w: 4, h: 2 }, WIDGET_COLS, 4)!;
  const a = next[0];
  assert.deepEqual([a.x, a.y], [8, 2]);
});

test('a move with no room for the displaced widgets is refused', () => {
  // A full 4x2 grid: two smalls side by side, no free cell anywhere.
  const layout = displayLayout([item('a', 's', 0, 0), item('b', 's', 2, 0)], 4, 2).placed;
  assert.equal(resolveMove(layout, 'a', { x: 1, y: 0, w: 2, h: 2 }, 4, 2), null);
});

test('resolution is deterministic', () => {
  const layout = displayLayout([item('a', 's', 0, 0), item('b', 's', 2, 0), item('c', 's', 4, 0), item('d', 'm', 0, 2)], WIDGET_COLS, 6).placed;
  const r1 = JSON.stringify(resolveMove(layout, 'd', { x: 1, y: 0, w: 4, h: 2 }, WIDGET_COLS, 6));
  const r2 = JSON.stringify(resolveMove(layout, 'd', { x: 1, y: 0, w: 4, h: 2 }, WIDGET_COLS, 6));
  assert.equal(r1, r2);
});

test('nearestFree prefers the closest cell', () => {
  assert.deepEqual(nearestFree([{ x: 0, y: 0, w: 2, h: 2 }], { x: 0, y: 0 }, 2, 2, 12, 4), { x: 2, y: 0 });
});

test('withCells writes cells back to saved items', () => {
  const items = [item('a', 's', 0, 0)];
  assert.deepEqual(withCells(items, [{ id: 'a', x: 3, y: 1, w: 2, h: 2 }])[0], { ...items[0], x: 3, y: 1 });
});

test('0.4.7 widget lists (no positions) are laid out in order', () => {
  const s = normalizeSettings({
    homeWidgets: { clock: true, search: true, github: { user: 'octo' }, weather: { place: 'Paris', lat: 48.85, lon: 2.35, fahrenheit: false }, items: [{ id: 'g', kind: 'github', size: 'w' }, { id: 'w', kind: 'weather', size: 's' }, { id: 'n', kind: 'notes', size: 'm' }], notes: { n: 'hello' } },
  });
  const it = s.homeWidgets.items;
  assert.deepEqual(it.map((i) => [i.id, i.x, i.y]), [['g', 0, 0], ['w', 8, 0], ['n', 0, 2]]);
  assert.deepEqual(it[0].config, { user: 'octo' }, 'github user carried into the widget');
  assert.equal((it[1].config as any).place, 'Paris');
  assert.equal(s.homeWidgets.notes.n, 'hello');
  noOverlap(it.map((i) => ({ x: i.x, y: i.y, w: i.size === 'w' ? 8 : i.size === 's' ? 2 : 4, h: i.size === 'l' ? 4 : 2 })));
});

test('0.4.6 per-widget flags still migrate', () => {
  const s = normalizeSettings({ homeWidgets: { weather: { enabled: true, place: 'Oslo', lat: 59.9, lon: 10.7 }, notes: { enabled: true, text: 'x' } } });
  assert.deepEqual(s.homeWidgets.items.map((i) => i.kind), ['weather', 'notes']);
  assert.equal(s.homeWidgets.notes['notes-1'], 'x');
});

test('unknown widget kinds and bad sizes are cleaned', () => {
  const s = normalizeSettings({ homeWidgets: { items: [{ id: 'z', kind: 'virus', size: 'm', x: 0, y: 0 }, { id: 'q', kind: 'calendar', size: 'huge', x: 0, y: 0 }] } });
  assert.deepEqual(s.homeWidgets.items.map((i) => [i.id, i.size]), [['q', 'm']]);
});

test('home background is validated', () => {
  const s = normalizeSettings({ homeBackground: { mode: 'image', image: { file: '../../etc/passwd' } } });
  assert.equal(s.homeBackground.mode, 'theme', 'image mode without a valid file falls back');
  const g = normalizeSettings({ homeBackground: { mode: 'gradient', gradient: { type: 'radial', angle: -30, stops: [{ color: '#ff0000', at: 80 }, { color: '#00ff00', at: 10 }, { color: 'red', at: 5 }] } } });
  assert.equal(g.homeBackground.gradient.type, 'radial');
  assert.equal(g.homeBackground.gradient.angle, 330);
  assert.deepEqual(g.homeBackground.gradient.stops, [{ color: '#00FF00', at: 10 }, { color: '#FF0000', at: 80 }]);
  const sp = normalizeSettings({ homeBackground: { mode: 'special', special: { seconds: 2 } } });
  assert.equal(sp.homeBackground.special.seconds, 15);
});

test('setup and tour flags', () => {
  assert.equal(normalizeSettings({}).setupVersion, 0);
  assert.equal(normalizeSettings({ setupVersion: 2, tourDone: true }).tourDone, true);
});

console.log(`\n${passed} passed`);
