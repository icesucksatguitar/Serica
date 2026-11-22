import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { ExternalLink, Loader2, Plus, Puzzle, Search, Settings2, Store, Trash2, X } from 'lucide-react';
import type { ExtensionInfo } from '../types';

export const CHROME_WEB_STORE_URL = 'https://chromewebstore.google.com/';

interface ExtensionsModalProps {
  isOpen: boolean;
  onClose: () => void;
  /** Opens a URL in a new tab and closes the panel. */
  onOpenUrl: (url: string) => void;
}

const ExtensionIcon: React.FC<{ src?: string | null }> = ({ src }) => {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [src]);
  if (!src || failed) return <Puzzle size={18} style={{ opacity: 0.72 }} />;
  return <img src={src} alt="" className="extension-icon-img" onError={() => setFailed(true)} />;
};

export const ExtensionsModal: React.FC<ExtensionsModalProps> = ({ isOpen, onClose, onOpenUrl }) => {
  const [extensions, setExtensions] = useState<ExtensionInfo[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [query, setQuery] = useState('');
  const [installInput, setInstallInput] = useState('');
  const [installing, setInstalling] = useState(false);
  const [message, setMessage] = useState<{ kind: 'error' | 'ok'; text: string } | null>(null);
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);

  useEffect(() => {
    if (!isOpen) return;
    let alive = true;
    void window.neutrino?.listExtensions?.().then((list) => {
      if (!alive) return;
      setExtensions(list ?? []);
      setLoaded(true);
    }).catch(() => setLoaded(true));
    const unsub = window.neutrino?.onExtensionsUpdated?.((list) => setExtensions(list ?? []));
    return () => {
      alive = false;
      unsub?.();
    };
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) {
      setQuery('');
      setInstallInput('');
      setMessage(null);
      setConfirmRemove(null);
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
    if (!q) return extensions;
    return extensions.filter((x) => `${x.name}\n${x.description}\n${x.id}`.toLowerCase().includes(q));
  }, [extensions, query]);

  const install = useCallback(async () => {
    const input = installInput.trim();
    if (!input || installing) return;
    setInstalling(true);
    setMessage(null);
    try {
      const rec = await window.neutrino?.installExtension?.(input);
      if (rec) {
        setMessage({ kind: 'ok', text: `${rec.name} was added.` });
        setInstallInput('');
      }
    } catch (e) {
      setMessage({ kind: 'error', text: String(e) });
    } finally {
      setInstalling(false);
    }
  }, [installInput, installing]);

  const toggle = (x: ExtensionInfo) => {
    setExtensions((list) => list.map((e) => (e.id === x.id ? { ...e, enabled: !x.enabled } : e)));
    void window.neutrino?.setExtensionEnabled?.(x.id, !x.enabled);
  };

  const remove = (x: ExtensionInfo) => {
    setConfirmRemove(null);
    setExtensions((list) => list.filter((e) => e.id !== x.id));
    void window.neutrino?.removeExtension?.(x.id);
  };

  const openPage = (x: ExtensionInfo, page: 'popup' | 'options') => {
    void window.neutrino?.openExtensionPage?.(x.id, page);
    onClose();
  };

  if (!isOpen) return null;

  return (
    <div className="modal-overlay history-modal-overlay" onClick={onClose} role="dialog" aria-modal="true" aria-label="Extensions">
      <div className="modal-content-card history-modal-card extensions-modal-card" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <div className="modal-title">
            <Puzzle size={18} />
            <span>Extensions</span>
            <span className="history-count-badge">{extensions.length}</span>
          </div>
          <button className="close-btn" onClick={onClose} aria-label="Close extensions" title="Close (Esc)">
            <X size={16} />
          </button>
        </div>

        <button type="button" className="extensions-store-cta" onClick={() => onOpenUrl(CHROME_WEB_STORE_URL)}>
          <Store size={16} />
          <span className="extensions-store-cta-text">
            <strong>Browse the Chrome Web Store</strong>
            <span>Open an extension and press “Add to Neutrino”.</span>
          </span>
          <ExternalLink size={14} className="extensions-store-cta-arrow" />
        </button>

        <form
          className="history-search-row"
          onSubmit={(e) => {
            e.preventDefault();
            void install();
          }}
        >
          <div className="history-search-box">
            <Plus size={14} className="history-search-icon" />
            <input
              type="text"
              className="history-search-input"
              placeholder="Paste a Web Store link or extension ID"
              value={installInput}
              onChange={(e) => setInstallInput(e.target.value)}
              aria-label="Chrome Web Store link or extension ID"
              disabled={installing}
            />
          </div>
          <button type="submit" className="history-clear-chip extensions-install-btn" disabled={!installInput.trim() || installing}>
            {installing ? <Loader2 size={13} className="extensions-spin" /> : 'Install'}
          </button>
        </form>
        {message && <div className={`extensions-message ${message.kind}`}>{message.text}</div>}

        {extensions.length > 4 && (
          <div className="history-search-box">
            <Search size={14} className="history-search-icon" />
            <input
              type="text"
              className="history-search-input"
              placeholder="Search installed extensions..."
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              aria-label="Search installed extensions"
            />
          </div>
        )}

        <div className="history-list-scroll">
          {filtered.length === 0 ? (
            <div className="history-empty-state">
              <Puzzle size={24} style={{ opacity: 0.45 }} />
              <div style={{ fontWeight: 600, fontSize: '13px' }}>
                {!loaded ? 'Loading…' : extensions.length === 0 ? 'No extensions yet' : 'No matching extensions'}
              </div>
              {loaded && extensions.length === 0 && (
                <div style={{ fontSize: '11.5px', opacity: 0.7 }}>Extensions you add from the Chrome Web Store appear here.</div>
              )}
            </div>
          ) : (
            filtered.map((x) => (
              <div key={x.id} className={`extension-row${x.enabled ? '' : ' disabled'}`} title={x.id}>
                <div className="extension-icon-wrap">
                  <ExtensionIcon src={x.icon} />
                </div>
                <div className="history-row-main">
                  <div className="history-row-title">
                    {x.name}
                    <span className="extension-version">{x.version}</span>
                  </div>
                  {x.description && <div className="extension-desc">{x.description}</div>}
                  <div className="extension-actions">
                    {x.popup && (
                      <button type="button" className="history-clear-chip" disabled={!x.enabled} onClick={() => openPage(x, 'popup')}>
                        Open
                      </button>
                    )}
                    {x.options && (
                      <button type="button" className="history-clear-chip" disabled={!x.enabled} onClick={() => openPage(x, 'options')}>
                        <Settings2 size={11} /> Options
                      </button>
                    )}
                    <button
                      type="button"
                      className="history-clear-chip"
                      onClick={() => onOpenUrl(`${CHROME_WEB_STORE_URL}detail/${x.id}`)}
                      title="View in Chrome Web Store"
                    >
                      Details
                    </button>
                    {confirmRemove === x.id ? (
                      <button type="button" className="history-clear-chip danger confirm" onClick={() => remove(x)}>
                        Confirm remove
                      </button>
                    ) : (
                      <button type="button" className="history-clear-chip danger" onClick={() => setConfirmRemove(x.id)} aria-label={`Remove ${x.name}`}>
                        <Trash2 size={11} /> Remove
                      </button>
                    )}
                  </div>
                </div>
                <label className="extension-switch" title={x.enabled ? 'Disable' : 'Enable'}>
                  <input type="checkbox" checked={x.enabled} onChange={() => toggle(x)} aria-label={`${x.enabled ? 'Disable' : 'Enable'} ${x.name}`} />
                  <span className="extension-switch-track" />
                </label>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
};
