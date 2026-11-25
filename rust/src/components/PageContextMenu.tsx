import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import {
  ArrowLeft,
  ArrowRight,
  ClipboardPaste,
  Code,
  Copy,
  Download,
  ExternalLink,
  FileCode,
  Image as ImageIcon,
  Link,
  Printer,
  Redo2,
  RotateCw,
  Scissors,
  Search,
  Smile,
  SpellCheck,
  TextSelect,
  Undo2,
  type LucideIcon,
} from 'lucide-react';
import type { PageContextMenuRequest, PageMenuItem } from '../types';
import { useElementHole } from '../hooks/useViewportOverlay';
import { addSettledBlurListener } from '../utils/settledBlur';

const WIDTH = 248;

/** Icons for the engine's items (by WebView2 item name) and Serica's entries. */
const ICONS: Record<string, LucideIcon> = {
  back: ArrowLeft,
  forward: ArrowRight,
  reload: RotateCw,
  saveAs: Download,
  print: Printer,
  inspectElement: Code,
  viewSource: FileCode,
  cut: Scissors,
  copy: Copy,
  paste: ClipboardPaste,
  pasteAndMatchStyle: ClipboardPaste,
  selectAll: TextSelect,
  undo: Undo2,
  redo: Redo2,
  emoji: Smile,
  spellCheck: SpellCheck,
  copyLinkLocation: Link,
  saveLinkAs: Download,
  saveImageAs: Download,
  copyImage: ImageIcon,
  copyImageLocation: Link,
  saveMediaAs: Download,
};

function iconFor(item: PageMenuItem): LucideIcon | null {
  if (item.type === 'separator') return null;
  if (item.type === 'custom') {
    if (item.name === 'viewSource') return FileCode;
    if (item.label.startsWith('Search')) return Search;
    if (item.label.startsWith('Open image')) return ImageIcon;
    return ExternalLink;
  }
  return ICONS[item.name] ?? null;
}

type Pick = { command?: number; custom?: number; refocus?: boolean };

/**
 * Serica's right-click menu for web pages. The tab host suppresses the
 * engine's menu and sends its items here; picking one runs it in the page as
 * the engine's own command (copy, paste, spelling, inspect...), Serica's own
 * entries open tabs. Shown through the live page like the sidebar's menus.
 */
export function PageContextMenu() {
  const [req, setReq] = useState<PageContextMenuRequest | null>(null);
  const [el, setEl] = useState<HTMLDivElement | null>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const [sub, setSub] = useState<{ index: number } | null>(null);
  const reqRef = useRef(req);
  reqRef.current = req;
  const ready = useElementHole('page-context-menu', req ? el : null);

  /** Ends the current menu once; the backend completes the engine's request. */
  const finish = useCallback((pick: Pick = {}) => {
    const cur = reqRef.current;
    if (!cur) return;
    reqRef.current = null;
    setReq(null);
    setSub(null);
    void window.serica?.pageMenuSelect?.(cur.id, pick);
  }, []);

  useEffect(() => {
    return window.serica?.onPageContextMenu?.((m) => {
      // A new request replaces an open menu (the backend already ended it).
      reqRef.current = m;
      setSub(null);
      setPos(null);
      setReq(m);
    });
  }, []);

  // Close on a click elsewhere, Escape, focus loss (a click back into the page)
  // or a resize.
  useEffect(() => {
    if (!req) return;
    const onDown = (e: PointerEvent) => {
      if (el && e.target instanceof Node && el.contains(e.target)) return;
      finish();
    };
    const onResize = () => finish();
    window.addEventListener('pointerdown', onDown, true);
    window.addEventListener('resize', onResize);
    const removeBlur = addSettledBlurListener(() => finish());
    return () => {
      window.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('resize', onResize);
      removeBlur();
    };
  }, [req, el, finish]);

  // Place the menu at the click, inside the window.
  useLayoutEffect(() => {
    if (!req || !el) return;
    const dpr = window.devicePixelRatio || 1;
    const x = req.x / dpr;
    const y = req.y / dpr;
    const h = el.offsetHeight;
    const left = Math.max(4, Math.min(x, window.innerWidth - WIDTH - 4));
    const top = y + h > window.innerHeight - 4 ? Math.max(4, y - h) : y;
    setPos({ left, top });
    el.focus({ preventScroll: true });
  }, [req, el]);

  if (!req) return null;

  const pickItem = (item: PageMenuItem) => {
    if (item.type === 'custom') finish({ custom: item.index });
    else if (item.type === 'command' && item.enabled) finish({ command: item.id });
  };

  const items = req.items;
  const subItem = sub ? items[sub.index] : null;
  const subChildren = subItem && subItem.type === 'submenu' ? subItem.children : null;

  const onKeyDown = (e: React.KeyboardEvent) => {
    const buttons = Array.from(el?.querySelectorAll<HTMLButtonElement>('button[role=menuitem]:not(:disabled)') ?? []);
    const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      finish({ refocus: true });
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const next = e.key === 'ArrowDown' ? (i + 1) % buttons.length : (i - 1 + buttons.length) % buttons.length;
      buttons[next]?.focus();
    } else if (e.key === 'Home' || e.key === 'End') {
      e.preventDefault();
      buttons[e.key === 'Home' ? 0 : buttons.length - 1]?.focus();
    }
  };

  const header =
    req.context === 'link' && req.link
      ? req.link.replace(/^https?:\/\//, '')
      : req.context === 'image' && req.image && !req.image.startsWith('data:')
        ? req.image.replace(/^https?:\/\//, '')
        : null;

  const renderItem = (item: PageMenuItem, i: number, inSub: boolean) => {
    if (item.type === 'separator') return <div key={`sep-${i}`} className="app-context-menu-sep" role="separator" />;
    const Icon = iconFor(item);
    const disabled = item.type !== 'custom' && !item.enabled;
    const isSub = item.type === 'submenu';
    return (
      <button
        key={`${item.type}-${'id' in item ? item.id : item.index}-${i}`}
        type="button"
        role="menuitem"
        aria-haspopup={isSub ? 'menu' : undefined}
        aria-checked={item.type === 'command' && item.checked ? true : undefined}
        className="nsb-menu-item"
        disabled={disabled}
        onClick={() => {
          if (isSub && !inSub) setSub({ index: i });
          else pickItem(item);
        }}
      >
        {Icon ? <Icon size={14} aria-hidden="true" /> : <span className="menu-icon-blank" aria-hidden="true" />}
        <span>{item.label}</span>
        {item.type === 'command' && item.checked && <span className="nsb-menu-hint">✓</span>}
        {item.type === 'command' && item.shortcut && <span className="nsb-menu-hint">{item.shortcut}</span>}
        {isSub && <span className="nsb-menu-more">›</span>}
      </button>
    );
  };

  return (
    <div
      ref={setEl}
      tabIndex={-1}
      className="nsb-menu page-menu"
      role="menu"
      aria-label="Page"
      style={{
        left: pos?.left ?? 0,
        top: pos?.top ?? 0,
        width: WIDTH,
        visibility: ready && pos ? 'visible' : 'hidden',
      }}
      onKeyDown={onKeyDown}
      onMouseDown={(e) => e.preventDefault()}
      onContextMenu={(e) => e.preventDefault()}
    >
      {subChildren ? (
        <>
          <button type="button" role="menuitem" className="nsb-menu-item nsb-menu-back" onClick={() => setSub(null)}>
            <ArrowLeft size={14} aria-hidden="true" />
            <span>{subItem && subItem.type !== 'separator' ? subItem.label : 'Back'}</span>
          </button>
          <div className="app-context-menu-sep" role="separator" />
          {subChildren.map((child, j) => renderItem(child, j, true))}
        </>
      ) : (
        <>
          {header && <div className="menu-label" title={header}>{header}</div>}
          {items.map((item, i) => renderItem(item, i, false))}
        </>
      )}
    </div>
  );
}
