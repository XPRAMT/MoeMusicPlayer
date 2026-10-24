import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import {
  classifyStartError,
  isRemoteUnreachableMessage,
  readStartFailure,
  shouldShowStartNotice,
  snapshotReportsNewUnreachable,
} from '../src/lib/start-failure-notice.js';

test('only unreachable-server messages are recognised as such', () => {
  assert.equal(isRemoteUnreachableMessage('伺服器無法連線'), true);
  assert.equal(isRemoteUnreachableMessage('無法開啟遠端檔案：無法連線到伺服器：Transport error'), true);
  assert.equal(isRemoteUnreachableMessage('superseded'), false);
  assert.equal(isRemoteUnreachableMessage('找不到檔案'), false);
  assert.equal(isRemoteUnreachableMessage(null), false);
});

test('a rejected command is an unreachable server, an unplayable song, a replaced choice, or an ordinary error', () => {
  assert.equal(classifyStartError('伺服器無法連線'), 'unreachable');
  assert.equal(classifyStartError('無法連線到伺服器：No route to host'), 'unreachable');
  assert.equal(classifyStartError('無法播放這首歌'), 'unplayable');
  assert.equal(classifyStartError('superseded'), 'superseded');
  assert.equal(classifyStartError('load superseded'), 'superseded');
  assert.equal(classifyStartError('音訊服務忙碌，請稍後再試。'), null);
  assert.equal(classifyStartError(undefined), null);
});

test('the markers match the stable texts of the Rust side', () => {
  const rust = readFileSync(new URL('../crates/player-core/src/track_start.rs', import.meta.url), 'utf8');
  const unplayable = rust.match(/pub const TRACK_UNPLAYABLE: &str = "([^"]+)";/)?.[1];
  assert.equal(classifyStartError(unplayable), 'unplayable');
  const remote = readFileSync(new URL('../crates/player-core/src/remote.rs', import.meta.url), 'utf8');
  const linkLost = remote.match(/pub const REMOTE_LINK_LOST: &str = "([^"]+)";/)?.[1];
  const unreachable = remote.match(/pub const REMOTE_UNREACHABLE: &str = "([^"]+)";/)?.[1];
  const superseded = remote.match(/pub const LOAD_SUPERSEDED: &str = "([^"]+)";/)?.[1];
  assert.equal(classifyStartError(linkLost), 'unreachable');
  assert.equal(classifyStartError(unreachable), 'unreachable');
  assert.equal(classifyStartError(superseded), 'superseded');
});

test('a snapshot reports a stream failure only when the error text becomes an unreachable one', () => {
  assert.equal(snapshotReportsNewUnreachable(null, '伺服器無法連線'), true);
  assert.equal(snapshotReportsNewUnreachable('伺服器無法連線', '伺服器無法連線'), false);
  assert.equal(snapshotReportsNewUnreachable('伺服器無法連線', null), false);
  assert.equal(snapshotReportsNewUnreachable(null, '解碼失敗'), false);
});

test('a failed start in a snapshot is new once, whichever way the start was asked for', () => {
  // The first snapshot only sets the baseline: a reloaded page does not replay an old failure.
  assert.deepEqual(readStartFailure(null, { kind: 'unplayable', sequence: 4 }), { sequence: 4, kind: null });
  assert.deepEqual(readStartFailure(null, null), { sequence: 0, kind: null });
  // A higher number is a new failure of that kind.
  assert.deepEqual(readStartFailure(4, { kind: 'unreachable', sequence: 5 }), { sequence: 5, kind: 'unreachable' });
  assert.deepEqual(readStartFailure(5, { kind: 'unplayable', sequence: 6 }), { sequence: 6, kind: 'unplayable' });
  // The same number again (the next poll), or none, is nothing new.
  assert.deepEqual(readStartFailure(5, { kind: 'unreachable', sequence: 5 }), { sequence: 5, kind: null });
  assert.deepEqual(readStartFailure(5, null), { sequence: 5, kind: null });
  assert.deepEqual(readStartFailure(5, undefined), { sequence: 5, kind: null });
  // An unknown kind is not shown.
  assert.deepEqual(readStartFailure(5, { kind: 'other', sequence: 6 }), { sequence: 6, kind: null });
});

test('the same attempt reported twice within two seconds shows one notice', () => {
  assert.equal(shouldShowStartNotice(null, 100), true);
  assert.equal(shouldShowStartNotice(100, 1500), false);
  assert.equal(shouldShowStartNotice(100, 2100), true);
});

test('App shows the notice for the newest command error and for a new failed start in a snapshot, never for replaced ones', () => {
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  assert.match(app, /if \(isLatest\(\)\) \{\s*const text = getErrorText\(error\);\s*const kind = classifyStartError\(text\);/);
  assert.match(app, /if \(kind === 'unreachable' \|\| kind === 'unplayable'\) showStartNotice\(kind\);\s*else if \(kind !== 'superseded'\) playbackError = text;/);
  assert.match(app, /readStartFailure\(lastStartFailureSequence, resolved\.startFailure\)/);
  assert.match(app, /snapshotReportsNewUnreachable\(previous\?\.lastError, resolved\.lastError\)\) showStartNotice\('unreachable'\)/);
});
