import test from 'node:test';
import assert from 'node:assert/strict';
import { floatingLyricsLockPath, FLOATING_LYRICS_LOCK_CLOSED_PATH, FLOATING_LYRICS_LOCK_OPEN_PATH } from '../src/lib/floating-lyrics-lock-icon.js';

test('floating lyrics lock glyph indicates the action that will be performed', () => {
  assert.equal(floatingLyricsLockPath(true), FLOATING_LYRICS_LOCK_OPEN_PATH);
  assert.equal(floatingLyricsLockPath(false), FLOATING_LYRICS_LOCK_CLOSED_PATH);
  assert.notEqual(FLOATING_LYRICS_LOCK_CLOSED_PATH, FLOATING_LYRICS_LOCK_OPEN_PATH);
});
