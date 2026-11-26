// Widget layout on the new-tab grid. Pure functions, no DOM.
//
// Every widget has a saved cell (x, y) on a grid of WIDGET_COLS columns. The
// window decides how many rows are visible. The layout shown is derived
// from the saved one: widgets keep their cells when they fit, and are moved
// to the nearest free cell (or hidden) when the window is too short. Saved
// positions only change when the user moves, resizes, adds or removes a
// widget.

import { WIDGET_SPAN, rectsOverlap, type CellRect, type WidgetItem, type WidgetSize } from '../../shared/settings';

export interface Placed extends CellRect {
  id: string;
}

export function spanOf(size: WidgetSize): { w: number; h: number } {
  const [w, h] = WIDGET_SPAN[size];
  return { w, h };
}

export function rectOf(item: Pick<WidgetItem, 'x' | 'y' | 'size'>): CellRect {
  return { x: item.x, y: item.y, ...spanOf(item.size) };
}

function inside(r: CellRect, cols: number, rows: number): boolean {
  return r.x >= 0 && r.y >= 0 && r.x + r.w <= cols && r.y + r.h <= rows;
}

/** Keeps a rectangle on the grid (moves it in from any edge it crosses). */
export function clampRect(r: CellRect, cols: number, rows: number): CellRect {
  const w = Math.min(r.w, cols);
  const h = Math.min(r.h, rows);
  return { x: Math.max(0, Math.min(cols - w, r.x)), y: Math.max(0, Math.min(rows - h, r.y)), w, h };
}

/**
 * The free cell nearest to `want` for a `w` x `h` rectangle (distance from
 * the wanted top-left, ties broken by row then column so the result never
 * depends on iteration order), or null when nothing fits.
 */
export function nearestFree(placed: CellRect[], want: { x: number; y: number }, w: number, h: number, cols: number, rows: number): { x: number; y: number } | null {
  let best: { x: number; y: number; d: number } | null = null;
  for (let y = 0; y + h <= rows; y++) {
    for (let x = 0; x + w <= cols; x++) {
      const r = { x, y, w, h };
      if (placed.some((p) => rectsOverlap(p, r))) continue;
      const d = (x - want.x) ** 2 + (y - want.y) ** 2 * 1.5;
      if (!best || d < best.d || (d === best.d && (y < best.y || (y === best.y && x < best.x)))) best = { x, y, d };
    }
  }
  return best ? { x: best.x, y: best.y } : null;
}

/**
 * Where each widget shows in a grid of `cols` x `rows`: its saved cell when
 * it fits and is free, else the nearest free cell. Widgets that find no
 * room are left out (`hidden`).
 */
export function displayLayout(items: WidgetItem[], cols: number, rows: number): { placed: Placed[]; hidden: string[] } {
  const placed: Placed[] = [];
  const hidden: string[] = [];
  // Earlier rows (then columns) claim their cells first.
  const order = [...items].sort((a, b) => a.y - b.y || a.x - b.x);
  for (const it of order) {
    const r = rectOf(it);
    if (inside(r, cols, rows) && !placed.some((p) => rectsOverlap(p, r))) {
      placed.push({ id: it.id, ...r });
      continue;
    }
    if (r.w > cols || r.h > rows) {
      hidden.push(it.id);
      continue;
    }
    const pos = nearestFree(placed, clampRect(r, cols, rows), r.w, r.h, cols, rows);
    if (pos) placed.push({ id: it.id, x: pos.x, y: pos.y, w: r.w, h: r.h });
    else hidden.push(it.id);
  }
  return { placed, hidden };
}

/**
 * Puts `moving` at `target` (clamped to the grid) and pushes whatever it
 * covers to the nearest free cells, nearest widgets first. Returns the new
 * cells of every widget, or null when the others can't all be fitted (the
 * move is then refused).
 */
export function resolveMove(
  layout: Placed[],
  movingId: string,
  target: CellRect,
  cols: number,
  rows: number,
): Placed[] | null {
  const moving: Placed = { id: movingId, ...clampRect(target, cols, rows) };
  const out: Placed[] = [moving];
  const others = layout.filter((p) => p.id !== movingId);
  const keep = others.filter((p) => !rectsOverlap(p, moving));
  out.push(...keep);
  // Displaced widgets, closest to the moved one first (deterministic).
  const center = (r: CellRect) => [r.x + r.w / 2, r.y + r.h / 2];
  const [mx, my] = center(moving);
  const displaced = others
    .filter((p) => rectsOverlap(p, moving))
    .sort((a, b) => {
      const [ax, ay] = center(a);
      const [bx, by] = center(b);
      return (ax - mx) ** 2 + (ay - my) ** 2 - ((bx - mx) ** 2 + (by - my) ** 2) || a.y - b.y || a.x - b.x;
    });
  for (const d of displaced) {
    const pos = nearestFree(out, d, d.w, d.h, cols, rows);
    if (!pos) return null;
    out.push({ ...d, x: pos.x, y: pos.y });
  }
  return out;
}

/** Applies display cells back to saved widgets (after a move or resize). */
export function withCells(items: WidgetItem[], cells: Placed[]): WidgetItem[] {
  const by = new Map(cells.map((c) => [c.id, c]));
  return items.map((it) => {
    const c = by.get(it.id);
    return c ? { ...it, x: c.x, y: c.y } : it;
  });
}
