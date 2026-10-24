import assert from 'node:assert/strict';
import test from 'node:test';
import {
  isRemoteUnreachableMessage,
  shouldShowUnreachableNotice,
  snapshotReportsNewUnreachable,
} from '../src/lib/remote-unreachable-notice.js';

test('only unreachable-server messages are recognised', () => {
  assert.equal(isRemoteUnreachableMessage('伺服器無法連線'), true);
  assert.equal(isRemoteUnreachableMessage('無法開啟遠端檔案：無法連線到伺服器：Transport error'), true);
  assert.equal(isRemoteUnreachableMessage('superseded'), false);
  assert.equal(isRemoteUnreachableMessage('找不到檔案'), false);
  assert.equal(isRemoteUnreachableMessage(null), false);
});

test('a snapshot reports a failure only when the error text becomes an unreachable one', () => {
  assert.equal(snapshotReportsNewUnreachable(null, '伺服器無法連線'), true);
  assert.equal(snapshotReportsNewUnreachable('伺服器無法連線', '伺服器無法連線'), false);
  assert.equal(snapshotReportsNewUnreachable('伺服器無法連線', null), false);
  assert.equal(snapshotReportsNewUnreachable(null, '解碼失敗'), false);
});

test('the same attempt reported twice within two seconds shows one notice', () => {
  assert.equal(shouldShowUnreachableNotice(null, 100), true);
  assert.equal(shouldShowUnreachableNotice(100, 1500), false);
  assert.equal(shouldShowUnreachableNotice(100, 2100), true);
});

test('App shows the notice for the newest command error and for a new snapshot error, never for superseded ones', async () => {
  const { readFileSync } = await import('node:fs');
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  assert.match(app, /if \(isLatest\(\)\) \{\s*const text = getErrorText\(error\);[\s\S]*?showUnreachableNotice\(\)/);
  assert.match(app, /snapshotReportsNewUnreachable\(previous\?\.lastError, resolved\.lastError\)\) showUnreachableNotice\(\)/);
});
