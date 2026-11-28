// The toolbar of a tab in a window of its own (see src-tauri/src/detached.rs).
// This is the whole UI of that window: the page itself is a native view
// placed right under it. Back, forward, reload, the address (editable), and
// a button that moves the tab back into Serica's main window.
import React, { useEffect, useRef, useState } from 'react';
import { ArrowLeft, ArrowRight, Globe, Lock, RotateCw, X, Undo2 } from 'lucide-react';
import type { BrowserAppState, TabState } from '../types';
import { installDragRegions } from '../hooks/useMacWindow';
import '../styles/detached.css';

function shownUrl(url: string): string {
  try {
    const u = new URL(url);
    return (u.host + u.pathname + u.search + u.hash).replace(/\/$/, '');
  } catch {
    return url;
  }
}

export const DetachedWindow: React.FC<{ tabId: string }> = ({ tabId }) => {
  const [tab, setTab] = useState<TabState | null>(null);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const api = window.serica;
    const pick = (s: BrowserAppState | null | undefined) => {
      const t = s?.tabs?.find((x) => x.id === tabId);
      if (t) setTab(t);
    };
    void api?.getState?.().then(pick).catch(() => null);
    return api?.onStateUpdate?.(pick);
  }, [tabId]);

  // The toolbar is the window's title bar: it drags the window, and a
  // double-click zooms or minimizes it (System Settings decides).
  useEffect(() => installDragRegions(), []);

  useEffect(() => {
    if (tab) document.title = tab.title || shownUrl(tab.url);
  }, [tab?.title, tab?.url]);

  const api = window.serica;
  const act = (a: 'back' | 'forward' | 'reload' | 'stop') => void api?.tabAction?.(tabId, a);
  const startEdit = () => {
    setDraft(tab?.url ?? '');
    setEditing(true);
    requestAnimationFrame(() => inputRef.current?.select());
  };
  const submit = () => {
    const v = draft.trim();
    setEditing(false);
    if (v) void api?.navigateTab?.(tabId, v);
  };

  const secure = tab?.url.startsWith('https://');
  return (
    <div className="dwin-bar">
      <button type="button" className="dwin-btn" title="Back (⌘[)" aria-label="Back" disabled={!tab?.canGoBack} onClick={() => act('back')}>
        <ArrowLeft size={16} strokeWidth={1.75} />
      </button>
      <button type="button" className="dwin-btn" title="Forward (⌘])" aria-label="Forward" disabled={!tab?.canGoForward} onClick={() => act('forward')}>
        <ArrowRight size={16} strokeWidth={1.75} />
      </button>
      <button
        type="button"
        className="dwin-btn"
        title={tab?.isLoading ? 'Stop' : 'Reload (⌘R)'}
        aria-label={tab?.isLoading ? 'Stop' : 'Reload'}
        onClick={() => act(tab?.isLoading ? 'stop' : 'reload')}
      >
        {tab?.isLoading ? <X size={16} strokeWidth={1.75} /> : <RotateCw size={15} strokeWidth={1.75} />}
      </button>
      <div className={`dwin-address${editing ? ' editing' : ''}`} onClick={() => !editing && startEdit()}>
        {secure ? <Lock size={12} strokeWidth={2} aria-hidden="true" /> : <Globe size={12} strokeWidth={2} aria-hidden="true" />}
        {editing ? (
          <input
            ref={inputRef}
            aria-label="Address"
            value={draft}
            spellCheck={false}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={() => setEditing(false)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') submit();
              if (e.key === 'Escape') setEditing(false);
            }}
          />
        ) : (
          <span className="dwin-url" title={tab?.url}>
            {tab ? shownUrl(tab.url) : ''}
          </span>
        )}
      </div>
      <button type="button" className="dwin-back" title="Move this tab back into the Serica window" onClick={() => void api?.reattachTab?.(tabId)}>
        <Undo2 size={14} strokeWidth={1.9} aria-hidden="true" />
        <span>Back to Serica</span>
      </button>
    </div>
  );
};
