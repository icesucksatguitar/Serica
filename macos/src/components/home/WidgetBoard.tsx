import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { HelpCircle, Minus, Plus, SlidersHorizontal, X } from 'lucide-react';
import { WIDGET_COLS, WIDGET_SPAN, firstFreeCell, type HomeWidgets, type WidgetItem, type WidgetKind, type WidgetSize } from '../../shared/settings';
import { APP_KINDS, BUILTIN_KINDS, SIZE_LABEL, WidgetView, widgetMeta, type WidgetContext } from './widgets';
import { WidgetEditor } from './WidgetEditor';
import { GuideTour, type GuideStep } from '../GuideTour';
import { displayLayout, resolveMove, spanOf, withCells, type Placed } from './layout';
import { usePresence } from '../../hooks/usePresence';

const WIDGET_GUIDE: GuideStep[] = [
  { target: '.wboard-add-btn', title: 'Add widgets', body: 'Weather, a world clock, Focus, notes, your calendar, top sites, GitHub, and widgets for apps like Claude, Notion and Gmail.' },
  { target: '.wtile', title: 'Drag to place', body: 'Drag a widget anywhere on the grid. Widgets in the way slide aside. Arrow keys move the selected one a cell at a time.' },
  { target: '.wtile .wtile-edit', title: 'Make it yours', body: 'Open a widget’s settings to change its size and what it shows: the city, the time zone, the paper colour…' },
  { target: '.wboard-done', title: 'Done', body: 'Click Done when you’re finished. Press and hold any widget to edit again.' },
];

const GAP = 14;
const LONG_PRESS_MS = 450;

interface Geometry {
  cell: number;
  rows: number;
  /** Left of the grid inside the board (it is centred). */
  left: number;
  width: number;
}

interface Drag {
  id: string;
  pointerId: number;
  /** Pointer position at pointer-down (client px). */
  originX: number;
  originY: number;
  /** The tile's grid position (board px) at pointer-down. */
  startPx: number;
  startPy: number;
  w: number;
  h: number;
  moved: boolean;
  target: { x: number; y: number } | null;
}

const cellPx = (n: number, cell: number) => n * (cell + GAP);
const spanPx = (n: number, cell: number) => n * cell + (n - 1) * GAP;

/**
 * The new tab's widgets on an explicit grid of WIDGET_COLS columns. Every
 * widget keeps the cell it was put in. In edit mode widgets are dragged with
 * the pointer captured: the tile follows the pointer's offset from where it
 * was grabbed and stays inside the grid; the cell under it is the target,
 * and widgets in the way slide to the nearest free cells. Changes are saved
 * when dropped.
 */
export const WidgetBoard: React.FC<{
  widgets: HomeWidgets;
  onUpdate: (next: HomeWidgets) => void;
  onOpen: (url: string) => void;
  editing: boolean;
  onEditingChange: (editing: boolean) => void;
  galleryOpen: boolean;
  onGalleryOpenChange: (open: boolean) => void;
  scale: number;
}> = ({ widgets, onUpdate, onOpen, editing, onEditingChange, galleryOpen, onGalleryOpenChange, scale }) => {
  const boardRef = useRef<HTMLDivElement>(null);
  const [geo, setGeo] = useState<Geometry>({ cell: 72, rows: 0, left: 0, width: 0 });
  const [guide, setGuide] = useState(false);
  const [editorId, setEditorId] = useState<string | null>(null);
  const [preview, setPreview] = useState<Placed[] | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const drag = useRef<Drag | null>(null);
  const press = useRef<{ timer: number; x: number; y: number; id: string; pointerId: number; el: HTMLElement } | null>(null);
  const tileEls = useRef(new Map<string, HTMLElement>());

  // ---- Geometry, measured only when the board's size changes.
  useLayoutEffect(() => {
    const el = boardRef.current;
    if (!el) return;
    const measure = () => {
      const W = el.clientWidth;
      const H = el.clientHeight;
      const cell = Math.max(48, Math.min(Math.round(84 * scale), Math.floor((W - (WIDGET_COLS - 1) * GAP) / WIDGET_COLS)));
      const width = spanPx(WIDGET_COLS, cell);
      const rows = Math.max(0, Math.floor((H + GAP) / (cell + GAP)));
      setGeo((g) => (g.cell === cell && g.rows === rows && g.width === width && g.left === Math.round((W - width) / 2) ? g : { cell, rows, width, left: Math.round((W - width) / 2) }));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [scale]);

  const items = widgets.items;
  const display = useMemo(() => displayLayout(items, WIDGET_COLS, geo.rows), [items, geo.rows]);
  const cells: Placed[] = preview ?? display.placed;
  const cellOf = useMemo(() => new Map(cells.map((c) => [c.id, c])), [cells]);
  const byId = useMemo(() => new Map(items.map((i) => [i.id, i])), [items]);

  // ---- Saving
  const save = useCallback((nextItems: WidgetItem[], extra?: Partial<HomeWidgets>) => onUpdate({ ...widgets, ...extra, items: nextItems }), [onUpdate, widgets]);
  const commitCells = useCallback((next: Placed[]) => save(withCells(items, next)), [items, save]);

  const setConfig = useCallback(
    (id: string, patch: Record<string, unknown>) => save(items.map((i) => (i.id === id ? { ...i, config: { ...i.config, ...patch } } : i))),
    [items, save],
  );

  const tryPlace = (id: string, x: number, y: number, size: WidgetSize) => {
    const { w, h } = spanOf(size);
    return resolveMove(display.placed, id, { x, y, w, h }, WIDGET_COLS, geo.rows);
  };

  const resize = (id: string, size: WidgetSize) => {
    const c = cellOf.get(id);
    if (!c) return;
    const next = tryPlace(id, c.x, c.y, size);
    if (!next) return;
    save(withCells(items, next).map((i) => (i.id === id ? { ...i, size } : i)));
  };
  const canResize = (id: string, size: WidgetSize) => {
    const c = cellOf.get(id);
    return !!c && !!tryPlace(id, c.x, c.y, size);
  };
  const remove = (id: string) => {
    const notes = { ...widgets.notes };
    delete notes[id];
    setEditorId(null);
    save(items.filter((i) => i.id !== id), { notes });
  };
  const freeCellFor = (size: WidgetSize) => {
    const [w, h] = WIDGET_SPAN[size];
    return firstFreeCell(display.placed, w, h, WIDGET_COLS, geo.rows);
  };
  const addSizeFor = (kind: WidgetKind): WidgetSize | null => {
    const meta = widgetMeta(kind);
    if (!meta) return null;
    const area = (s: WidgetSize) => WIDGET_SPAN[s][0] * WIDGET_SPAN[s][1];
    const tries = [meta.defaultSize, ...meta.sizes.filter((s) => s !== meta.defaultSize).sort((a, b) => area(a) - area(b))];
    return tries.find((s) => freeCellFor(s) !== null) ?? null;
  };
  const add = (kind: WidgetKind) => {
    const size = addSizeFor(kind);
    const meta = widgetMeta(kind);
    if (!size || !meta) return;
    const pos = freeCellFor(size)!;
    const id = `${kind.replace(':', '-')}-${Date.now().toString(36)}`;
    save([...items, { id, kind, size, x: pos.x, y: pos.y, config: meta.defaultConfig() }]);
    onGalleryOpenChange(false);
    setSelectedId(id);
    if (meta.configurable) setEditorId(id);
  };
  const canAddAny = items.length < 24 && [...BUILTIN_KINDS, ...APP_KINDS].some((k) => addSizeFor(k) !== null);

  // ---- Guide
  useEffect(() => {
    if (editing && !widgets.guideSeen && items.length > 0) setGuide(true);
  }, [editing, widgets.guideSeen, items.length]);
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

  // Escape leaves edit mode (sheets and the guide handle their own).
  useEffect(() => {
    if (!editing || galleryOpen || guide || editorId) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onEditingChange(false);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [editing, galleryOpen, guide, editorId, onEditingChange]);
  useEffect(() => {
    if (!editing) setSelectedId(null);
  }, [editing]);

  // ---- Dragging
  const boardRect = useRef<DOMRect | null>(null);
  const beginDrag = (id: string, pointerId: number, x: number, y: number, el: HTMLElement) => {
    const c = cellOf.get(id);
    if (!c) return;
    boardRect.current = boardRef.current!.getBoundingClientRect();
    drag.current = { id, pointerId, originX: x, originY: y, startPx: cellPx(c.x, geo.cell), startPy: cellPx(c.y, geo.cell), w: c.w, h: c.h, moved: false, target: { x: c.x, y: c.y } };
    try {
      el.setPointerCapture(pointerId);
    } catch {
      // The pointer is already gone.
    }
  };

  const onTilePointerDown = (e: React.PointerEvent<HTMLElement>, id: string) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest('button, input, textarea, a, select, .wtile-chrome')) return;
    const el = e.currentTarget;
    if (editing) {
      setSelectedId(id);
      beginDrag(id, e.pointerId, e.clientX, e.clientY, el);
      return;
    }
    // Press and hold a widget to start editing (and dragging) it.
    const timer = window.setTimeout(() => {
      const p = press.current;
      press.current = null;
      if (!p) return;
      onEditingChange(true);
      setSelectedId(p.id);
      beginDrag(p.id, p.pointerId, p.x, p.y, p.el);
    }, LONG_PRESS_MS);
    press.current = { timer, x: e.clientX, y: e.clientY, id, pointerId: e.pointerId, el };
  };

  const onTilePointerMove = (e: React.PointerEvent<HTMLElement>) => {
    const p = press.current;
    if (p && Math.hypot(e.clientX - p.x, e.clientY - p.y) > 6) {
      window.clearTimeout(p.timer);
      press.current = null;
    }
    const d = drag.current;
    if (!d || e.pointerId !== d.pointerId) return;
    const dx = e.clientX - d.originX;
    const dy = e.clientY - d.originY;
    if (!d.moved) {
      if (Math.hypot(dx, dy) < 4) return;
      d.moved = true;
      setDragId(d.id);
    }
    // Relative to the pointer-down origin, inside the grid's bounds.
    const maxX = spanPx(WIDGET_COLS, geo.cell) - spanPx(d.w, geo.cell);
    const maxY = Math.max(0, cellPx(geo.rows, geo.cell) - GAP - spanPx(d.h, geo.cell));
    const px = Math.max(0, Math.min(maxX, d.startPx + dx));
    const py = Math.max(0, Math.min(maxY, d.startPy + dy));
    const el = tileEls.current.get(d.id);
    if (el) el.style.transform = `translate3d(${px}px, ${py}px, 0) scale(1.03)`;
    const tx = Math.max(0, Math.min(WIDGET_COLS - d.w, Math.round(px / (geo.cell + GAP))));
    const ty = Math.max(0, Math.min(geo.rows - d.h, Math.round(py / (geo.cell + GAP))));
    if (d.target && d.target.x === tx && d.target.y === ty) return;
    const next = resolveMove(display.placed, d.id, { x: tx, y: ty, w: d.w, h: d.h }, WIDGET_COLS, geo.rows);
    if (next) {
      d.target = { x: tx, y: ty };
      setPreview(next);
    }
  };

  const endDrag = (e: React.PointerEvent<HTMLElement>) => {
    const p = press.current;
    if (p) {
      window.clearTimeout(p.timer);
      press.current = null;
    }
    const d = drag.current;
    if (!d || e.pointerId !== d.pointerId) return;
    drag.current = null;
    const el = tileEls.current.get(d.id);
    // The tile glides from the pointer into its cell (transform set by render).
    if (el) el.style.transform = '';
    setDragId(null);
    if (d.moved && preview) commitCells(preview);
    setPreview(null);
  };

  // Keyboard: arrows move the selected widget a cell; Enter opens its settings.
  const onTileKeyDown = (e: React.KeyboardEvent, id: string) => {
    if (!editing) return;
    const c = cellOf.get(id);
    if (!c) return;
    const moves: Record<string, [number, number]> = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
    const mv = moves[e.key];
    if (mv) {
      e.preventDefault();
      const next = resolveMove(display.placed, id, { x: c.x + mv[0], y: c.y + mv[1], w: c.w, h: c.h }, WIDGET_COLS, geo.rows);
      if (next) commitCells(next);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      setEditorId(id);
    } else if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      remove(id);
    }
  };

  const gallery = usePresence(galleryOpen, 160);
  const editorItem = editorId ? byId.get(editorId) : undefined;
  const editorPresence = usePresence(!!editorItem, 160);
  const [lastEditor, setLastEditor] = useState<WidgetItem | undefined>(undefined);
  useEffect(() => {
    if (editorItem) setLastEditor(editorItem);
  }, [editorItem]);

  const gridHeight = geo.rows > 0 ? cellPx(geo.rows, geo.cell) - GAP : 0;
  const ghost = dragId && preview ? preview.find((c) => c.id === dragId) : null;

  return (
    <div
      ref={boardRef}
      className={`wboard2${editing ? ' editing' : ''}${dragId ? ' dragging' : ''}`}
      style={{ '--cell': `${geo.cell}px`, '--gap': `${GAP}px`, '--wscale': scale } as React.CSSProperties}
    >
      <div className="wgrid2" style={{ left: geo.left, width: geo.width, height: gridHeight }}>
        {editing && (
          <div className="wgrid2-cells" aria-hidden="true">
            {Array.from({ length: geo.rows * WIDGET_COLS }, (_, i) => (
              <span key={i} style={{ transform: `translate3d(${cellPx(i % WIDGET_COLS, geo.cell)}px, ${cellPx(Math.floor(i / WIDGET_COLS), geo.cell)}px, 0)` }} />
            ))}
          </div>
        )}
        {ghost && (
          <div
            className="wghost"
            aria-hidden="true"
            style={{ width: spanPx(ghost.w, geo.cell), height: spanPx(ghost.h, geo.cell), transform: `translate3d(${cellPx(ghost.x, geo.cell)}px, ${cellPx(ghost.y, geo.cell)}px, 0)` }}
          />
        )}
        {items.map((item) => {
          const c = cellOf.get(item.id);
          const meta = widgetMeta(item.kind);
          if (!c || !meta) return null;
          const ctx: WidgetContext = {
            item,
            setConfig: (patch) => setConfig(item.id, patch),
            note: widgets.notes[item.id] ?? '',
            setNote: (text) => onUpdate({ ...widgets, notes: { ...widgets.notes, [item.id]: text } }),
            open: onOpen,
            editing,
            edit: () => {
              onEditingChange(true);
              setEditorId(item.id);
            },
          };
          const dragging = dragId === item.id;
          return (
            <div
              key={item.id}
              ref={(el) => {
                if (el) tileEls.current.set(item.id, el);
                else tileEls.current.delete(item.id);
              }}
              data-widget-id={item.id}
              className={`wtile size-${item.size} kind-${item.kind.replace(':', '-')}${dragging ? ' dragging' : ''}${selectedId === item.id && editing ? ' selected' : ''}`}
              style={{
                width: spanPx(c.w, geo.cell),
                height: spanPx(c.h, geo.cell),
                transform: dragging ? undefined : `translate3d(${cellPx(c.x, geo.cell)}px, ${cellPx(c.y, geo.cell)}px, 0)`,
              }}
              tabIndex={editing ? 0 : -1}
              aria-label={meta.name}
              onPointerDown={(e) => onTilePointerDown(e, item.id)}
              onPointerMove={onTilePointerMove}
              onPointerUp={endDrag}
              onPointerCancel={endDrag}
              onLostPointerCapture={endDrag}
              onKeyDown={(e) => onTileKeyDown(e, item.id)}
              onFocus={() => editing && setSelectedId(item.id)}
            >
              <div className="wtile-face">
                <WidgetView ctx={ctx} hour12={widgets.clock.hour12} />
              </div>
              {editing && (
                <div className="wtile-chrome">
                  <button type="button" className="wtile-remove" aria-label={`Remove ${meta.name}`} title="Remove" onClick={() => remove(item.id)}>
                    <Minus size={12} strokeWidth={3} />
                  </button>
                  <button type="button" className="wtile-edit" aria-label={`${meta.name} settings`} title={`Size${meta.configurable ? ' and settings' : ''}`} onClick={() => setEditorId(item.id)}>
                    <SlidersHorizontal size={12} />
                  </button>
                </div>
              )}
            </div>
          );
        })}
      </div>

      {editing && (
        <div className="wboard-bar2" role="toolbar" aria-label="Editing widgets">
          {display.hidden.length > 0 && (
            <span className="wboard-note">
              {display.hidden.length} {display.hidden.length === 1 ? 'widget doesn’t' : 'widgets don’t'} fit this window
            </span>
          )}
          <button type="button" className="btn wboard-add-btn" disabled={!canAddAny} title={canAddAny ? undefined : 'No room: move, shrink or remove a widget'} onClick={() => onGalleryOpenChange(true)}>
            <Plus size={14} aria-hidden="true" />
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

      {gallery.mounted && (
        <div className="presence" data-state={gallery.state}>
          <div
            className="modal-overlay wgallery-overlay"
            onMouseDown={(e) => {
              if (e.target === e.currentTarget) onGalleryOpenChange(false);
            }}
            onKeyDown={(e) => {
              if (e.key === 'Escape') {
                e.stopPropagation();
                onGalleryOpenChange(false);
              }
            }}
          >
            <div className="sheet wgallery" role="dialog" aria-modal="true" aria-label="Add a widget">
              <header className="wed-head">
                <div className="wed-titles">
                  <h2 className="wed-title">Add a widget</h2>
                  <p className="wed-sub">It goes in the first free place; drag it where you like.</p>
                </div>
                <button type="button" className="btn btn-quiet btn-icon" aria-label="Close" autoFocus onClick={() => onGalleryOpenChange(false)}>
                  <X size={16} />
                </button>
              </header>
              <div className="wgallery-body">
                {(
                  [
                    ['Serica', BUILTIN_KINDS],
                    ['Apps', APP_KINDS],
                  ] as const
                ).map(([title, kinds]) => (
                  <section key={title} className="wgallery-section">
                    <h3 className="settings2-group-title">{title}</h3>
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
                            title={size ? `Adds a ${SIZE_LABEL[size].toLowerCase()} widget` : 'No room left: move, shrink or remove a widget'}
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
        </div>
      )}

      {editorPresence.mounted && (editorItem ?? lastEditor) && (
        <div className="presence" data-state={editorPresence.state}>
          <WidgetEditor
            item={(editorItem ?? lastEditor)!}
            canResize={(s) => canResize((editorItem ?? lastEditor)!.id, s)}
            onResize={(s) => resize((editorItem ?? lastEditor)!.id, s)}
            setConfig={(patch) => setConfig((editorItem ?? lastEditor)!.id, patch)}
            onRemove={() => remove((editorItem ?? lastEditor)!.id)}
            onClose={() => setEditorId(null)}
          />
        </div>
      )}

      {guide && <GuideTour id="widgets" steps={WIDGET_GUIDE} onClose={closeGuide} />}
    </div>
  );
};
