import assert from 'node:assert/strict';
import test from 'node:test';
import { createUpdateController, EMPTY_UPDATE_STATE, formatBuildTime, updateProgressPercent } from '../src/lib/update-controller.ts';

const latest = { buildId: '20261009T120000Z', version: '0.1.0+20261009T120000Z', publishedAt: '2026-10-09T12:00:00Z', notes: '', downloadBytes: 100 };
const state = (status, extra = {}) => ({ ...EMPTY_UPDATE_STATE, status, latest, ...extra });
function fixture(overrides = {}) {
  const calls = [];
  let receive;
  let view;
  const controller = createUpdateController({
    subscribe: async (callback) => { calls.push('subscribe'); receive = callback; return () => calls.push('unsubscribe'); },
    getState: async () => { calls.push('get'); return state('idle'); },
    check: async () => { calls.push('check'); return state('available'); },
    ignore: async () => { calls.push('ignore'); return state('idle'); },
    download: async (mode) => { calls.push(mode); return state('deferred'); },
    onChange: (next, visible) => { view = { state: next, visible }; },
    ...overrides,
  });
  return { controller, calls, receive: (next) => receive(next), view: () => view };
}

test('startup subscribes before checking, runs once and manual check shows ignored version again', async () => {
  const f = fixture();
  await Promise.all([f.controller.start(), f.controller.start()]);
  assert.deepEqual(f.calls, ['subscribe', 'get', 'check']);
  assert.equal(f.view().visible, true);
  await f.controller.ignore();
  assert.equal(f.view().visible, false);
  await f.controller.check();
  assert.equal(f.view().visible, true);
  f.controller.dispose();
  assert.equal(f.calls.at(-1), 'unsubscribe');
});

test('no published release and check failures do not open an install notification', async () => {
  const f = fixture({ check: async () => state('unpublished', { latest: null }) });
  await f.controller.start();
  assert.equal(f.view().visible, false);
  assert.equal(f.view().state.status, 'unpublished');
  const failed = fixture({ check: async () => { throw new Error('offline'); } });
  await failed.controller.start();
  assert.equal(failed.view().visible, false);
  assert.equal(failed.view().state.error, 'offline');
});

test('download errors retain release and notification for retry; download progress can be dismissed without ignoring backend work', async () => {
  let fail = true;
  let finish;
  const f = fixture({ download: async () => {
    if (fail) { fail = false; throw new Error('network interrupted'); }
    return await new Promise((resolve) => { finish = resolve; });
  } });
  await f.controller.start();
  await f.controller.download('nextLaunch');
  assert.equal(f.view().visible, true);
  assert.equal(f.view().state.error, 'network interrupted');
  assert.equal(f.view().state.latest.buildId, latest.buildId);
  const pending = f.controller.download('nextLaunch');
  f.receive(state('downloading', { downloadedBytes: 43, totalBytes: 100 }));
  assert.equal(updateProgressPercent(f.view().state), 43);
  await f.controller.ignore();
  assert.equal(f.view().visible, false);
  assert.equal(f.calls.includes('ignore'), false);
  f.controller.show();
  assert.equal(f.view().visible, true);
  finish(state('deferred'));
  await pending;
  assert.equal(f.view().state.status, 'deferred');
  assert.equal(f.view().visible, false);
});

test('new events win over an older get-state response and disposal unsubscribes late listeners', async () => {
  let finish;
  const f = fixture({ getState: () => new Promise((resolve) => { finish = resolve; }) });
  const started = f.controller.start();
  await new Promise((resolve) => setTimeout(resolve, 0));
  f.receive(state('deferred'));
  finish(state('idle'));
  await started;
  assert.equal(f.view().state.status, 'deferred');
  assert.equal(f.calls.includes('check'), false);
  let resolveListener;
  let stopped = false;
  const late = fixture({ subscribe: () => new Promise((resolve) => { resolveListener = resolve; }) });
  const registration = late.controller.start();
  late.controller.dispose();
  resolveListener(() => { stopped = true; });
  await registration;
  assert.equal(stopped, true);
});

test('build timestamps require an explicit timezone and display Taipei local time', () => {
  assert.equal(formatBuildTime('2026-10-09T12:00:00'), '未提供');
  assert.match(formatBuildTime('2026-10-09T12:00:00Z', 'Asia/Taipei'), /20:00:00.*GMT\+8/);
  assert.equal(updateProgressPercent(state('downloading', { totalBytes: null })), null);
});
