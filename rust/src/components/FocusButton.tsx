import React from 'react';
import { Lock, Target } from 'lucide-react';
import { formatRemaining, remainingNow, useFocusView, useTicker } from '../hooks/useFocus';

/**
 * Focus Mode in the top bar: a quiet icon when nothing is blocked, the time
 * left (and a lock) while a list is on. Opens Settings › Focus.
 */
export const FocusButton: React.FC<{ onOpen: () => void }> = ({ onOpen }) => {
  const view = useFocusView();
  const active = view?.active.find((a) => a.remainingMs !== null) ?? view?.active[0];
  useTicker(!!active && active.remainingMs !== null);
  const name = active ? view?.blocks.find((b) => b.id === active.blockId)?.name ?? 'Focus' : null;
  const left = active ? remainingNow(active.remainingMs) : null;
  return (
    <button
      type="button"
      data-tour="focus"
      className={`minimal-pill-btn focus-top${active ? ' on' : ''}${active?.locked ? ' locked' : ''}`}
      onClick={onOpen}
      title={active ? `${name}: ${active.remainingMs === null ? 'on until you stop it' : `${formatRemaining(left)} left`}${active.locked ? ' (locked)' : ''}` : 'Focus Mode'}
      aria-label={active ? `Focus Mode on: ${name}` : 'Focus Mode'}
    >
      {active?.locked ? <Lock size={13} aria-hidden="true" /> : <Target size={14} aria-hidden="true" />}
      {active && <span className="tnum">{formatRemaining(left)}</span>}
    </button>
  );
};

export default FocusButton;
