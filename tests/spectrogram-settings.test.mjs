import test from 'node:test';
import assert from 'node:assert/strict';
import {
  DEFAULT_SPECTROGRAM_SETTINGS,
  fractionToFrequency,
  frequencyTicks,
  frequencyToFraction,
  normalizeSpectrogramSettings,
  toRenderOptions,
} from '../src/lib/spectrogram-settings.js';

test('normalize falls back to defaults and repairs ranges', () => {
  assert.deepEqual(normalizeSpectrogramSettings(null), DEFAULT_SPECTROGRAM_SETTINGS);
  const s = normalizeSpectrogramSettings({ fftSize: 3000, window: 'x', dbFloor: -60, dbCeiling: -80, channel: { mode: 'channel', index: 1 }, colormap: 'viridis' });
  assert.equal(s.fftSize, 4096, 'an unknown size falls back to the default, 4096');
  assert.equal(s.window, 'hann');
  assert.equal(s.dbCeiling, 0, 'legacy upper limits are ignored');
  assert.deepEqual(s.channel, { mode: 'channel', index: 1 });
  assert.equal(s.colormap, 'viridis');
});

test('log axis round-trips and ticks stay below max', () => {
  for (const hz of [50, 1000, 15000]) {
    assert.ok(Math.abs(fractionToFrequency(frequencyToFraction(hz, 22050, true), 22050, true) - hz) < 1e-6);
  }
  assert.equal(frequencyToFraction(11025, 22050, false), 0.5);
  assert.ok(frequencyTicks(22050, true).every((f) => f < 22050));
  assert.deepEqual(frequencyTicks(22050, false).slice(0, 3), [0, 2000, 4000]);
});

test('a missing or unknown channel means the left channel; mix stays a choice', () => {
  assert.deepEqual(normalizeSpectrogramSettings({}).channel, { mode: 'channel', index: 0 });
  assert.deepEqual(normalizeSpectrogramSettings({ channel: { mode: 'mix' } }).channel, { mode: 'mix' });
  assert.equal('logFrequency' in normalizeSpectrogramSettings({ logFrequency: true }), false);
});

test('render options clamp size', () => {
  const o = toRenderOptions(DEFAULT_SPECTROGRAM_SETTINGS, 99999, 10);
  assert.equal(o.columns, 8000);
  assert.equal(o.height, 64);
  assert.equal(toRenderOptions({ ...DEFAULT_SPECTROGRAM_SETTINGS, dbCeiling: 20 }, 100, 100).dbCeiling, 0);
  assert.deepEqual(o.channel, { mode: 'channel', index: 0 }, 'the left channel by default');
  assert.equal(o.logFrequency, false, 'the frequency axis is always linear');
});
