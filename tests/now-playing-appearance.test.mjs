import assert from 'node:assert/strict';
import test from 'node:test';
import {
  createNowPlayingAppearanceWriter,
  DEFAULT_NOW_PLAYING_APPEARANCE_PREFERENCES,
  normalizeNowPlayingAppearancePreferences,
} from '../src/lib/now-playing-appearance.js';

test('appearance preferences default and clamp to supported ranges', () => {
  assert.deepEqual(normalizeNowPlayingAppearancePreferences(), {
    backgroundBlurPx: 20,
    surfaceTransparencyPercent: 35,
  });
  assert.deepEqual(normalizeNowPlayingAppearancePreferences({
    backgroundBlurPx: -4,
    surfaceTransparencyPercent: 140,
  }), {
    backgroundBlurPx: 0,
    surfaceTransparencyPercent: 100,
  });
  assert.deepEqual(normalizeNowPlayingAppearancePreferences({
    backgroundBlurPx: 80.4,
    surfaceTransparencyPercent: 21.6,
  }), {
    backgroundBlurPx: 40,
    surfaceTransparencyPercent: 22,
  });
  assert.deepEqual(normalizeNowPlayingAppearancePreferences({
    backgroundBlurPx: Number.NaN,
    surfaceTransparencyPercent: Infinity,
  }), DEFAULT_NOW_PLAYING_APPEARANCE_PREFERENCES);
});

test('slow saves coalesce drag updates and ignore stale acknowledgements', async () => {
  const requests = [];
  const saved = [];
  const errors = [];
  const held = [];
  let active = 0;
  let maxActive = 0;
  const writer = createNowPlayingAppearanceWriter({
    delayMs: 25,
    write(preferences) {
      requests.push(preferences);
      active += 1;
      maxActive = Math.max(maxActive, active);
      return new Promise((resolve, reject) => held.push({ preferences, resolve, reject }));
    },
    onSaved(value) { saved.push(value); },
    onError(error) { errors.push(error); },
  });
  const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const resolveNext = () => {
    const request = held.shift();
    assert.ok(request, 'expected a pending write');
    active -= 1;
    request.resolve(request.preferences);
  };

  writer.schedule({ backgroundBlurPx: 20, surfaceTransparencyPercent: 35 });
  await wait(35);
  assert.equal(requests.length, 1, 'debounced change should start one write');
  writer.schedule({ backgroundBlurPx: 21, surfaceTransparencyPercent: 35 });
  await wait(8);
  writer.schedule({ backgroundBlurPx: 22, surfaceTransparencyPercent: 48 });
  await wait(35);
  assert.equal(requests.length, 1, 'new values must wait behind the slow acknowledgement');
  resolveNext();
  await wait(0);
  assert.equal(requests.length, 2, 'one latest value should follow the active write');
  assert.deepEqual(requests[1], { backgroundBlurPx: 22, surfaceTransparencyPercent: 48 });
  resolveNext();
  await wait(0);
  assert.deepEqual(saved, [{ backgroundBlurPx: 22, surfaceTransparencyPercent: 48 }]);
  assert.equal(maxActive, 1, 'settings writes must not overlap');
  assert.deepEqual(errors, []);

  writer.schedule({ backgroundBlurPx: 24, surfaceTransparencyPercent: 50 }, true);
  await wait(0);
  const failed = held.shift();
  assert.ok(failed);
  active -= 1;
  failed.reject(new Error('settings unavailable'));
  await wait(0);
  assert.equal(errors.length, 1, 'current write failures should be surfaced');
  assert.match(errors[0].message, /settings unavailable/);
  writer.invalidate();
});
