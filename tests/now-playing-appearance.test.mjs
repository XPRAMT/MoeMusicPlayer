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
    backgroundOpacityPercent: 40,
    coverCornerStyle: 'rounded',
    timelineStyle: 'edge',
  });
  assert.deepEqual(normalizeNowPlayingAppearancePreferences({
    backgroundBlurPx: -4,
    backgroundOpacityPercent: 140,
    coverCornerStyle: 'square',
    timelineStyle: 'edge',
  }), {
    backgroundBlurPx: 0,
    backgroundOpacityPercent: 100,
    coverCornerStyle: 'square',
    timelineStyle: 'edge',
  });
  assert.deepEqual(normalizeNowPlayingAppearancePreferences({
    backgroundBlurPx: 80.4,
    backgroundOpacityPercent: 21.6,
    coverCornerStyle: 'nope',
    timelineStyle: 'nope',
  }), {
    backgroundBlurPx: 40,
    backgroundOpacityPercent: 22,
    coverCornerStyle: 'rounded',
    timelineStyle: 'edge',
  });
  // Legacy line/bar/minimal values all resolve to the only remaining edge timeline.
  for (const legacy of ['line', 'bar', 'minimal']) {
    assert.deepEqual(normalizeNowPlayingAppearancePreferences({
      backgroundBlurPx: 10,
      backgroundOpacityPercent: 40,
      coverCornerStyle: 'rounded',
      timelineStyle: legacy,
    }), {
      backgroundBlurPx: 10,
      backgroundOpacityPercent: 40,
      coverCornerStyle: 'rounded',
      timelineStyle: 'edge',
    });
  }
  assert.deepEqual(normalizeNowPlayingAppearancePreferences({
    backgroundBlurPx: Number.NaN,
    backgroundOpacityPercent: Infinity,
  }), DEFAULT_NOW_PLAYING_APPEARANCE_PREFERENCES);
  for (const legacySurface of [0, 100]) {
    assert.deepEqual(normalizeNowPlayingAppearancePreferences({
      backgroundBlurPx: 27,
      surfaceTransparencyPercent: legacySurface,
    }), { backgroundBlurPx: 27, backgroundOpacityPercent: 40, coverCornerStyle: 'rounded', timelineStyle: 'edge' });
  }
  assert.equal(
    normalizeNowPlayingAppearancePreferences({ backgroundBrightnessPercent: 55 }).backgroundOpacityPercent,
    55,
  );
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

  writer.schedule({ backgroundBlurPx: 20, backgroundOpacityPercent: 40, coverCornerStyle: 'rounded', timelineStyle: 'line' });
  await wait(35);
  assert.equal(requests.length, 1, 'debounced change should start one write');
  writer.schedule({ backgroundBlurPx: 21, backgroundOpacityPercent: 40, coverCornerStyle: 'rounded', timelineStyle: 'line' });
  await wait(8);
  writer.schedule({ backgroundBlurPx: 22, backgroundOpacityPercent: 48, coverCornerStyle: 'square', timelineStyle: 'edge' });
  await wait(35);
  assert.equal(requests.length, 1, 'new values must wait behind the slow acknowledgement');
  resolveNext();
  await wait(0);
  assert.equal(requests.length, 2, 'one latest value should follow the active write');
  assert.deepEqual(requests[1], { backgroundBlurPx: 22, backgroundOpacityPercent: 48, coverCornerStyle: 'square', timelineStyle: 'edge' });
  resolveNext();
  await wait(0);
  assert.deepEqual(saved, [{ backgroundBlurPx: 22, backgroundOpacityPercent: 48, coverCornerStyle: 'square', timelineStyle: 'edge' }]);
  assert.equal(maxActive, 1, 'settings writes must not overlap');
  assert.deepEqual(errors, []);

  writer.schedule({ backgroundBlurPx: 24, backgroundOpacityPercent: 50, coverCornerStyle: 'rounded', timelineStyle: 'line' }, true);
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

