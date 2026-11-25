import { type ReactNode, useCallback, useEffect, useLayoutEffect, useState } from 'react';
import { Clipboard, ClipboardPaste, Copy, Redo2, Scissors, TextSelect, Undo2 } from 'lucide-react';
import { useElementHole } from '../hooks/useViewportOverlay';

/**
 * The app's own right-click menu, replacing WebView2's generic Edge menu on
 * Serica's UI. Parts of the UI with a menu of their own (tabs, sidebar, pins)
 * call preventDefault and are left alone. Text fields get Undo/Cut/Copy/Paste/
 * Select all, selected text gets Copy, and everything else gets no menu.
 */

type Editable = HTMLInputElement | HTMLTextAreaElement | HTMLElement;

interface MenuState {
  x: number;
  y: number;
  editable: Editable | null;
  hasSelection: boolean;
  readOnly: boolean;
}

const WIDTH = 200;
const TEXT_INPUT_TYPES = new Set(['text', 'search', 'url', 'email', 'tel', 'password', 'number', '']);

function editableOf(target: EventTarget | null): Editable | null {
  const el = target instanceof Element ? target : null;
  if (!el) return null;
  const field = el.closest('input, textarea, [contenteditable=""], [contenteditable="true"]');
  if (!field) return null;
  if (field instanceof HTMLInputElement && !TEXT_INPUT_TYPES.has(field.type)) return null;
  return field as Editable;
}

function selectionIn(field: Editable | null): boolean {
  if (field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement) {
    try {
      return (field.selectionStart ?? 0) !== (field.selectionEnd ?? 0);
    } catch {
      return false; // inputs like type=number have no selection API
    }
  }
  const s = window.getSelection();
  return !!s && !s.isCollapsed && s.toString().length > 0;
}

export function AppContextMenu() {
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [el, setEl] = useState<HTMLDivElement | null>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  // Shows through a live page like the sidebar's menus.
  const ready = useElementHole('app-context-menu', menu ? el : null);

  useEffect(() => {
    // Bubble phase: menus of components ran first and prevented the default.
    const onContextMenu = (e: MouseEvent) => {
      if (e.defaultPrevented) return;
      e.preventDefault();
      const editable = editableOf(e.target);
      const hasSelection = selectionIn(editable);
      if (!editable && !hasSelection) {
        setMenu(null);
        return;
      }
      const readOnly =
        !!editable &&
        ((editable as HTMLInputElement).readOnly || (editable as HTMLInputElement).disabled);
      if (editable && document.activeElement !== editable) editable.focus();
      setMenu({ x: e.clientX, y: e.clientY, editable, hasSelection, readOnly });
    };
    window.addEventListener('contextmenu', onContextMenu);
    return () => window.removeEventListener('contextmenu', onContextMenu);
  }, []);

  const close = useCallback(() => setMenu(null), []);

  useEffect(() => {
    if (!menu) return;
    const onDown = (e: PointerEvent) => {
      if (el && e.target instanceof Node && el.contains(e.target)) return;
      close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    window.addEventListener('pointerdown', onDown, true);
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('blur', close);
    window.addEventListener('resize', close);
    return () => {
      window.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('blur', close);
      window.removeEventListener('resize', close);
    };
  }, [menu, el, close]);

  // Keep the menu inside the window.
  useLayoutEffect(() => {
    if (!menu || !el) {
      setPos(null);
      return;
    }
    const h = el.offsetHeight;
    const left = Math.max(4, Math.min(menu.x, window.innerWidth - WIDTH - 4));
    const top = menu.y + h > window.innerHeight - 4 ? Math.max(4, menu.y - h) : menu.y;
    setPos({ left, top });
  }, [menu, el]);

  if (!menu) return null;

  const run = (fn: () => void | Promise<void>) => async () => {
    menu.editable?.focus();
    try {
      await fn();
    } finally {
      close();
    }
  };
  const exec = (cmd: string) => run(() => void document.execCommand(cmd));
  const paste = run(async () => {
    const text = await window.serica?.readClipboardText?.().catch(() => null);
    if (text) document.execCommand('insertText', false, text);
  });
  const selectAll = run(() => {
    const f = menu.editable;
    if (f instanceof HTMLInputElement || f instanceof HTMLTextAreaElement) f.select();
    else document.execCommand('selectAll');
  });

  const { editable, hasSelection, readOnly } = menu;
  const canWrite = !!editable && !readOnly;
  const isPassword = editable instanceof HTMLInputElement && editable.type === 'password';

  return (
    <div
      ref={setEl}
      className="nsb-menu app-context-menu"
      role="menu"
      style={{
        left: pos?.left ?? menu.x,
        top: pos?.top ?? menu.y,
        width: WIDTH,
        visibility: ready && pos ? 'visible' : 'hidden',
      }}
      // Keep focus and the selection in the field while using the menu.
      onMouseDown={(e) => e.preventDefault()}
      onContextMenu={(e) => e.preventDefault()}
    >
      {editable ? (
        <>
          <Item icon={<Undo2 size={13} />} label="Undo" hint="Ctrl+Z" disabled={!canWrite} onClick={exec('undo')} />
          <Item icon={<Redo2 size={13} />} label="Redo" hint="Ctrl+Y" disabled={!canWrite} onClick={exec('redo')} />
          <div className="app-context-menu-sep" />
          <Item icon={<Scissors size={13} />} label="Cut" hint="Ctrl+X" disabled={!canWrite || !hasSelection || isPassword} onClick={exec('cut')} />
          <Item icon={<Copy size={13} />} label="Copy" hint="Ctrl+C" disabled={!hasSelection || isPassword} onClick={exec('copy')} />
          <Item icon={<ClipboardPaste size={13} />} label="Paste" hint="Ctrl+V" disabled={!canWrite} onClick={paste} />
          <div className="app-context-menu-sep" />
          <Item icon={<TextSelect size={13} />} label="Select all" hint="Ctrl+A" onClick={selectAll} />
        </>
      ) : (
        <Item icon={hasSelection ? <Copy size={13} /> : <Clipboard size={13} />} label="Copy" hint="Ctrl+C" onClick={exec('copy')} />
      )}
    </div>
  );
}

function Item(props: { icon: ReactNode; label: string; hint?: string; disabled?: boolean; onClick: () => void }) {
  return (
    <button type="button" role="menuitem" className="nsb-menu-item" disabled={props.disabled} onClick={props.onClick}>
      {props.icon}
      <span>{props.label}</span>
      {props.hint && <span className="nsb-menu-hint">{props.hint}</span>}
    </button>
  );
}
