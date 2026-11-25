import React, { useState } from 'react';
import { ArrowLeft, ArrowRight, Edit2, FolderPlus, Palette, Plus, Trash2 } from 'lucide-react';
import type { Space } from '../types';

/** Colors offered for a space; the first ones match the backend's defaults. */
export const SPACE_COLORS = [
  '#8B5CF6',
  '#3B82F6',
  '#06B6D4',
  '#10B981',
  '#84CC16',
  '#F59E0B',
  '#F97316',
  '#EF4444',
  '#EC4899',
  '#A1A1AA',
] as const;

export interface SpaceMenuProps {
  space: Space;
  /** Position among its profile's spaces, and how many it has. */
  index: number;
  count: number;
  /** Open tabs in the space (said in the delete confirmation). */
  tabCount: number;
  icons: readonly string[];
  renderIcon: (icon: string, color: string) => React.ReactNode;
  onRename: () => void;
  onUpdate: (patch: { icon?: string; color?: string }) => void;
  onMove: (toIndex: number) => void;
  onNewTab: () => void;
  onNewFolder: () => void;
  onDelete: () => void;
  onClose: () => void;
}

/**
 * Space actions, from a right-click on a space (its switcher icon or the
 * sidebar header) or the header's "more" button. Deleting asks first, inside
 * the menu, since it closes the space's tabs.
 */
export const SpaceMenu: React.FC<SpaceMenuProps> = ({
  space,
  index,
  count,
  tabCount,
  icons,
  renderIcon,
  onRename,
  onUpdate,
  onMove,
  onNewTab,
  onNewFolder,
  onDelete,
  onClose,
}) => {
  const [view, setView] = useState<'main' | 'look' | 'delete'>('main');
  const run = (fn: () => void) => () => {
    fn();
    onClose();
  };

  if (view === 'look') {
    return (
      <div className="space-look" role="group" aria-label={`Icon and color for ${space.name}`}>
        <button type="button" role="menuitem" className="nsb-menu-item nsb-menu-back" onClick={() => setView('main')}>
          <ArrowLeft size={13} />
          <span>Icon and color</span>
        </button>
        <div className="space-look-grid" role="radiogroup" aria-label="Icon">
          {icons.map((icon) => (
            <button
              key={icon}
              type="button"
              role="radio"
              aria-checked={space.icon === icon}
              aria-label={icon}
              title={icon}
              className={`space-look-cell${space.icon === icon ? ' selected' : ''}`}
              onClick={() => onUpdate({ icon })}
            >
              {renderIcon(icon, space.color)}
            </button>
          ))}
        </div>
        <div className="space-look-colors" role="radiogroup" aria-label="Color">
          {SPACE_COLORS.map((color) => (
            <button
              key={color}
              type="button"
              role="radio"
              aria-checked={space.color.toLowerCase() === color.toLowerCase()}
              aria-label={color}
              className={`space-look-swatch${space.color.toLowerCase() === color.toLowerCase() ? ' selected' : ''}`}
              style={{ '--swatch': color } as React.CSSProperties}
              onClick={() => onUpdate({ color })}
            />
          ))}
        </div>
      </div>
    );
  }

  if (view === 'delete') {
    const lastOne = count <= 1;
    return (
      <div className="menu-confirm" role="alertdialog" aria-label={`Delete ${space.name}`}>
        <p className="menu-confirm-title">Delete “{space.name}”?</p>
        <p className="menu-confirm-text">
          {lastOne
            ? 'A profile needs at least one space.'
            : tabCount > 0
              ? `Its ${tabCount} ${tabCount === 1 ? 'tab closes' : 'tabs close'}, and its folders are removed.`
              : 'Its folders are removed.'}
        </p>
        <div className="menu-confirm-actions">
          <button type="button" className="btn btn-quiet" autoFocus onClick={() => setView('main')}>
            Cancel
          </button>
          <button type="button" className="btn btn-danger" disabled={lastOne} onClick={run(onDelete)}>
            Delete space
          </button>
        </div>
      </div>
    );
  }

  return (
    <>
      <button type="button" role="menuitem" className="nsb-menu-item" onClick={run(onNewTab)}>
        <Plus size={13} />
        <span>New tab</span>
      </button>
      <button type="button" role="menuitem" className="nsb-menu-item" onClick={run(onNewFolder)}>
        <FolderPlus size={13} />
        <span>New folder</span>
      </button>
      <div className="app-context-menu-sep" role="separator" />
      <button type="button" role="menuitem" className="nsb-menu-item" onClick={run(onRename)}>
        <Edit2 size={13} />
        <span>Rename</span>
      </button>
      <button type="button" role="menuitem" className="nsb-menu-item" onClick={() => setView('look')}>
        <Palette size={13} />
        <span>Icon and color</span>
        <span className="nsb-menu-more">›</span>
      </button>
      {count > 1 && (
        <>
          <button
            type="button"
            role="menuitem"
            className="nsb-menu-item"
            disabled={index <= 0}
            onClick={run(() => onMove(index - 1))}
          >
            <ArrowLeft size={13} />
            <span>Move left</span>
          </button>
          <button
            type="button"
            role="menuitem"
            className="nsb-menu-item"
            disabled={index >= count - 1}
            onClick={run(() => onMove(index + 1))}
          >
            <ArrowRight size={13} />
            <span>Move right</span>
          </button>
        </>
      )}
      <div className="app-context-menu-sep" role="separator" />
      <button type="button" role="menuitem" className="nsb-menu-item danger" onClick={() => setView('delete')}>
        <Trash2 size={13} />
        <span>Delete space…</span>
      </button>
    </>
  );
};
