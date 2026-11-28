// Tests for telling a new trackpad swipe from the inertia of the last one.
// Run: node tools/tests/run.mjs
import assert from 'node:assert/strict';
import { feedTail, startTail } from '../../src/utils/spaceSwipe';

let passed = 0;
const test = (name: string, fn: () => void) => {
  fn();
  passed++;
  console.log('ok -', name);
};
const run = (deltas: number[], sign = 1, last = 28) => {
  let t = startTail(sign, last);
  for (let i = 0; i < deltas.length; i++) {
    const r = feedTail(t, deltas[i]);
    t = r.tail;
    if (r.restart) return i;
  }
  return -1;
};
const decay = (from: number, k = 0.86, min = 1) => {
  const out: number[] = [];
  for (let v = from; v > min; v *= k) out.push(Math.round(v));
  return out;
};

test('inertia alone never starts a swipe', () => {
  assert.equal(run(decay(30)), -1);
  assert.equal(run(decay(60, 0.93)), -1);
});

test('the same fingers moving on after an early switch never start one', () => {
  // Steady or speeding up, with jitter, then inertia.
  assert.equal(run([28, 30, 27, 31, 29, 33, 30, 28, 26, ...decay(26)]), -1);
  assert.equal(run([20, 24, 30, 36, 40, 38, 35, 30, ...decay(30)]), -1);
});

test('a new swipe during the inertia is seen within a few events', () => {
  const at = run([...decay(30).slice(0, 10), 8, 16, 24, 28, 28]);
  assert.ok(at >= 0 && at <= 13, `restart at ${at}`);
});

test('a swipe back is seen, small jitter the other way is not', () => {
  assert.equal(run([...decay(30), -1, -2, -1, 1]), -1);
  const at = run([...decay(30).slice(0, 6), -10, -20, -26]);
  assert.ok(at >= 0, 'reversal restarts');
});

console.log(`\n${passed} passed`);
