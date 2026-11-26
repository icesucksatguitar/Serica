import React, { useEffect, useRef, useState } from 'react';
import { FileUp, Link, Search } from 'lucide-react';
import type { DragEventPayload } from '../adapter/tauriBridge';

type Zone = { id: 'sidebar' | 'strip' | 'here' | 'window'; rect: DOMRect; label: string };

interface DropZonesProps {
  /** The active tab shows the new-tab page (a drop there opens in it). */
  onNewTabPage: boolean;
  spaceName?: string;
  onOpenInNewTab: (url: string) => void;
  onOpenHere: (url: string) => void;
}

function zoneAt(x: number, y: number, props: DropZonesProps): Zone {
  const sidebar = document.querySelector('.nsb.visible') as HTMLElement | null;
  if (sidebar) {
    const r = sidebar.getBoundingClientRect();
    if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) {
      return { id: 'sidebar', rect: r, label: `Open in ${props.spaceName ?? 'this space'}` };
    }
  }
  const strip = document.querySelector('.top-tab-strip') as HTMLElement | null;
  if (strip) {
    const r = strip.getBoundingClientRect();
    if (y >= r.top && y <= r.bottom) return { id: 'strip', rect: r, label: 'Open in a new tab' };
  }
  const home = document.querySelector('.home2') as HTMLElement | null;
  if (props.onNewTabPage && home) {
    const r = home.getBoundingClientRect();
    if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) return { id: 'here', rect: r, label: 'Open here' };
  }
  return {
    id: 'window',
    rect: new DOMRect(8, 8, window.innerWidth - 16, window.innerHeight - 16),
    label: 'Open in a new tab',
  };
}

/**
 * Shows where something dragged over Serica (files from Explorer, a link out
 * of a page, selected text) will land, and opens it on drop: the new-tab page
 * opens it in place, anywhere else in a new tab. Pages take their own drops.
 */
export const DropZones: React.FC<DropZonesProps> = (props) => {
  const [drag, setDrag] = useState<{ data: NonNullable<DragEventPayload['data']>; zone: Zone } | null>(null);
  const propsRef = useRef(props);
  propsRef.current = props;

  useEffect(() => {
    let frame = 0;
    let latest: DragEventPayload | null = null;
    const apply = () => {
      frame = 0;
      const e = latest;
      if (!e) return;
      if (e.phase === 'leave' || !e.data) {
        setDrag(null);
        return;
      }
      const k = e.clientWidth > 0 ? window.innerWidth / e.clientWidth : 1;
      const zone = zoneAt(e.x * k, e.y * k, propsRef.current);
      if (e.phase === 'drop') {
        setDrag(null);
        const p = propsRef.current;
        const d = e.data;
        if (d.kind === 'files') {
          if (zone.id === 'here' && d.paths.length === 1) p.onOpenHere(d.paths[0]);
          else void window.serica?.openDroppedPaths?.(d.paths);
        } else {
          const target = (d.kind === 'url' ? d.url : d.text) ?? '';
          if (!target.trim()) return;
          if (zone.id === 'here') p.onOpenHere(target);
          else p.onOpenInNewTab(target);
        }
        return;
      }
      setDrag((cur) => (cur && cur.zone.id === zone.id && cur.data === e.data ? cur : { data: e.data!, zone }));
    };
    // Drag events arrive at the OS's pointer rate: handle one per frame.
    const off = window.serica?.onDrag?.((e: DragEventPayload) => {
      latest = e;
      if (e.phase === 'drop' || e.phase === 'leave') {
        if (frame) cancelAnimationFrame(frame);
        apply();
      } else if (!frame) {
        frame = requestAnimationFrame(apply);
      }
    });
    return () => {
      off?.();
      if (frame) cancelAnimationFrame(frame);
    };
  }, []);

  const [visible, setVisible] = useState(false);
  useEffect(() => {
    if (!drag) {
      setVisible(false);
      return;
    }
    const r = requestAnimationFrame(() => setVisible(true));
    return () => cancelAnimationFrame(r);
  }, [!!drag]);

  if (!drag) return null;
  const { zone, data } = drag;
  const Icon = data.kind === 'files' ? FileUp : data.kind === 'url' ? Link : Search;
  const what =
    data.kind === 'files'
      ? data.paths.length === 1
        ? data.paths[0].split(/[\\/]/).pop()
        : `${data.paths.length} files`
      : data.kind === 'url'
        ? (data.url ?? '').replace(/^https?:\/\//, '').slice(0, 60)
        : `Search “${(data.text ?? '').slice(0, 40)}”`;
  return (
    <div className="drop-veil" aria-hidden="true">
      <div
        className={`drop-zone hot${visible ? ' on' : ''}`}
        style={{ left: zone.rect.left + 4, top: zone.rect.top + 4, width: zone.rect.width - 8, height: zone.rect.height - 8 }}
      >
        <span className="drop-label">
          <Icon size={14} />
          <span>
            {zone.label} · {what}
          </span>
        </span>
      </div>
    </div>
  );
};

export default DropZones;
