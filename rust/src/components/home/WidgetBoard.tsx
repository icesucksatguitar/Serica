import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { HelpCircle, Minus, Plus, X } from 'lucide-react';
import type { HomeWidgets, WidgetItem, WidgetKind, WidgetSize } from '../../shared/settings';
import { APP_KINDS, BUILTIN_KINDS, SIZE_LABEL, WidgetView, widgetMeta, type WidgetContext } from './widgets';
import { GuideTour, type GuideStep } from '../GuideTour';

const WIDGET_GUIDE: GuideStep[] = [
  {
    target: '.wboard-add',
    title: 'Add widgets',
    body: 'Pick from Serica’s widgets (GitHub, weather, notes, calendar, top sites) and app widgets for Claude, Notion, Gmail and more.',
  },
  {
    target: '.wtile',
    title: 'Move them around',
    body: 'While editing, drag a widget onto another to change the order. The others make room as you go.',
  },
  {
    target: '.wtile .wsize',
    title: 'Change the size',
    body: 'Small, Medium, Large or Wide: each widget shows more as it gets bigger. Remove one with the minus at its corner.',
  },
  {
    target: '.wboard-done',
    title: 'Done',
    body: 'Click Done when you are finished. Edit again any time from Customize, at the bottom right of the new tab.',
  },
];

/** Cells each size takes: [columns, rows]. */
const SPAN: Record<WidgetSize, [number, number]> = { s: [2, 2], m: [4, 2], l: [4, 4], w: [8, 2] };
/** At most this many widgets, whatever the window size: more gets busy. */
export const MAX_WIDGETS = 6;

/**
 * Which items fit a grid of `cols` x `rows` cells, placed in order the way
 * the CSS grid places them (row by row, first free spot, filling gaps).
 */
function fitting(items: { id: string; size: WidgetSize }[], cols: number, rows: number): Set<string> {
  const used: boolean[][] = Array.from({ length: rows }, () => Array(cols).fill(false));
  const fits = new Set<string>();
  for (const it of items) {
    const w = Math.min(SPAN[it.size][0], cols);
    const h = SPAN[it.size][1];
    let placed = false;
    for (let r = 0; r + h <= rows && !placed; r++) {
      for (let c = 0; c + w <= cols && !placed; c++) {
        let free = true;
        for (let y = r; y < r + h && free; y++) for (let x = c; x < c + w && free; x++) if (used[y][x]) free = false;
        if (!free) continue;
        for (let y = r; y < r + h; y++) for (let x = c; x < c + w; x++) used[y][x] = true;
        placed = true;
      }
    }
    if (placed) fits.add(it.id);
  }
  return fits;
}

interface Drag {
  id: string;
  pointerId: number;
  grabX: number;
  grabY: number;
  startX: number;
  startY: number;
  moved: boolean;
}

/**
 * The new tab's widgets: a grid in which every widget takes a fixed number of
 * cells for its size, in the order the user arranged them. In edit mode the
 * tiles can be dragged to a new place (the others glide out of the way),
 * resized, removed, or added from the gallery.
 */
export const WidgetBoard: React.FC<{
  widgets: HomeWidgets;
  onUpdate: (next: HomeWidgets) => void;
  onOpen: (url: string) => void;
  editing: boolean;
  onEditingChange: (editing: boolean) => void;
  scale: number;
}> = ({ widgets, onUpdate, onOpen, editing, onEditingChange, scale }) => {
  const [order, setOrder] = useState<string[]>(() => widgets.items.map((i) => i.id));
  const [galleryOpen, setGalleryOpen] = useState(false);
  const [guide, setGuide] = useState(false);
  const gridRef = useRef<HTMLDivElement>(null);
  const drag = useRef<Drag | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const rects = useRef(new Map<string, DOMRect>());

  // Follow saved changes, except while a drag is rearranging the tiles.
  useEffect(() => {
    if (!drag.current) setOrder(widgets.items.map((i) => i.id));
  }, [widgets.items]);

  const byId = new Map(widgets.items.map((i) => [i.id, i]));
  const allItems = order.map((id) => byId.get(id)).filter((i): i is WidgetItem => !!i);

  // The new tab never scrolls: widgets get the rows that fit between the
  // search bar and the bottom of the window, and no more.
  const [room, setRoom] = useState<{ cols: number; rows: number }>({ cols: 8, rows: 2 });
  useLayoutEffect(() => {
    const measure = () => {
      const grid = gridRef.current;
      if (!grid) return;
      const cs = getComputedStyle(grid);
      const cell = parseFloat(cs.gridAutoRows) || 72;
      const gap = parseFloat(cs.rowGap) || 14;
      const top = grid.getBoundingClientRect().top;
      const bottomReserve = 64; // Customize and the edit bar
      const rows = Math.max(0, Math.floor((window.innerHeight - top - bottomReserve + gap) / (cell + gap)));
      const cols = Math.max(2, Math.min(8, Math.floor((grid.clientWidth + gap) / (cell + gap))));
      setRoom((r) => (r.rows === rows && r.cols === cols ? r : { cols, rows }));
    };
    measure();
    window.addEventListener('resize', measure);
    return () => window.removeEventListener('resize', measure);
  }, [scale, widgets.clock, widgets.search]);

  const shownIds = fitting(allItems.slice(0, MAX_WIDGETS), room.cols, room.rows);
  const items = allItems.filter((i) => shownIds.has(i.id));
  const hiddenCount = allItems.length - items.length;
  /** Whether everything in `list` (shown widgets plus a new or resized one) fits. */
  const fitsWith = (list: { id: string; size: WidgetSize }[]) =>
    list.length <= MAX_WIDGETS && fitting(list, room.cols, room.rows).size === list.length;
  /** The size a new widget of `kind` is added at: its default, else the smallest that fits. */
  const addSizeFor = (kind: WidgetKind): WidgetSize | null => {
    const meta = widgetMeta(kind);
    if (!meta) return null;
    const area = (x: WidgetSize) => SPAN[x][0] * SPAN[x][1];
    const tries = [meta.defaultSize, ...meta.sizes.filter((x) => x !== meta.defaultSize).sort((a, b) => area(a) - area(b))];
    return tries.find((size) => fitsWith([...items, { id: '__new', size }])) ?? null;
  };
  const roomForAddTile = editing && fitsWith([...items, { id: '__add', size: 's' }]);

  const ctx: WidgetContext = { widgets, update: onUpdate, open: onOpen, editing };

  const save = useCallback(
    (nextItems: WidgetItem[]) => onUpdate({ ...widgets, items: nextItems }),
    [onUpdate, widgets],
  );

  // First time in edit mode, the guide explains it.
  useEffect(() => {
    if (editing && !widgets.guideSeen) setGuide(true);
  }, [editing, widgets.guideSeen]);
  useEffect(() => {
    const onGuide = (e: Event) => {
      if ((e as CustomEvent).detail !== 'widgets') return;
      onEditingChange(true);
      setGuide(true);
    };
    window.addEventListener('serica:guide', onGuide);
    return () => window.removeEventListener('serica:guide', onGuide);
  }, [onEditingChange]);
  const closeGuide = () => {
    setGuide(false);
    if (!widgets.guideSeen) onUpdate({ ...widgets, guideSeen: true });
  };

  // Escape leaves edit mode (the gallery and guide handle their own Escape).
  useEffect(() => {
    if (!editing || galleryOpen || guide) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onEditingChange(false);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [editing, galleryOpen, guide, onEditingChange]);

  // ---- Reflow animation (FLIP): tiles glide from where they were.
  const captureRects = () => {
    rects.current.clear();
    gridRef.current?.querySelectorAll<HTMLElement>('[data-widget-id]').forEach((el) => {
      rects.current.set(el.dataset.widgetId!, el.getBoundingClientRect());
    });
  };
  useLayoutEffect(() => {
    if (rects.current.size === 0) return;
    const grid = gridRef.current;
    if (!grid) return;
    grid.querySelectorAll<HTMLElement>('[data-widget-id]').forEach((el) => {
      const id = el.dataset.widgetId!;
      const before = rects.current.get(id);
      if (!before || id === drag.current?.id) return;
      const after = el.getBoundingClientRect();
      const dx = before.left - after.left;
      const dy = before.top - after.top;
      if (!dx && !dy) return;
      el.style.transition = 'none';
      el.style.transform = `translate(${dx}px, ${dy}px)`;
      requestAnimationFrame(() => {
        el.style.transition = '';
        el.style.transform = '';
      });
    });
    rects.current.clear();
  }, [order, widgets.items]);

  // ---- Drag to rearrange (edit mode)
  const placeDragged = (el: HTMLElement, x: number, y: number) => {
    const g = gridRef.current!.getBoundingClientRect();
    const dx = x - g.left - drag.current!.grabX - el.offsetLeft;
    const dy = y - g.top - drag.current!.grabY - el.offsetTop;
    el.style.transform = `translate(${dx}px, ${dy}px) scale(1.03)`;
  };

  const onPointerDown = (e: React.PointerEvent<HTMLElement>, id: string) => {
    if (!editing || e.button !== 0) return;
    if ((e.target as HTMLElement).closest('button, input, textarea, a, select')) return;
    const el = e.currentTarget;
    const r = el.getBoundingClientRect();
    drag.current = { id, pointerId: e.pointerId, grabX: e.clientX - r.left, grabY: e.clientY - r.top, startX: e.clientX, startY: e.clientY, moved: false };
    el.setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: React.PointerEvent<HTMLElement>) => {
    const d = drag.current;
    if (!d || e.pointerId !== d.pointerId) return;
    const el = e.currentTarget;
    if (!d.moved) {
      if (Math.hypot(e.clientX - d.startX, e.clientY - d.startY) < 4) return;
      d.moved = true;
      setDragId(d.id);
    }
    placeDragged(el, e.clientX, e.clientY);
    // The tile under the pointer (the dragged one ignores the pointer).
    const under = document
      .elementsFromPoint(e.clientX, e.clientY)
      .map((n) => (n as HTMLElement).closest<HTMLElement>('[data-widget-id]'))
      .find((n) => n && n.dataset.widgetId !== d.id);
    if (!under) return;
    const targetId = under.dataset.widgetId!;
    const r = under.getBoundingClientRect();
    const after = e.clientX > r.left + r.width / 2 || e.clientY > r.top + r.height * 0.75;
    setOrder((cur) => {
      const without = cur.filter((x) => x !== d.id);
      const at = without.indexOf(targetId) + (after ? 1 : 0);
      const next = [...without.slice(0, at), d.id, ...without.slice(at)];
      if (next.join() === cur.join()) return cur;
      captureRects();
      return next;
    });
    requestAnimationFrame(() => {
      if (drag.current) placeDragged(el, e.clientX, e.clientY);
    });
  };

  const endDrag = (e: React.PointerEvent<HTMLElement>) => {
    const d = drag.current;
    if (!d || e.pointerId !== d.pointerId) return;
    drag.current = null;
    setDragId(null);
    const el = e.currentTarget;
    // Glide into its slot.
    el.style.transform = '';
    if (d.moved) {
      const next = order.map((id) => byId.get(id)).filter((i): i is WidgetItem => !!i);
      if (next.map((i) => i.id).join() !== widgets.items.map((i) => i.id).join()) save(next);
    }
  };

  const resize = (id: string, size: WidgetSize) => {
    captureRects();
    save(widgets.items.map((i) => (i.id === id ? { ...i, size } : i)));
  };
  const remove = (id: string) => {
    captureRects();
    const notes = { ...widgets.notes };
    delete notes[id];
    onUpdate({ ...widgets, notes, items: widgets.items.filter((i) => i.id !== id) });
  };
  const add = (kind: WidgetKind) => {
    const size = addSizeFor(kind);
    if (!size) return;
    captureRects();
    save([...widgets.items, { id: `${kind.replace(':', '-')}-${Date.now().toString(36)}`, kind, size }]);
    setGalleryOpen(false);
  };
  const canAddAny = items.length < MAX_WIDGETS && [...BUILTIN_KINDS, ...APP_KINDS].some((k) => addSizeFor(k) !== null);

  if (!editing && items.length === 0) return null;

  return (
    <div className={`wboard${editing ? ' editing' : ''}`} style={{ '--wscale': scale } as React.CSSProperties} onClick={(e) => e.stopPropagation()}>
      <div className="wgrid" ref={gridRef}>
        {items.map((item) => {
          const meta = widgetMeta(item.kind);
          if (!meta) return null;
          return (
            <div
              key={item.id}
              data-widget-id={item.id}
              className={`wtile size-${item.size}${dragId === item.id ? ' dragging' : ''}`}
              onPointerDown={(e) => onPointerDown(e, item.id)}
              onPointerMove={onPointerMove}
              onPointerUp={endDrag}
              onPointerCancel={endDrag}
              aria-label={meta.name}
            >
              <WidgetView item={item} ctx={ctx} />
              {editing && (
                <>
                  <button type="button" className="wremove" aria-label={`Remove ${meta.name}`} title="Remove" onClick={() => remove(item.id)}>
                    <Minus size={12} strokeWidth={3} />
                  </button>
                  {meta.sizes.length > 1 && (
                    <div className="wsize" role="radiogroup" aria-label={`${meta.name} size`}>
                      {meta.sizes.map((s) => {
                        const fitsSize =
                          item.size === s || fitsWith(items.map((i) => (i.id === item.id ? { id: i.id, size: s } : i)));
                        return (
                          <button
                            key={s}
                            type="button"
                            role="radio"
                            aria-checked={item.size === s}
                            title={fitsSize ? SIZE_LABEL[s] : `${SIZE_LABEL[s]}: no room`}
                            className={item.size === s ? 'on' : ''}
                            disabled={!fitsSize}
                            onClick={() => resize(item.id, s)}
                          >
                            {SIZE_LABEL[s][0]}
                          </button>
                        );
                      })}
                    </div>
                  )}
                </>
              )}
            </div>
          );
        })}
        {roomForAddTile && canAddAny && (
          <button type="button" className="wtile size-s wboard-add" onClick={() => setGalleryOpen(true)}>
            <Plus size={20} aria-hidden="true" />
            <span>Add widget</span>
          </button>
        )}
      </div>

      {editing && (
        <div className="wboard-bar" role="toolbar" aria-label="Editing widgets">
          {(hiddenCount > 0 || !canAddAny) && (
            <span className="wboard-note">
              {hiddenCount > 0
                ? `${hiddenCount} ${hiddenCount === 1 ? 'widget is' : 'widgets are'} hidden: not enough room in this window`
                : items.length >= MAX_WIDGETS
                  ? `Up to ${MAX_WIDGETS} widgets`
                  : 'No room for more'}
            </span>
          )}
          <button
            type="button"
            className="btn"
            disabled={!canAddAny}
            title={canAddAny ? undefined : 'Remove or shrink a widget to make room'}
            onClick={() => setGalleryOpen(true)}
          >
            <Plus size={13} aria-hidden="true" />
            Add widget
          </button>
          <button type="button" className="btn btn-quiet btn-icon" title="How editing works" aria-label="How editing works" onClick={() => setGuide(true)}>
            <HelpCircle size={15} />
          </button>
          <button type="button" className="btn btn-primary wboard-done" onClick={() => onEditingChange(false)}>
            Done
          </button>
        </div>
      )}

      {galleryOpen && (
        <div
          className="modal-overlay wgallery-overlay"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) setGalleryOpen(false);
          }}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              e.stopPropagation();
              setGalleryOpen(false);
            }
          }}
        >
          <div className="panel wgallery" role="dialog" aria-modal="true" aria-label="Add a widget">
            <header className="panel-head">
              <h2 className="panel-title">Add a widget</h2>
              <button type="button" className="btn btn-quiet btn-icon" aria-label="Close" autoFocus onClick={() => setGalleryOpen(false)}>
                <X size={16} />
              </button>
            </header>
            <div className="panel-body">
              {(
                [
                  ['Serica', BUILTIN_KINDS],
                  ['Apps', APP_KINDS],
                ] as const
              ).map(([title, kinds]) => (
                <section key={title} className="panel-section">
                  <h3 className="panel-label">{title}</h3>
                  <div className="wgallery-grid">
                    {kinds.map((kind) => {
                      const meta = widgetMeta(kind)!;
                      const size = addSizeFor(kind);
                      return (
                        <button
                          key={kind}
                          type="button"
                          className="wgallery-item"
                          disabled={!size}
                          title={size ? `Adds a ${SIZE_LABEL[size].toLowerCase()} widget` : 'No room left: remove or shrink a widget'}
                          onClick={() => add(kind)}
                        >
                          <span className="wgallery-icon">{meta.icon}</span>
                          <span className="wgallery-text">
                            <span className="wgallery-name">{meta.name}</span>
                            <span className="wgallery-desc">{meta.description}</span>
                          </span>
                          <Plus size={14} className="wgallery-plus" aria-hidden="true" />
                        </button>
                      );
                    })}
                  </div>
                </section>
              ))}
            </div>
          </div>
        </div>
      )}

      {guide && <GuideTour id="widgets" steps={WIDGET_GUIDE} onClose={closeGuide} />}
    </div>
  );
};
