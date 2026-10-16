import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { getSwipeActions, resolveSwipeDirection, getSwipeOffset, getSwipeVelocity, isSwipeAxisForDirection, isSwipePointer, resolveSwipeAxis, shouldDismissSwipe, SWIPE_IGNORE_SELECTOR } from '../src/lib/swipe-dismiss.js';

test('axis lock waits for slop and picks the dominant axis', () => {
  assert.equal(resolveSwipeAxis(3, 4), null);
  assert.equal(resolveSwipeAxis(-20, 5), 'x');
  assert.equal(resolveSwipeAxis(4, 30), 'y');
  assert.equal(isSwipeAxisForDirection('left', 'x'), true);
  assert.equal(isSwipeAxisForDirection('left', 'y'), false);
  assert.equal(isSwipeAxisForDirection('down', 'y'), true);
});

test('offset only counts movement in the dismiss direction', () => {
  assert.equal(getSwipeOffset('left', -50, 3), 50);
  assert.equal(getSwipeOffset('left', 40, 0), 0);
  assert.equal(getSwipeOffset('right', 40, 0), 40);
  assert.equal(getSwipeOffset('down', 9, 120), 120);
  assert.equal(getSwipeOffset('down', 0, -30), 0);
});

test('velocity uses the last samples, signed by direction', () => {
  assert.equal(getSwipeVelocity('left', { x: 100, y: 0, t: 0 }, { x: 80, y: 0, t: 10 }), 2);
  assert.equal(getSwipeVelocity('right', { x: 100, y: 0, t: 0 }, { x: 80, y: 0, t: 10 }), -2);
  assert.equal(getSwipeVelocity('down', { x: 0, y: 0, t: 5 }, { x: 0, y: 10, t: 5 }), 0);
});

test('dismiss past 30% of size or above the velocity threshold, otherwise snap back', () => {
  assert.equal(shouldDismissSwipe({ offsetPx: 90, sizePx: 300, velocityPxPerMs: 0 }), true);
  assert.equal(shouldDismissSwipe({ offsetPx: 89, sizePx: 300, velocityPxPerMs: 0.2 }), false);
  assert.equal(shouldDismissSwipe({ offsetPx: 20, sizePx: 300, velocityPxPerMs: 0.8 }), true);
  assert.equal(shouldDismissSwipe({ offsetPx: 0, sizePx: 300, velocityPxPerMs: 3 }), false);
});

test('only touch and pen start a swipe; sliders and fields are ignored', () => {
  assert.equal(isSwipePointer('touch'), true);
  assert.equal(isSwipePointer('pen'), true);
  assert.equal(isSwipePointer('mouse'), false);
  assert.match(SWIPE_IGNORE_SELECTOR, /input/);
  assert.match(SWIPE_IGNORE_SELECTOR, /role="slider"/);
});

test('nav panel, quick settings and cover are wired with the shared helper', () => {
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  assert.match(app, /use:swipeDismiss=\{\{ direction: 'left'[^}]*closeNavPanel/);
  assert.match(app, /use:swipeDismiss=\{\{ direction: 'right'[^}]*closeQuickSettings/);
  // Cover: still image, left = previous, right = next (as the user specified), down = close.
  const cover = app.match(/use:swipeDismiss=\{\{ follow: false[^\n]*/)?.[0] ?? '';
  assert.match(cover, /left: \(\) => void controlPlayback\('playback_previous'\)/);
  assert.match(cover, /right: \(\) => void controlPlayback\('playback_next'\)/);
  assert.match(cover, /down: \(\) => void closeNowPlaying\(\)/);
});

test('locked swipe resolves its direction from the locked axis sign', () => {
  assert.equal(resolveSwipeDirection('x', -40, 10), 'left');
  assert.equal(resolveSwipeDirection('x', 40, -10), 'right');
  assert.equal(resolveSwipeDirection('y', 5, 60), 'down');
  assert.equal(resolveSwipeDirection('y', 5, -60), 'up');
});

test('single-direction and multi-action options map to handlers; missing directions are ignored', () => {
  const close = () => {};
  assert.deepEqual(Object.keys(getSwipeActions({ direction: 'left', onDismiss: close })), ['left']);
  const prev = () => {}; const next = () => {};
  const actions = getSwipeActions({ follow: false, actions: { left: prev, right: next, down: close } });
  assert.equal(actions.left, prev);
  assert.equal(actions.right, next);
  assert.equal(actions.up, undefined);
  // Non-following swipes still use the same release thresholds.
  assert.equal(shouldDismissSwipe({ offsetPx: 100, sizePx: 300, velocityPxPerMs: 0 }), true);
  assert.equal(shouldDismissSwipe({ offsetPx: 40, sizePx: 300, velocityPxPerMs: 0.1 }), false);
});
