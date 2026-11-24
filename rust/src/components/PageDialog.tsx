import React, { useEffect, useRef, useState } from 'react';
import { ExternalLink, LogOut, MessageSquare } from 'lucide-react';
import type { PageDialogRequest } from '../types';
import { useViewportOverlay, useViewportCovered } from '../hooks/useViewportOverlay';

interface PageDialogProps {
  activeTabId: string | null | undefined;
  /** Ids of open tabs: dialogs of a closed tab are cancelled. */
  tabIds: string[];
}

const answer = (id: number, accept: boolean, text?: string) => {
  void window.serica?.answerPageDialog?.(id, accept, text);
};

/** Words shown for each dialog kind. */
function copy(d: PageDialogRequest) {
  switch (d.kind) {
    case 'beforeunload':
      return { title: 'Leave site?', body: 'Changes you made may not be saved.', ok: 'Leave', cancel: 'Stay', icon: LogOut };
    case 'external':
      return {
        title: 'Open in another app?',
        body: `${d.origin || 'This site'} wants to open a ${d.scheme ?? ''} link outside Serica.`,
        ok: 'Open app',
        cancel: 'Cancel',
        icon: ExternalLink,
      };
    case 'alert':
      return { title: `${d.origin || 'This page'} says`, body: d.message, ok: 'OK', cancel: null, icon: MessageSquare };
    default:
      return { title: `${d.origin || 'This page'} says`, body: d.message, ok: 'OK', cancel: 'Cancel', icon: MessageSquare };
  }
}

/**
 * Serica-styled replacement for WebView2's alert/confirm/prompt, "Leave site?"
 * and "Open in app?" dialogs. A dialog from a background tab waits until that
 * tab is shown, like in Chrome.
 */
export const PageDialog: React.FC<PageDialogProps> = ({ activeTabId, tabIds }) => {
  const [queue, setQueue] = useState<PageDialogRequest[]>([]);
  const [text, setText] = useState('');
  const okRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => window.serica?.onPageDialog?.((d) => setQueue((q) => [...q, d])), []);

  // A closed tab's dialogs are answered "cancel" so the page never hangs.
  const tabKey = tabIds.join('|');
  useEffect(() => {
    const open = new Set(tabIds);
    setQueue((q) => {
      const gone = q.filter((d) => !open.has(d.tabId));
      if (!gone.length) return q;
      gone.forEach((d) => answer(d.id, false));
      return q.filter((d) => open.has(d.tabId));
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tabKey]);

  const current = queue.find((d) => d.tabId === activeTabId) ?? null;
  useViewportOverlay(!!current);
  const covered = useViewportCovered();

  useEffect(() => {
    if (!current) return;
    setText(current.defaultText ?? '');
    const t = window.setTimeout(() => {
      if (current.kind === 'prompt') inputRef.current?.select();
      else okRef.current?.focus();
    }, 30);
    return () => window.clearTimeout(t);
  }, [current]);

  if (!current) return null;
  const c = copy(current);
  const Icon = c.icon;

  const close = (accept: boolean) => {
    answer(current.id, accept, current.kind === 'prompt' && accept ? text : undefined);
    setQueue((q) => q.filter((d) => d.id !== current.id));
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      close(current.kind === 'alert');
    } else if (e.key === 'Enter' && !e.shiftKey && !(e.target instanceof HTMLButtonElement)) {
      // A focused button answers Enter itself (Cancel/Stay must not accept).
      e.preventDefault();
      close(true);
    }
  };

  return (
    <div className={`page-dialog-scrim${covered ? ' visible' : ''}`} onMouseDown={(e) => e.target === e.currentTarget && e.preventDefault()}>
      <div
        className="page-dialog"
        role={current.kind === 'alert' ? 'alertdialog' : 'dialog'}
        aria-modal="true"
        aria-labelledby="page-dialog-title"
        aria-describedby="page-dialog-body"
        onKeyDown={onKeyDown}
      >
        <div className="page-dialog-head">
          <span className={`page-dialog-icon kind-${current.kind}`} aria-hidden="true">
            <Icon size={16} />
          </span>
          <h2 id="page-dialog-title" className="page-dialog-title">
            {c.title}
          </h2>
        </div>
        {c.body && (
          <p id="page-dialog-body" className="page-dialog-body">
            {c.body}
          </p>
        )}
        {current.kind === 'external' && <div className="page-dialog-link" title={current.message}>{current.message}</div>}
        {current.kind === 'prompt' && (
          <input
            ref={inputRef}
            className="page-dialog-input"
            value={text}
            onChange={(e) => setText(e.target.value)}
            spellCheck={false}
            aria-label="Your answer"
          />
        )}
        <div className="page-dialog-actions">
          {c.cancel && (
            <button type="button" className="page-dialog-btn secondary" onClick={() => close(false)}>
              {c.cancel}
            </button>
          )}
          <button
            ref={okRef}
            type="button"
            className={`page-dialog-btn primary${current.kind === 'beforeunload' ? ' danger' : ''}`}
            onClick={() => close(true)}
          >
            {c.ok}
          </button>
        </div>
      </div>
    </div>
  );
};

export default PageDialog;
