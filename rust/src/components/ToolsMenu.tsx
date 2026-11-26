import React, { useEffect, useRef, useState } from 'react';
import { Ellipsis, Moon, Settings, Sun } from 'lucide-react';
import { TOOLS, type ToolActions, type ToolKey } from '../shared/tools';

export interface ToolsMenuProps {
  isDarkMode: boolean;
  vpnOn: boolean;
  /** Tools pinned to the top bar (Settings > Appearance > Top bar tools). */
  pinned: readonly ToolKey[];
  actions: ToolActions;
  onToggleDarkMode: () => void;
  onSettings: () => void;
  /** Lets the top bar make room (it hides the URL pill on narrow windows). */
  onOpenChange?: (open: boolean) => void;
}

interface Tool {
  key: string;
  label: string;
  icon: React.ReactNode;
  run: () => void;
  active?: boolean;
}

/**
 * The toolbar's "more" button. The pinned tools, the light/dark switch and
 * Settings slide out sideways from it,
 * inside the top bar (they never cover the page), and slide back after a pick,
 * on Escape or on a click elsewhere.
 */
export const ToolsMenu: React.FC<ToolsMenuProps> = (p) => {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const onOpenChange = p.onOpenChange;
  useEffect(() => {
    onOpenChange?.(open);
  }, [open, onOpenChange]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    window.addEventListener('pointerdown', onDown, true);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('keydown', onKey);
    };
  }, [open]);

  const pinned = new Set(p.pinned);
  const tools: Tool[] = [
    ...TOOLS.filter((t) => pinned.has(t.key)).map((t) => {
      const Icon = t.icon;
      const on = t.key === 'vpn' && p.vpnOn;
      return { key: t.key, label: on ? `${t.label} (on)` : t.label, icon: <Icon size={15} />, run: p.actions[t.key], active: on };
    }),
    {
      key: 'mode',
      label: p.isDarkMode ? 'Switch to light mode' : 'Switch to dark mode',
      icon: p.isDarkMode ? <Sun size={15} /> : <Moon size={15} />,
      run: p.onToggleDarkMode,
    },
    { key: 'settings', label: 'Settings (Ctrl+,)', icon: <Settings size={15} />, run: p.onSettings },
  ];

  return (
    <div
      className={`tools-tray${open ? ' open' : ''}`}
      ref={rootRef}
      style={{ '--tools-count': tools.length } as React.CSSProperties}
    >
      <div className="tools-tray-items" role="toolbar" aria-label="Browser tools" aria-hidden={!open}>
        {tools.map((t, i) => (
          <button
            key={t.key}
            type="button"
            className={`minimal-icon-btn tools-tray-btn${t.active ? ' on' : ''}`}
            style={{ '--tool-index': i } as React.CSSProperties}
            title={t.label}
            aria-label={t.label}
            tabIndex={open ? 0 : -1}
            onClick={() => {
              setOpen(false);
              t.run();
            }}
          >
            {t.icon}
          </button>
        ))}
      </div>
      <button
        type="button"
        className={`minimal-icon-btn tools-menu-btn${open ? ' active' : ''}`}
        data-tour="tools"
        aria-expanded={open}
        aria-label={open ? 'Hide browser tools' : 'Browser tools'}
        title={open ? 'Hide browser tools' : 'Browser tools'}
        onClick={() => setOpen((o) => !o)}
      >
        <Ellipsis size={16} strokeWidth={2.2} className="tools-menu-glyph" />
      </button>
    </div>
  );
};
