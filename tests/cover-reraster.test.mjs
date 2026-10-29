import assert from 'node:assert/strict';
import test from 'node:test';
import {
  COVER_RERASTER_SCALE_EPSILON,
  createRerasterState,
  invalidateReraster,
  isRerasterTranslate,
  markRerastered,
  observeRerasterScale,
  planReraster,
} from '../src/lib/cover-reraster.js';

const idleMs = 50;
const ready = (state, now) => planReraster(state, { now, idleMs, busy: false, inFlight: false });

test('no nudge at scale 1 or below, however long it is stable', () => {
  for (const scale of [1, 0.5, 0.9999]) {
    const state = observeRerasterScale(createRerasterState(1, 0), scale, 10);
    assert.equal(ready(state, 10_000), 'skip', `scale ${scale}`);
  }
});

test('a nudge is due only after the scale above 1 has been stable for the idle time', () => {
  let state = observeRerasterScale(createRerasterState(1, 0), 3, 100);
  assert.equal(ready(state, 100), 'wait');
  assert.equal(ready(state, 149), 'wait');
  assert.equal(ready(state, 150), 'run');
  // A change larger than the epsilon restarts the clock; a smaller one does not.
  state = observeRerasterScale(state, 3 + COVER_RERASTER_SCALE_EPSILON / 2, 140);
  assert.equal(ready(state, 150), 'run', 'within the epsilon: still the same stable scale');
  state = observeRerasterScale(state, 3.01, 140);
  assert.equal(ready(state, 150), 'wait');
  assert.equal(ready(state, 190), 'run');
});

test('a gesture, swipe feedback or a nudge in flight postpones it', () => {
  const state = observeRerasterScale(createRerasterState(1, 0), 2, 0);
  assert.equal(planReraster(state, { now: 1000, idleMs, busy: true, inFlight: false }), 'wait');
  assert.equal(planReraster(state, { now: 1000, idleMs, busy: false, inFlight: true }), 'wait');
  assert.equal(planReraster(state, { now: 1000, idleMs, busy: false, inFlight: false }), 'run');
});

test('one nudge per settled scale; a later zoom change nudges again; coming back to rest forgets it', () => {
  let state = observeRerasterScale(createRerasterState(1, 0), 2, 0);
  assert.equal(ready(state, 100), 'run');
  state = markRerastered(state);
  assert.equal(ready(state, 10_000), 'skip', 'nothing more at the same scale (also after a pan)');
  state = observeRerasterScale(state, 4, 200);
  assert.equal(ready(state, 220), 'wait');
  assert.equal(ready(state, 260), 'run', 'after a zoom change once stable again');
  state = markRerastered(state);
  // Zoom out to rest and in to the very same scale: nudged again.
  state = observeRerasterScale(state, 1, 300);
  state = observeRerasterScale(state, 4, 310);
  assert.equal(ready(state, 400), 'run');
});

test('a new cover image (track change) asks for a nudge at the same scale', () => {
  let state = markRerastered(observeRerasterScale(createRerasterState(1, 0), 3, 0));
  assert.equal(ready(state, 500), 'skip');
  state = invalidateReraster(state, 500);
  assert.equal(ready(state, 520), 'wait');
  assert.equal(ready(state, 560), 'run');
});

test('the nudge translate is recognised in both serialisations and nothing else', () => {
  assert.equal(isRerasterTranslate('0.01px 0px'), true);
  assert.equal(isRerasterTranslate('0.01px'), true);
  assert.equal(isRerasterTranslate('0.01px 0'), true);
  assert.equal(isRerasterTranslate(''), false);
  assert.equal(isRerasterTranslate(undefined), false);
  assert.equal(isRerasterTranslate('12px'), false);
  assert.equal(isRerasterTranslate('0.01px 3px'), false);
  assert.equal(isRerasterTranslate('0.02px'), false);
});
