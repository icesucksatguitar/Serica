// Arc-style swipe between spaces. A two-finger horizontal wheel gesture moves the
// pane track 1:1 (refs + rAF, no React state per frame); on release it snaps to
// the neighbouring space or springs back. External space changes (clicking a
// space icon, Ctrl+N) play the same slide. Gesture math lives in utils/spaceSwipe.
import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import {
  FLICK_VELOCITY,
  GESTURE_END_MS,
  SNAP_MS,
  decideTarget,
  easeOutCubic,
  estimateVelocity,
  lockAxis,
  rawBounds,
  resolveOffset,
  settleOffset,
  type SwipeAxis,
} from '../utils/spaceSwipe';

export interface SwipeSpace {
  id: string;
  color: string;
}

export interface SpaceSwipeOptions {
  /** Element receiving the wheel events; also carries `--nsb-space-color`. */
  rootRef: RefObject<HTMLElement>;
  /** The three-pane track. Its parent is the clipping viewport. */
  trackRef: RefObject<HTMLElement>;
  order: readonly SwipeSpace[];
  activeId: string;
  /** True while dragging a tab or with the context menu open: no swipes start. */
  blocked: boolean;
  /** Called once, when the snap animation completes on another space. */
  onCommit: (spaceId: string) => void;
  /** Called when a swipe starts (hide hover previews). */
  onStart: () => void;
}

/** A space shown in an adjacent slot while a click/shortcut slide plays. */
export interface OutgoingSpace {
  id: string;
  side: 'prev' | 'next';
}

type Phase = 'idle' | 'drag' | 'anim' | 'pending';

/** shortcut: if main never answers selectSpace, give up waiting after this and spring back. */
const PENDING_TIMEOUT_MS = 800;
const TINT_VAR = '--nsb-space-color';

const reducedMotion = () =>
  typeof window !== 'undefined' && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches === true;

export function useSpaceSwipe(opts: SpaceSwipeOptions): OutgoingSpace | null {
  const [outgoing, setOutgoing] = useState<OutgoingSpace | null>(null);
  const optsRef = useRef(opts);
  optsRef.current = opts;

  const s = useRef({
    phase: 'idle' as Phase,
    axis: null as SwipeAxis | 'spent' | null,
    accX: 0,
    accY: 0,
    raw: 0,
    lastAt: 0,
    width: 0,
    index: 0,
    offset: 0,
    samples: [] as { t: number; x: number }[],
    vHist: [] as { t: number; v: number }[],
    reduced: false,
    otherColor: null as string | null, // colour slid away from during an external slide
    raf: 0,
    frame: 0,
    endTimer: 0,
    pendingTimer: 0,
    pendingId: null as string | null,
    /** Offset the commit animation heads for (one pane either way). */
    settle: 0,
    prevActive: opts.activeId,
    /** The track's running compositor animation (see `animate`). */
    anim: null as Animation | null,
    animFrom: 0,
    animTo: 0,
    animMs: 0,
    /** Last tint written, so unchanged frames skip a style recalculation. */
    tint: '',
  }).current;

  const writeTint = (value: string) => {
    if (value === s.tint) return;
    s.tint = value;
    optsRef.current.rootRef.current?.style.setProperty(TINT_VAR, value);
  };

  const setTint = (offset: number) => {
    const { order, activeId, rootRef } = optsRef.current;
    if (!rootRef.current) return;
    const i = order.findIndex((x) => x.id === activeId);
    const own = order[i]?.color;
    if (!own) return;
    const other = s.otherColor ?? order[i + (offset < 0 ? 1 : -1)]?.color;
    const p = Math.min(1, Math.abs(offset) / (s.width || 1));
    // Whole percents: finer steps are invisible and would recalculate the
    // sidebar's styles on every frame of a slide.
    const pct = Math.round(p * 100);
    writeTint(other && pct > 0 ? `color-mix(in srgb, ${other} ${pct}%, ${own})` : own);
  };

  const plainTint = (id: string) => {
    const color = optsRef.current.order.find((x) => x.id === id)?.color;
    if (!color) return;
    // Always written (once per slide): React also sets this property from the
    // active space's colour, so the cached value may be out of date here.
    s.tint = color;
    optsRef.current.rootRef.current?.style.setProperty(TINT_VAR, color);
  };

  const trackTransform = (offset: number) => `translate3d(${offset - s.width}px,0,0)`;

  const apply = (offset: number) => {
    s.offset = offset;
    const track = optsRef.current.trackRef.current;
    if (track) track.style.transform = trackTransform(offset);
    setTint(offset);
  };

  /** Where the running track animation is on screen right now. */
  const animOffset = () => {
    const a = s.anim;
    if (!a) return s.offset;
    const t = Number(a.currentTime ?? 0);
    return s.animFrom + (s.animTo - s.animFrom) * easeOutCubic(s.animMs > 0 ? t / s.animMs : 1);
  };

  /** Stops any slide, leaving the track exactly where it is on screen. */
  const stopAnim = () => {
    cancelAnimationFrame(s.raf);
    cancelAnimationFrame(s.frame);
    s.raf = 0;
    s.frame = 0;
    if (s.anim) {
      // Pin the inline transform to the visible spot before the animation's
      // effect goes away, so nothing jumps.
      apply(animOffset());
      s.anim.cancel();
      s.anim = null;
    }
  };

  /**
   * Glides the track to `to`. The movement itself runs as a compositor
   * animation, so it stays smooth while the main thread is busy (switching
   * spaces re-renders the app and swaps the page at exactly this moment); a
   * rAF loop alongside only keeps `s.offset` and the colour tint in step.
   */
  const animate = (to: number, done: () => void) => {
    stopAnim();
    const from = s.offset;
    if (Math.abs(to - from) < 0.5) {
      apply(to);
      done();
      return;
    }
    // Dynamically scale duration with distance: full pane is SNAP_MS (~220ms), tiny adjustments take ~130ms
    const dist = Math.abs(to - from);
    const duration = Math.min(SNAP_MS, Math.max(130, (dist / (s.width || 260)) * SNAP_MS));
    const track = optsRef.current.trackRef.current;
    if (track && typeof track.animate === 'function') {
      s.animFrom = from;
      s.animTo = to;
      s.animMs = duration;
      // cubic-bezier(0.33, 1, 0.68, 1) is easeOutCubic.
      s.anim = track.animate([{ transform: trackTransform(from) }, { transform: trackTransform(to) }], {
        duration,
        easing: 'cubic-bezier(0.33, 1, 0.68, 1)',
        fill: 'forwards',
      });
    }
    const t0 = performance.now();
    const finish = () => {
      s.raf = 0;
      apply(to); // inline transform takes over before the animation is dropped
      if (s.anim) {
        s.anim.cancel();
        s.anim = null;
      }
      done();
    };
    const step = (t: number) => {
      if (s.anim) {
        if (s.anim.playState === 'finished') return finish();
        s.offset = animOffset();
        setTint(s.offset);
        s.raf = requestAnimationFrame(step);
        return;
      }
      const p = Math.min(1, (t - t0) / duration);
      if (p >= 1) return finish();
      apply(from + (to - from) * easeOutCubic(p));
      s.raf = requestAnimationFrame(step);
    };
    s.raf = requestAnimationFrame(step);
  };

  /** Back to the resting layout (CSS default transform) with the current space's tint. */
  const rest = () => {
    stopAnim();
    const { trackRef, activeId } = optsRef.current;
    if (trackRef.current) trackRef.current.style.transform = '';
    s.offset = 0;
    s.raw = 0;
    s.accX = 0;
    s.accY = 0;
    // A committed swipe's momentum stays ignored until the wheel pauses.
    if (s.axis !== 'spent') s.axis = null;
    s.samples = [];
    s.vHist = [];
    s.otherColor = null;
    s.phase = 'idle';
    s.pendingId = null;
    window.clearTimeout(s.pendingTimer);
    window.clearTimeout(s.endTimer);
    plainTint(activeId);
  };

  const springBack = () => {
    s.phase = 'anim';
    animate(0, rest);
  };

  /**
   * Switches to `targetId` now and lets the track glide the rest of the way.
   * The space changes the moment the swipe is decided (not after the glide),
   * so the sidebar, top bar and page never lag behind the gesture.
   */
  const commitTo = (target: number, targetId: string) => {
    window.clearTimeout(s.endTimer);
    if (s.reduced) {
      rest();
      optsRef.current.onCommit(targetId);
      return;
    }
    s.phase = 'anim';
    s.pendingId = targetId;
    s.settle = settleOffset(target, s.index, s.width);
    // The rest of this gesture's momentum must not start another swipe.
    s.axis = 'spent';
    animate(s.settle, rest);
    optsRef.current.onCommit(targetId);
  };

  /** Mid-gesture: a swipe past half a pane, or a clear flick, is decided at once. */
  const maybeCommitEarly = (now: number) => {
    const { order } = optsRef.current;
    const offset = resolveOffset(s.raw, s.index, order.length, s.width);
    const velocity = estimateVelocity(s.samples, now);
    const far = Math.abs(offset) >= 0.5 * s.width;
    const flick = Math.abs(velocity) >= FLICK_VELOCITY * 1.3 && Math.sign(velocity) === Math.sign(offset) && Math.abs(offset) >= 32;
    if (!far && !flick) return false;
    const target = Math.max(0, Math.min(order.length - 1, s.index + (offset < 0 ? 1 : -1)));
    const targetId = order[target]?.id;
    if (target === s.index || !targetId) return false;
    commitTo(target, targetId);
    return true;
  };

  const finishGesture = () => {
    if (s.phase !== 'drag') return;
    const { order, activeId } = optsRef.current;
    const now = performance.now();
    const last = s.samples[s.samples.length - 1];
    // Evaluate velocity over the most recent active window before release
    const velocity = estimateVelocity(s.samples, last?.t ?? now);
    const offset = resolveOffset(s.raw, s.index, order.length, s.width);
    const target = decideTarget({ offset, velocity, width: s.width, index: s.index, count: order.length });
    const targetId = order[target]?.id;
    if (target === s.index || !targetId || targetId === activeId) {
      if (s.reduced) rest();
      else springBack();
      return;
    }
    commitTo(target, targetId);
  };

  const onWheel = (e: WheelEvent) => {
    const o = optsRef.current;
    if (e.ctrlKey || e.deltaMode !== 0) return; // pinch zoom / line-based wheels
    if ((e.target as Element | null)?.closest?.('.nsb-switcher')) return; // the icon row scrolls itself
    const now = performance.now();
    const dt = now - s.lastAt;
    s.lastAt = now;
    const horizontalish = Math.abs(e.deltaX) >= Math.abs(e.deltaY);

    if (s.axis === 'spent') {
      // Momentum of a finished gesture: prevent page back/forward navigation
      if (horizontalish) e.preventDefault();
      // A new swipe starts after a pause; the decaying momentum of the one
      // just committed never does (it would skip a second space).
      if (dt > GESTURE_END_MS) {
        s.axis = null;
        s.accX = 0;
        s.accY = 0;
      } else {
        return;
      }
    }

    if (dt > GESTURE_END_MS && s.phase === 'idle') {
      s.axis = null;
      s.accX = 0;
      s.accY = 0;
    }

    if (s.axis === 'v') return;

    // Fluid interruption: if user starts a horizontal swipe while the track is animating, catch it immediately!
    if (s.phase === 'anim' && horizontalish) {
      stopAnim();
      s.phase = 'drag';
      s.raw = s.offset;
      s.axis = 'h';
      s.samples = [{ t: now, x: s.raw }];
      s.vHist = [];
      o.onStart();
    }

    if (s.axis === null) {
      if (o.blocked) return;
      s.accX += e.deltaX;
      s.accY += e.deltaY;
      const axis = lockAxis(s.accX, s.accY);
      if (axis === 'v') {
        s.axis = 'v';
        return;
      }
      if (horizontalish) e.preventDefault();
      if (axis === null) return;
      if (s.phase !== 'idle' && s.phase !== 'drag') {
        s.axis = 'spent';
        return;
      }
      // Horizontal lock: the gesture starts here.
      const idx = o.order.findIndex((x) => x.id === o.activeId);
      s.axis = 'h';
      s.phase = 'drag';
      s.index = Math.max(0, idx);
      const measuredWidth =
        o.trackRef.current?.parentElement?.clientWidth || o.trackRef.current?.clientWidth || 260;
      s.width = measuredWidth;
      s.reduced = reducedMotion();
      s.raw = -s.accX;
      s.samples = [];
      s.vHist = [];
      o.onStart();
    } else {
      s.raw -= e.deltaX;
      // Bound accumulated raw displacement to avoid deadzones on direction reversal
      const [minRaw, maxRaw] = rawBounds(s.index, o.order.length, s.width);
      s.raw = Math.max(minRaw, Math.min(maxRaw, s.raw));
    }
    e.preventDefault();
    s.samples.push({ t: now, x: s.raw });
    s.vHist.push({ t: now, v: estimateVelocity(s.samples, now) });
    if (s.samples.length > 40) s.samples.shift();
    if (s.vHist.length > 40) s.vHist.shift();
    if (s.phase === 'drag' && maybeCommitEarly(now)) return;
    if (!s.reduced && !s.frame) {
      s.frame = requestAnimationFrame(() => {
        s.frame = 0;
        if (s.phase === 'drag') apply(resolveOffset(s.raw, s.index, optsRef.current.order.length, s.width));
      });
    }
    window.clearTimeout(s.endTimer);
    s.endTimer = window.setTimeout(() => {
      finishGesture();
    }, GESTURE_END_MS);
  };

  useEffect(() => {
    const root = opts.rootRef.current;
    if (!root) return;
    root.addEventListener('wheel', onWheel, { passive: false });
    return () => {
      root.removeEventListener('wheel', onWheel);
      stopAnim();
      window.clearTimeout(s.endTimer);
      window.clearTimeout(s.pendingTimer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The active space changed. Ours (swipe commit): drop the transform in the same
  // commit that re-centres the panes, so nothing visibly moves. Anyone else's
  // (icon click, Ctrl+N): slide from the old space.
  useLayoutEffect(() => {
    const prev = s.prevActive;
    const { activeId, order, trackRef } = optsRef.current;
    s.prevActive = activeId;
    if (prev === activeId) return;
    if ((s.phase === 'pending' || s.phase === 'anim') && s.pendingId === activeId) {
      // Our own commit: the panes now centre on the new space, so the same
      // on-screen position is one pane closer. Keep gliding from there.
      setOutgoing(null);
      if (s.phase === 'anim' && s.settle !== 0) {
        stopAnim();
        apply(s.offset - s.settle);
        s.settle = 0;
        s.pendingId = null;
        animate(0, rest);
      } else {
        rest();
      }
      return;
    }
    stopAnim();
    window.clearTimeout(s.endTimer);
    window.clearTimeout(s.pendingTimer);
    s.axis = null;
    const iNew = order.findIndex((x) => x.id === activeId);
    const iOld = order.findIndex((x) => x.id === prev);
    const track = trackRef.current;
    const measuredWidth = track?.parentElement?.clientWidth || track?.clientWidth || 260;
    s.width = measuredWidth;
    if (!track || iNew < 0 || iOld < 0 || reducedMotion() || measuredWidth <= 0) {
      rest();
      setOutgoing(null);
      return;
    }
    const dir = iNew > iOld ? 1 : -1;
    s.otherColor = order[iOld].color;
    s.phase = 'anim';
    setOutgoing({ id: prev, side: dir > 0 ? 'prev' : 'next' });
    apply(dir * s.width);
    animate(0, () => {
      rest();
      setOutgoing(null);
    });
  }, [opts.activeId]);

  return outgoing;
}
