import React, { useEffect, useMemo, useState } from 'react';
import {
  Archive,
  Download,
  File,
  FileAudio,
  FileCode,
  FileImage,
  FileText,
  FileVideo,
  FolderOpen,
  Pause,
  Play,
  RotateCw,
  Search,
  X,
} from 'lucide-react';
import type { DownloadAction, DownloadItem } from '../types';
import { describeDownload, isActiveDownload } from '../hooks/useDownloads';

interface DownloadsPanelProps {
  isOpen: boolean;
  onClose: () => void;
  downloads: DownloadItem[];
}

function iconFor(d: DownloadItem) {
  const ext = d.filename.split('.').pop()?.toLowerCase() ?? '';
  const mime = d.mime.toLowerCase();
  if (mime.startsWith('image/') || ['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp', 'ico', 'avif'].includes(ext)) return FileImage;
  if (mime.startsWith('video/') || ['mp4', 'mkv', 'webm', 'mov', 'avi'].includes(ext)) return FileVideo;
  if (mime.startsWith('audio/') || ['mp3', 'wav', 'flac', 'ogg', 'm4a', 'aac'].includes(ext)) return FileAudio;
  if (['zip', 'rar', '7z', 'tar', 'gz', 'xz', 'bz2', 'iso', 'msi', 'exe'].includes(ext)) return Archive;
  if (['js', 'ts', 'json', 'html', 'css', 'py', 'rs', 'xml', 'sh', 'ps1'].includes(ext)) return FileCode;
  if (mime.startsWith('text/') || ['pdf', 'txt', 'md', 'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'csv', 'epub'].includes(ext)) return FileText;
  return File;
}

function dayLabel(ts: number, now = new Date()): string {
  const d = new Date(ts);
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  if (ts >= start) return 'Today';
  if (ts >= start - 86_400_000) return 'Yesterday';
  return d.toLocaleDateString([], { month: 'short', day: 'numeric', year: d.getFullYear() === now.getFullYear() ? undefined : 'numeric' });
}

export const DownloadsPanel: React.FC<DownloadsPanelProps> = ({ isOpen, onClose, downloads }) => {
  const [query, setQuery] = useState('');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isOpen) {
      setQuery('');
      setError(null);
    }
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [isOpen, onClose]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return downloads;
    return downloads.filter((d) => `${d.filename}\n${d.url}`.toLowerCase().includes(q));
  }, [downloads, query]);

  const groups = useMemo(() => {
    const out: { label: string; items: DownloadItem[] }[] = [];
    const now = new Date();
    for (const d of filtered) {
      const label = isActiveDownload(d) ? 'In progress' : dayLabel(d.startedAt, now);
      const last = out[out.length - 1];
      if (last && last.label === label) last.items.push(d);
      else out.push({ label, items: [d] });
    }
    return out;
  }, [filtered]);

  const hasFinished = downloads.some((d) => !isActiveDownload(d));

  const act = (d: DownloadItem, action: DownloadAction) => {
    setError(null);
    window.serica?.downloadAction?.(d.id, action).catch((e) => setError(String(e)));
    if (action === 'retry') onClose();
  };

  if (!isOpen) return null;

  return (
    <div className="modal-overlay history-modal-overlay" onClick={onClose} role="dialog" aria-modal="true" aria-label="Downloads">
      <div className="modal-content-card history-modal-card downloads-modal-card" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <div className="modal-title">
            <Download size={18} />
            <span>Downloads</span>
            {downloads.length > 0 && <span className="history-count-badge">{downloads.length}</span>}
          </div>
          <div className="downloads-header-actions">
            <button type="button" className="history-clear-chip" onClick={() => void window.serica?.openDownloadsFolder?.()} title="Open the Downloads folder">
              <FolderOpen size={12} /> Folder
            </button>
            <button type="button" className="history-clear-chip" disabled={!hasFinished} onClick={() => void window.serica?.clearDownloads?.()} title="Clear finished downloads from the list (files are kept)">
              Clear
            </button>
            <button className="close-btn" onClick={onClose} aria-label="Close downloads" title="Close (Esc)">
              <X size={16} />
            </button>
          </div>
        </div>

        {downloads.length > 6 && (
          <div className="history-search-box">
            <Search size={14} className="history-search-icon" />
            <input
              type="text"
              className="history-search-input"
              placeholder="Search downloads..."
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              aria-label="Search downloads"
            />
          </div>
        )}

        {error && <div className="extensions-message error">{error}</div>}

        <div className="history-list-scroll">
          {filtered.length === 0 ? (
            <div className="history-empty-state">
              <Download size={24} style={{ opacity: 0.45 }} />
              <div style={{ fontWeight: 600, fontSize: '13px' }}>{downloads.length === 0 ? 'No downloads yet' : 'No matching downloads'}</div>
              {downloads.length === 0 && <div style={{ fontSize: '11.5px', opacity: 0.7 }}>Files you download appear here (Ctrl+J).</div>}
            </div>
          ) : (
            groups.map((g) => (
              <div key={g.label} className="history-bucket">
                <div className="history-bucket-header">{g.label}</div>
                {g.items.map((d) => {
                  const Icon = iconFor(d);
                  const active = isActiveDownload(d);
                  const openable = d.state === 'completed' && d.fileExists;
                  const pct = d.totalBytes > 0 ? Math.min(100, (d.receivedBytes / d.totalBytes) * 100) : null;
                  return (
                    <div
                      key={d.id}
                      className={`download-row state-${d.state}${openable ? ' openable' : ''}${d.state === 'completed' && !d.fileExists ? ' missing' : ''}`}
                      role={openable ? 'button' : undefined}
                      tabIndex={openable ? 0 : undefined}
                      title={openable ? `Open ${d.filename}` : d.url}
                      onClick={() => openable && act(d, 'open')}
                      onKeyDown={(e) => {
                        if (openable && (e.key === 'Enter' || e.key === ' ')) {
                          e.preventDefault();
                          act(d, 'open');
                        }
                      }}
                    >
                      <div className="download-icon-wrap">
                        <Icon size={17} />
                      </div>
                      <div className="history-row-main">
                        <div className="history-row-title">{d.filename}</div>
                        <div className="download-status">{describeDownload(d)}</div>
                        {active && (
                          <div className={`download-progress${pct === null ? ' indeterminate' : ''}${d.state === 'paused' ? ' paused' : ''}`}>
                            <div className="download-progress-fill" style={pct === null ? undefined : { width: `${pct}%` }} />
                          </div>
                        )}
                      </div>
                      <div className="download-actions" onClick={(e) => e.stopPropagation()}>
                        {d.state === 'inProgress' && (
                          <button type="button" className="history-row-action-btn" title="Pause" aria-label={`Pause ${d.filename}`} onClick={() => act(d, 'pause')}>
                            <Pause size={13} />
                          </button>
                        )}
                        {(d.state === 'paused' || (d.state === 'failed' && d.canResume)) && (
                          <button type="button" className="history-row-action-btn" title="Resume" aria-label={`Resume ${d.filename}`} onClick={() => act(d, 'resume')}>
                            <Play size={13} />
                          </button>
                        )}
                        {(d.state === 'failed' || d.state === 'canceled') && !d.canResume && (
                          <button type="button" className="history-row-action-btn" title="Retry" aria-label={`Retry ${d.filename}`} onClick={() => act(d, 'retry')}>
                            <RotateCw size={13} />
                          </button>
                        )}
                        {d.state === 'completed' && (
                          <button type="button" className="history-row-action-btn" title="Show in folder" aria-label={`Show ${d.filename} in folder`} onClick={() => act(d, 'show')}>
                            <FolderOpen size={13} />
                          </button>
                        )}
                        {active || (d.state === 'failed' && d.canResume) ? (
                          <button type="button" className="history-row-action-btn delete" title="Cancel" aria-label={`Cancel ${d.filename}`} onClick={() => act(d, 'cancel')}>
                            <X size={13} />
                          </button>
                        ) : (
                          <button type="button" className="history-row-action-btn delete" title="Remove from list" aria-label={`Remove ${d.filename} from list`} onClick={() => act(d, 'remove')}>
                            <X size={13} />
                          </button>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
};
