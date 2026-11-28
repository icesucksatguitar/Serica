import { useEffect, useRef, useState } from 'react';
import type { DownloadItem } from '../types';

/** Live download list (newest first), kept in sync with the backend. */
export function useDownloads(onStarted?: (item: DownloadItem) => void) {
  const [downloads, setDownloads] = useState<DownloadItem[]>([]);
  const onStartedRef = useRef(onStarted);
  onStartedRef.current = onStarted;

  useEffect(() => {
    let alive = true;
    void window.serica?.listDownloads?.().then((list) => {
      if (alive && list) setDownloads(list);
    });
    const upsert = (item: DownloadItem) =>
      setDownloads((list) => {
        const i = list.findIndex((d) => d.id === item.id);
        if (i < 0) return [item, ...list];
        const next = list.slice();
        next[i] = item;
        return next;
      });
    const unsubs = [
      window.serica?.onDownloadStarted?.((item) => {
        upsert(item);
        onStartedRef.current?.(item);
      }),
      window.serica?.onDownloadUpdated?.(upsert),
      window.serica?.onDownloadsChanged?.((list) => setDownloads(list ?? [])),
    ];
    return () => {
      alive = false;
      unsubs.forEach((u) => u?.());
    };
  }, []);

  return downloads;
}

export function isActiveDownload(d: DownloadItem): boolean {
  return d.state === 'inProgress' || d.state === 'paused';
}

/** Combined progress (0..1) of active downloads with a known size, or null. */
export function aggregateProgress(list: DownloadItem[]): number | null {
  const active = list.filter((d) => isActiveDownload(d) && d.totalBytes > 0);
  if (active.length === 0) return null;
  const total = active.reduce((s, d) => s + d.totalBytes, 0);
  const got = active.reduce((s, d) => s + Math.min(d.receivedBytes, d.totalBytes), 0);
  return total > 0 ? got / total : null;
}

export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return '?';
  if (n < 1024) return `${n} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = n / 1024;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u++;
  }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${units[u]}`;
}

export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '';
  const s = Math.round(seconds);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

/** One-line status under a download's name. */
export function describeDownload(d: DownloadItem): string {
  const size = d.totalBytes > 0 ? formatBytes(d.totalBytes) : null;
  switch (d.state) {
    case 'inProgress': {
      const parts = [size ? `${formatBytes(d.receivedBytes)} of ${size}` : formatBytes(d.receivedBytes)];
      if (d.bytesPerSec > 0) {
        parts.push(`${formatBytes(d.bytesPerSec)}/s`);
        if (d.totalBytes > 0) {
          const left = formatDuration((d.totalBytes - d.receivedBytes) / d.bytesPerSec);
          if (left) parts.push(`${left} left`);
        }
      } else if (d.receivedBytes === 0) {
        parts[0] = 'Starting…';
      }
      return parts.join(' · ');
    }
    case 'paused':
      return `Paused · ${size ? `${formatBytes(d.receivedBytes)} of ${size}` : formatBytes(d.receivedBytes)}`;
    case 'completed':
      return d.fileExists ? `${size ?? formatBytes(d.receivedBytes)} · ${hostOf(d.url)}` : 'Deleted';
    case 'canceled':
      return 'Canceled';
    case 'failed':
      return `Failed${d.error ? ` · ${d.error}` : ''}`;
  }
}

export function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '');
  } catch {
    return '';
  }
}
