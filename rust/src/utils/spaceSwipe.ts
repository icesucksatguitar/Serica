// Pure gesture math for the Arc-style swipe between spaces. No DOM, no React.
//
// Sign convention: `offset` is the horizontal translation of the pane track in
// px. Negative moves content left (towards the NEXT space), positive moves it
// right (towards the PREVIOUS space). Wheel `deltaX > 0` is a swipe towards the
// next space, so `offset = -sum(deltaX)`.

/** No wheel events for this long ends the gesture. */
export const GESTURE_END_MS = 140;
/** Duration of the snap / spring-back animation. */
export const SNAP_MS = 220;
/** Fraction of the pane width that commits a swipe. */
export const COMMIT_FRACTION = 0.3;
/** Release velocity (px/ms) that counts as a flick. */
export const FLICK_VELOCITY = 0.55;
/** A flick must have moved at least this far (px) to count. */
export const FLICK_MIN_OFFSET = 20;
/** Accumulated movement (px) needed before the axis is decided. */
export const AXIS_LOCK_PX = 6;
/** Samples older than this (ms) are ignored for the release velocity. */
export const VELOCITY_WINDOW_MS = 100;

export type SwipeAxis = 'h' | 'v';

/** Decides the gesture axis from accumulated deltas; `null` while still too small. */
export function lockAxis(accX: number, accY: number): SwipeAxis | null {
  if (Math.max(Math.abs(accX), Math.abs(accY)) < AXIS_LOCK_PX) return null;
  return Math.abs(accX) > Math.abs(accY) ? 'h' : 'v';
}

/** Diminishing-returns resistance: approaches `width` asymptotically, never reaches it. */
export function rubberBand(distance: number, width: number): number {
  if (width <= 0) return 0;
  const d = Math.abs(distance);
  const out = width * (1 - 1 / ((0.55 * d) / width + 1));
  return distance < 0 ? -out : out;
}

/** Furthest the track moves when pulled past the first or last space (px). */
export const EDGE_PULL_MAX = 48;

/**
 * Pull past the first or last space: a short, stiff bounce that never shows
 * more than a sliver of the empty side (at most EDGE_PULL_MAX, or 18% of a
 * narrow pane), so it reads as "no more spaces" rather than an empty pane.
 */
export function edgePull(distance: number, width: number): number {
  const limit = Math.min(EDGE_PULL_MAX, 0.18 * width);
  if (limit <= 0) return 0;
  const d = Math.abs(distance);
  const out = limit * (1 - 1 / (d / (2 * limit) + 1));
  return distance < 0 ? -out : out;
}

/**
 * Track offset for a raw (1:1) offset: clamped to one pane either way, with
 * a short bounce when pulling past the first or last space.
 */
export function resolveOffset(raw: number, index: number, count: number, width: number): number {
  if (width <= 0) return 0;
  const clamped = Math.max(-width, Math.min(width, raw));
  if (clamped > 0 && index <= 0) return edgePull(clamped, width);
  if (clamped < 0 && index >= count - 1) return edgePull(clamped, width);
  return clamped;
}

/**
 * Bounds for the raw gesture offset. Towards an existing space it may go a
 * little past one pane; past the first or last space it stops at one pane, so
 * pulling further and then reversing has no dead stretch to undo first.
 */
export function rawBounds(index: number, count: number, width: number): [number, number] {
  const w = width || 260;
  const far = w * 1.35;
  return [index >= count - 1 ? -w : -far, index <= 0 ? w : far];
}

/** Index the track should settle on after release. Never wraps. */
export function decideTarget(args: {
  offset: number;
  velocity: number;
  width: number;
  index: number;
  count: number;
}): number {
  const { offset, velocity, width, index, count } = args;
  if (offset === 0 || width <= 0) return index;
  const dir = offset < 0 ? 1 : -1;
  const movedFar = Math.abs(offset) >= COMMIT_FRACTION * width;
  const flicked =
    Math.abs(velocity) >= FLICK_VELOCITY &&
    Math.sign(velocity) === Math.sign(offset) &&
    Math.abs(offset) >= FLICK_MIN_OFFSET;
  if (!movedFar && !flicked) return index;
  return Math.max(0, Math.min(count - 1, index + dir));
}

export interface OffsetSample {
  t: number;
  x: number;
}

/** Velocity (px/ms) over the most recent samples; 0 when there is too little data. */
export function estimateVelocity(samples: readonly OffsetSample[], now: number): number {
  const recent = samples.filter((s) => now - s.t <= VELOCITY_WINDOW_MS);
  if (recent.length < 2) return 0;
  const first = recent[0];
  const last = recent[recent.length - 1];
  const dt = last.t - first.t;
  return dt > 0 ? (last.x - first.x) / dt : 0;
}

export function easeOutCubic(p: number): number {
  const c = Math.max(0, Math.min(1, p));
  return 1 - Math.pow(1 - c, 3);
}

/** Final track offset for a settled target: 0 stays, otherwise one pane over. */
export function settleOffset(targetIndex: number, index: number, width: number): number {
  return targetIndex === index ? 0 : targetIndex > index ? -width : width;
}

// ---------------------------------------------------------------------------
// A new swipe during the momentum of the last one.
//
// Once a swipe has switched spaces, the rest of its wheel events (the fingers
// still moving, then the touchpad's inertia) must not start another swipe, or
// one flick would skip two spaces. Windows wheel events carry no gesture
// phase, and the inertia runs for half a second or more with no pause, so a
// second swipe made during it used to be ignored: switching spaces quickly
// felt delayed. Inertia only ever decays; a new swipe pushes the deltas back
// up (or the other way). So a new swipe is one whose deltas, after having
// decayed to a trough, rise well above it for a few events in a row, or that
// clearly reverses direction.

export interface MomentumTail {
  /** Direction of the committed swipe (sign of deltaX). */
  sign: number;
  /** Largest |deltaX| seen since the commit. */
  peak: number;
  /** Smallest |deltaX| since the peak. */
  trough: number;
  /** Rising (or reversed) events in a row. */
  streak: number;
}

export function startTail(sign: number, lastMag: number): MomentumTail {
  const m = Math.abs(lastMag);
  return { sign: Math.sign(sign) || 1, peak: m, trough: m, streak: 0 };
}

/** Events in a row that must look like a new swipe. */
const NEW_SWIPE_EVENTS = 3;

/** Feeds one wheel delta of the spent gesture; `restart` when a new swipe began. */
export function feedTail(t: MomentumTail, dx: number): { tail: MomentumTail; restart: boolean } {
  const mag = Math.abs(dx);
  const sign = Math.sign(dx);
  let { peak, trough, streak } = t;
  if (sign !== 0 && sign !== t.sign && mag >= 4) {
    // The other way: a swipe back.
    streak = streak > 0 && t.trough === -1 ? streak + 1 : 1;
    return { tail: { ...t, trough: -1, streak }, restart: streak >= NEW_SWIPE_EVENTS - 1 };
  }
  if (trough === -1) trough = mag; // reversal streak broken: back to tracking decay
  if (mag > peak) {
    // Still speeding up (the fingers of the same swipe): not a trough yet.
    return { tail: { ...t, peak: mag, trough: mag, streak: 0 }, restart: false };
  }
  const decayed = trough <= peak * 0.5;
  if (decayed && mag >= Math.max(trough * 2, trough + 6)) {
    streak += 1;
  } else {
    streak = 0;
    trough = Math.min(trough, mag);
  }
  return { tail: { ...t, peak, trough, streak }, restart: streak >= NEW_SWIPE_EVENTS };
}
