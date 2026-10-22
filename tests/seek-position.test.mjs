import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { execSync } from 'node:child_process';
import { normalizeSeekPositionMs } from '../src/lib/seek-position.js';

test('seek positions become whole, non-negative milliseconds inside the track', () => {
  assert.equal(normalizeSeekPositionMs(6325.30000000447), 6325, 'the reported clock estimate');
  assert.equal(normalizeSeekPositionMs(6325.5), 6326);
  assert.equal(normalizeSeekPositionMs(-4.2), 0);
  assert.equal(normalizeSeekPositionMs(Number.NaN), 0);
  assert.equal(normalizeSeekPositionMs(Number.POSITIVE_INFINITY, 1000), 0);
  assert.equal(normalizeSeekPositionMs(200_000.7, 180_000.9), 180_000);
  assert.equal(normalizeSeekPositionMs(5000, null), 5000);
  assert.equal(normalizeSeekPositionMs(5000, Number.NaN), 5000);
  assert.ok(Number.isInteger(normalizeSeekPositionMs(1234.567, 9999)));
});

test('every playback_seek goes through seekPlayback in ipc.ts', () => {
  const ipc = readFileSync(new URL('../src/lib/ipc.ts', import.meta.url), 'utf8');
  assert.match(ipc, /export function seekPlayback\([^)]*\)[^{]*\{\s*return invokeCommand\('playback_seek', \{ positionMs: normalizeSeekPositionMs\(positionMs, durationMs\) \}\);/);
  const callers = execSync("git grep -l \"'playback_seek'\" -- src", { cwd: new URL('..', import.meta.url), encoding: 'utf8' })
    .split('\n').filter(Boolean);
  assert.deepEqual(callers, ['src/lib/ipc.ts'], 'no other file invokes playback_seek directly');
});