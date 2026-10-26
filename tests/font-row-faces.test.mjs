import assert from 'node:assert/strict';
import test from 'node:test';
import { FONT_ROW_MAX_LOADS, FONT_ROW_SETTLE_MS, createFontRowScheduler, fontRowSource } from '../src/lib/font-row-faces.js';

/** A scheduler with a manual clock and loads the test resolves itself. */
function harness() {
  let now = 0;
  const timers = [];
  const loads = [];
  const loadedEvents = [];
  const scheduler = createFontRowScheduler({
    load: (row) => new Promise((resolve, reject) => loads.push({ key: row.key, resolve, reject })),
    onLoaded: (key, face) => loadedEvents.push([key, face]),
    setTimer: (callback, ms) => {
      const timer = { at: now + ms, callback, cleared: false };
      timers.push(timer);
      return timer;
    },
    clearTimer: (timer) => {
      timer.cleared = true;
    },
  });
  const advance = (ms) => {
    now += ms;
    for (const timer of timers.filter((t) => !t.cleared && t.at <= now)) {
      timer.cleared = true;
      timer.callback();
    }
  };
  const flush = () => new Promise((resolve) => setImmediate(resolve));
  const row = (n) => ({ key: `subset:${n}`, source: `url("${n}")` });
  return { scheduler, advance, flush, loads, loadedEvents, row };
}

test('a row loads its face only after staying in view for the settle delay', async () => {
  const { scheduler, advance, flush, loads, row } = harness();
  scheduler.show(row(1));
  advance(FONT_ROW_SETTLE_MS - 1);
  await flush();
  assert.equal(loads.length, 0, 'not yet');
  advance(1);
  await flush();
  assert.deepEqual(loads.map((load) => load.key), ['subset:1']);
});

test('rows scrolled away before their turn are skipped', async () => {
  const { scheduler, advance, flush, loads, row } = harness();
  scheduler.show(row(1));
  advance(50);
  scheduler.hide('subset:1');
  advance(200);
  await flush();
  assert.equal(loads.length, 0, 'left before the delay');
  // Queued behind the four running loads, then scrolled away: never loaded.
  for (const n of [2, 3, 4, 5, 6, 7]) scheduler.show(row(n));
  advance(FONT_ROW_SETTLE_MS);
  await flush();
  assert.deepEqual(loads.map((load) => load.key), ['subset:2', 'subset:3', 'subset:4', 'subset:5']);
  scheduler.hide('subset:6');
  loads[0].resolve('MoeFontRow 1');
  await flush();
  await flush();
  assert.deepEqual(loads.map((load) => load.key).slice(4), ['subset:7']);
});

test(`at most ${FONT_ROW_MAX_LOADS} faces load at once`, async () => {
  const { scheduler, advance, flush, loads, row } = harness();
  for (let n = 0; n < 10; n += 1) scheduler.show(row(n));
  advance(FONT_ROW_SETTLE_MS);
  await flush();
  assert.equal(loads.length, FONT_ROW_MAX_LOADS);
  assert.equal(scheduler.running, FONT_ROW_MAX_LOADS);
  loads[1].reject(new Error('bad font'));
  await flush();
  await flush();
  assert.equal(loads.length, FONT_ROW_MAX_LOADS + 1, 'a finished (even failed) load lets the next start');
});

test('a loaded face is remembered and never loaded again; a failed one is not retried', async () => {
  const { scheduler, advance, flush, loads, loadedEvents, row } = harness();
  scheduler.show(row(1));
  scheduler.show(row(2));
  advance(FONT_ROW_SETTLE_MS);
  await flush();
  loads[0].resolve('MoeFontRow 1');
  loads[1].reject(new Error('bad'));
  await flush();
  await flush();
  assert.deepEqual(loadedEvents, [['subset:1', 'MoeFontRow 1']]);
  assert.equal(scheduler.loaded('subset:1'), 'MoeFontRow 1');
  scheduler.hide('subset:1');
  scheduler.hide('subset:2');
  scheduler.show(row(1));
  scheduler.show(row(2));
  advance(FONT_ROW_SETTLE_MS * 3);
  await flush();
  assert.equal(loads.length, 2, 'no new loads');
});

test('a face shown by two pickers stays wanted until both hide it', async () => {
  const { scheduler, advance, flush, loads, row } = harness();
  scheduler.show(row(1));
  scheduler.show(row(1));
  scheduler.hide('subset:1');
  advance(FONT_ROW_SETTLE_MS);
  await flush();
  assert.equal(loads.length, 1);
});

test('a row loads its subset, or the installed family by name without one', () => {
  const toUrl = (path) => `http://asset.localhost/${encodeURIComponent(path)}`;
  assert.deepEqual(fontRowSource({ family: 'Microsoft JhengHei', subset: 'D:/s/ab.ttf' }, toUrl), {
    key: 'subset:D:/s/ab.ttf',
    source: 'url("http://asset.localhost/D%3A%2Fs%2Fab.ttf")',
  });
  assert.deepEqual(fontRowSource({ family: '文鼎新藝體' }, toUrl), {
    key: 'local:文鼎新藝體',
    source: 'local("文鼎新藝體"), local("文鼎新藝體 Regular")',
  });
  assert.equal(fontRowSource(null, toUrl), null);
});
