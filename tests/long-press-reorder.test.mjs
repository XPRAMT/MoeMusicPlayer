import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { exceedsLongPressSlop, reduceLongPressPhase, shouldBlockTouchScroll, LONG_PRESS_REORDER_SLOP_PX } from '../src/lib/long-press-reorder.js';

test('slop: small jitter keeps the long-press pending, larger movement cancels it', () => {
  assert.equal(exceedsLongPressSlop(3, 4), false);
  assert.equal(exceedsLongPressSlop(LONG_PRESS_REORDER_SLOP_PX, 0), false);
  assert.equal(exceedsLongPressSlop(6, 6), true);
});

test('quick swipe: moving beyond slop before the timer returns to idle (native scroll), timer then does nothing', () => {
  let phase = reduceLongPressPhase('idle', { type: 'down' });
  phase = reduceLongPressPhase(phase, { type: 'move', dx: 0, dy: 30 });
  assert.equal(phase, 'idle');
  assert.equal(shouldBlockTouchScroll(phase), false);
  assert.equal(reduceLongPressPhase(phase, { type: 'timer' }), 'idle');
});

test('long-press: timer fires within slop, then drag blocks touch scrolling until release', () => {
  let phase = reduceLongPressPhase('idle', { type: 'down' });
  phase = reduceLongPressPhase(phase, { type: 'move', dx: 2, dy: 1 });
  assert.equal(shouldBlockTouchScroll(phase), false, 'pending still lets the browser decide');
  phase = reduceLongPressPhase(phase, { type: 'timer' });
  assert.equal(phase, 'dragging');
  phase = reduceLongPressPhase(phase, { type: 'move', dx: 0, dy: 120 });
  assert.equal(shouldBlockTouchScroll(phase), true);
  assert.equal(reduceLongPressPhase(phase, { type: 'up' }), 'idle');
  assert.equal(reduceLongPressPhase('dragging', { type: 'cancel' }), 'idle');
});

test('queue and track-column settings share the guard and keep two-axis native pans', () => {
  const list = readFileSync(new URL('../src/lib/PagedVirtualList.svelte', import.meta.url), 'utf8');
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  const css = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');
  assert.match(list, /use:longPressDragGuard=\{\(\) => dragFromIndex !== null\}/);
  assert.match(app, /use:longPressDragGuard=\{\(\) => trackColumnOrderDrag !== null\}/);
  assert.match(list, /\.paged-virtual-row\.reorderable \{\s*touch-action: pan-x pan-y;/);
  assert.match(css, /\.track-column-setting-row \{[^}]*touch-action: pan-x pan-y;/);
  const guard = readFileSync(new URL('../src/lib/long-press-reorder.js', import.meta.url), 'utf8');
  assert.match(guard, /addEventListener\('touchmove', onTouchMove, \{ passive: false \}\)/);
});
