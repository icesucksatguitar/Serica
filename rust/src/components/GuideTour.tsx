import React, { useCallback, useEffect, useLayoutEffect, useState } from 'react';
import { useElementHole } from '../hooks/useViewportOverlay';

export interface GuideStep {
  /** CSS selector of the element explained; none centers the card. */
  target?: string;
  title: string;
  body: string;
}

interface Rect {
  left: number;
  top: number;
  width: number;
  height: number;
}

const CARD_W = 300;
const PAD = 6;

/**
 * A short guided tour: dims the window except the element a step is about,
 * and explains it in a small card beside it. Steps whose element is missing
 * (closed panel, nothing to point at) are shown centered instead. Escape or
 * Skip ends it; arrow keys move between steps.
 */
export const GuideTour: React.FC<{ id: string; steps: GuideStep[]; onClose: () => void }> = ({ id, steps, onClose }) => {
  const [i, setI] = useState(0);
  const [rect, setRect] = useState<Rect | null>(null);
  const [card, setCard] = useState<HTMLDivElement | null>(null);
  const [cardPos, setCardPos] = useState<{ left: number; top: number } | null>(null);
  const step = steps[i];
  // The card shows through a live page like the app's menus.
  useElementHole(`guide-${id}`, card);

  // Follow the target while layout settles (panels sliding in) and on resize.
  useLayoutEffect(() => {
    let raf = 0;
    let frames = 0;
    const measure = () => {
      const el = step?.target ? document.querySelector<HTMLElement>(step.target) : null;
      if (el) {
        el.scrollIntoView({ block: 'nearest', inline: 'nearest' });
        const r = el.getBoundingClientRect();
        setRect((prev) =>
          prev && prev.left === r.left && prev.top === r.top && prev.width === r.width && prev.height === r.height
            ? prev
            : { left: r.left, top: r.top, width: r.width, height: r.height },
        );
      } else {
        setRect(null);
      }
      if (++frames < 30) raf = requestAnimationFrame(measure);
    };
    measure();
    const onResize = () => {
      frames = 0;
      measure();
    };
    window.addEventListener('resize', onResize);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener('resize', onResize);
    };
  }, [step]);

  // Beside the target: below it if there is room, else above, else centered.
  useLayoutEffect(() => {
    if (!card) return;
    const h = card.offsetHeight;
    const W = window.innerWidth;
    const H = window.innerHeight;
    if (!rect) {
      setCardPos({ left: (W - CARD_W) / 2, top: (H - h) / 2 });
      return;
    }
    const clampX = (x: number) => Math.max(12, Math.min(W - CARD_W - 12, x));
    const below = rect.top + rect.height + PAD + 10;
    const above = rect.top - PAD - 10 - h;
    const right = rect.left + rect.width + PAD + 12;
    let pos: { left: number; top: number };
    if (below + h <= H - 12) pos = { left: clampX(rect.left), top: below };
    else if (above >= 12) pos = { left: clampX(rect.left), top: above };
    else if (right + CARD_W <= W - 12) pos = { left: right, top: Math.max(12, Math.min(H - h - 12, rect.top)) };
    else pos = { left: (W - CARD_W) / 2, top: (H - h) / 2 };
    setCardPos(pos);
  }, [card, rect, i]);

  const next = useCallback(() => (i < steps.length - 1 ? setI(i + 1) : onClose()), [i, steps.length, onClose]);
  const back = useCallback(() => setI((v) => Math.max(0, v - 1)), []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      } else if (e.key === 'ArrowRight' || e.key === 'Enter') {
        e.preventDefault();
        next();
      } else if (e.key === 'ArrowLeft') {
        e.preventDefault();
        back();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [next, back, onClose]);

  if (!step) return null;
  return (
    <div className="guide-layer" role="dialog" aria-modal="true" aria-label={step.title}>
      {rect ? (
        <div
          className="guide-spotlight"
          style={{ left: rect.left - PAD, top: rect.top - PAD, width: rect.width + PAD * 2, height: rect.height + PAD * 2 }}
        />
      ) : (
        <div className="guide-scrim" />
      )}
      <div
        ref={setCard}
        className="guide-card"
        style={{ left: cardPos?.left ?? -9999, top: cardPos?.top ?? 0, width: CARD_W }}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="guide-progress" aria-hidden="true">
          {steps.map((_, k) => (
            <span key={k} className={k === i ? 'on' : k < i ? 'done' : ''} />
          ))}
        </div>
        <h3 className="guide-title">{step.title}</h3>
        <p className="guide-body">{step.body}</p>
        <div className="guide-actions">
          <button type="button" className="btn btn-quiet" onClick={onClose}>
            {i === steps.length - 1 ? 'Close' : 'Skip'}
          </button>
          <span className="guide-spacer" />
          {i > 0 && (
            <button type="button" className="btn" onClick={back}>
              Back
            </button>
          )}
          <button type="button" className="btn btn-primary" autoFocus onClick={next}>
            {i === steps.length - 1 ? 'Done' : 'Next'}
          </button>
        </div>
      </div>
    </div>
  );
};

/** Asks the app to play a guide (`'widgets'` or `'sidebar'`). */
export function startGuide(name: 'widgets' | 'sidebar'): void {
  window.dispatchEvent(new CustomEvent('serica:guide', { detail: name }));
}
