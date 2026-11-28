import React, { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Palette, X } from 'lucide-react';
import type { Space } from '../types';
import { SPACE_COLORS } from './SpaceMenu';
import { useViewportOverlay } from '../hooks/useViewportOverlay';

/**
 * A space's editing window (double-click the space's name): its name, icon
 * and color, and a way into Theme Studio for its colors. Icon and color
 * apply as they are picked; the name when you press Enter, click Done, or
 * close the window.
 */
export const SpaceEditor: React.FC<{
  space: Space;
  icons: readonly string[];
  renderIcon: (icon: string, color: string) => React.ReactNode;
  onUpdate: (patch: { name?: string; icon?: string; color?: string }) => void;
  onOpenTheme: () => void;
  onClose: () => void;
}> = ({ space, icons, renderIcon, onUpdate, onOpenTheme, onClose }) => {
  const [name, setName] = useState(space.name);
  const nameRef = useRef(name);
  nameRef.current = name;
  // The page shows as a still image while this covers it.
  useViewportOverlay(true);

  const commitName = () => {
    const n = nameRef.current.trim();
    if (n && n !== space.name) onUpdate({ name: n });
  };
  const close = () => {
    commitName();
    onClose();
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        close();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  });

  return createPortal(
    <div className="modal-overlay wed-overlay space-editor-overlay" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div className="sheet space-editor" role="dialog" aria-modal="true" aria-label={`Edit ${space.name}`}>
        <header className="wed-head">
          <span className="wed-icon" style={{ color: space.color }}>
            {renderIcon(space.icon, space.color)}
          </span>
          <div className="wed-titles">
            <h2 className="wed-title">Edit space</h2>
            <p className="wed-sub">Its name, icon and color. Each space keeps its own tabs and theme.</p>
          </div>
          <button type="button" className="btn btn-quiet btn-icon" aria-label="Close" onClick={close}>
            <X size={16} />
          </button>
        </header>
        <div className="wed-body">
          <div className="wed-field">
            <div className="wed-label">Name</div>
            <input
              className="input"
              value={name}
              maxLength={32}
              autoFocus
              onFocus={(e) => e.currentTarget.select()}
              onChange={(e) => setName(e.target.value)}
              onBlur={commitName}
              onKeyDown={(e) => {
                e.stopPropagation();
                if (e.key === 'Enter') close();
                else if (e.key === 'Escape') {
                  e.preventDefault();
                  close();
                }
              }}
              aria-label="Space name"
            />
          </div>
          <div className="wed-field">
            <div className="wed-label">Icon</div>
            <div className="space-editor-icons" role="radiogroup" aria-label="Icon">
              {icons.map((icon) => (
                <button
                  key={icon}
                  type="button"
                  role="radio"
                  aria-checked={space.icon === icon}
                  aria-label={icon}
                  title={icon}
                  className={`space-editor-icon${space.icon === icon ? ' on' : ''}`}
                  onClick={() => onUpdate({ icon })}
                >
                  {renderIcon(icon, space.color)}
                </button>
              ))}
            </div>
          </div>
          <div className="wed-field">
            <div className="wed-label">Color</div>
            <div className="space-editor-colors" role="radiogroup" aria-label="Color">
              {SPACE_COLORS.map((color) => {
                const on = space.color.toLowerCase() === color.toLowerCase();
                return (
                  <button
                    key={color}
                    type="button"
                    role="radio"
                    aria-checked={on}
                    aria-label={color}
                    className={`space-editor-swatch${on ? ' on' : ''}`}
                    style={{ '--swatch': color } as React.CSSProperties}
                    onClick={() => onUpdate({ color })}
                  />
                );
              })}
            </div>
          </div>
          <div className="wed-row space-editor-theme">
            <span>
              <span className="space-editor-theme-title">Theme</span>
              <span className="wed-hint">This space’s colors in the browser</span>
            </span>
            <button
              type="button"
              className="btn"
              onClick={() => {
                commitName();
                onClose();
                onOpenTheme();
              }}
            >
              <Palette size={13} aria-hidden="true" /> Theme Studio…
            </button>
          </div>
        </div>
        <footer className="wed-foot">
          <span className="wed-spacer" />
          <button type="button" className="btn btn-primary" onClick={close}>
            Done
          </button>
        </footer>
      </div>
    </div>,
    document.body,
  );
};

export default SpaceEditor;
