import assert from 'node:assert/strict';
import test from 'node:test';
import {
  FLOATING_LYRICS_NUDGE_REPEAT,
  floatingLyricsLockPx,
  floatingLyricsMarginPx,
  FLOATING_LYRICS_AREA_SHARE,
  floatingLyricsWidthMax,
  pressFloatingLyricsNudge,
  floatingLyricsWidthField,
  DEFAULT_FLOATING_LYRICS_SETTINGS,
  buildFloatingLyricsNativeData,
  normalizeFloatingLyricsSettings,
  normalizeLyricsSettingsTab,
} from '../src/lib/floating-lyrics-settings.js';

test('floating lyrics settings repair field by field with their own defaults', () => {
  assert.deepEqual(normalizeFloatingLyricsSettings(undefined), { ...DEFAULT_FLOATING_LYRICS_SETTINGS });
  const repaired = normalizeFloatingLyricsSettings({
    enabled: true,
    fontSizePx: 'big',
    translationScalePercent: 250,
    customColor: '#ABCDEF',
    align: 'middle',
    textEffect: 'stroke',
    locked: true,
    position: { x: -1200, y: 30.5 },
  });
  assert.equal(repaired.enabled, true);
  assert.equal(repaired.fontSizePx, 28);
  assert.equal(repaired.translationScalePercent, 100);
  assert.equal(repaired.opacityPercent, 100, 'missing opacity is fully opaque');
  assert.equal(normalizeFloatingLyricsSettings({ opacityPercent: 40 }).opacityPercent, 40);
  assert.equal(normalizeFloatingLyricsSettings({ opacityPercent: 1 }).opacityPercent, 1);
  assert.equal(normalizeFloatingLyricsSettings({ opacityPercent: 0 }).opacityPercent, 1);
  assert.equal(normalizeFloatingLyricsSettings({}).widthPx, 800);
  assert.equal(normalizeFloatingLyricsSettings({ widthPx: 50 }).widthPx, 100);
  assert.equal(normalizeFloatingLyricsSettings({ widthPx: 99999 }).widthPx, 8192);
  assert.equal(normalizeFloatingLyricsSettings({ widthPx: 1234.4 }).widthPx, 1234);
  assert.equal(normalizeFloatingLyricsSettings({ widthPx: 'wide' }).widthPx, 800);
  assert.deepEqual(
    (({ widthPx, landscapeWidthPx }) => ({ widthPx, landscapeWidthPx }))(normalizeFloatingLyricsSettings({ widthPx: 600, landscapeWidthPx: 9999 })),
    { widthPx: 600, landscapeWidthPx: 8192 },
  );
  assert.equal(floatingLyricsWidthField(true, true), 'landscapeWidthPx');
  assert.equal(floatingLyricsWidthField(true, false), 'widthPx');
  assert.equal(floatingLyricsWidthField(false, true), 'widthPx');
  assert.equal(repaired.customColor, '#abcdef');
  assert.equal(repaired.align, 'center');
  assert.equal(repaired.textEffect, 'stroke');
  assert.equal(repaired.locked, true);
  assert.equal(repaired.position, null);
  assert.equal(repaired.showTranslation, true);
  assert.deepEqual(normalizeFloatingLyricsSettings({ position: { x: -1200, y: 300 } }).position, { x: -1200, y: 300 });
  assert.equal(normalizeFloatingLyricsSettings({ position: { x: 5_000_000, y: 0 } }).position, null);
  assert.equal(normalizeFloatingLyricsSettings({ fontSizePx: 0.2 }).fontSizePx, 1);
  assert.equal('displayMode' in normalizeFloatingLyricsSettings({}), false);
});

test('native overlay data keeps timed lines in order with one combined offset', () => {
  const data = {
    trackId: 't1',
    offsetMs: 100,
    lines: [
      { startMs: 2000, text: ' b ', translation: null },
      { startMs: null, text: 'untimed' },
      { startMs: 1000, text: 'a', translation: ' A ' },
    ],
  };
  assert.deepEqual(buildFloatingLyricsNativeData(data, 350.4, 180000), {
    trackId: 't1',
    offsetMs: 350,
    durationMs: 180000,
    lines: [
      { startMs: 1000, text: 'a', translation: 'A' },
      { startMs: 2000, text: 'b', translation: '' },
    ],
  });
  assert.deepEqual(buildFloatingLyricsNativeData(null, 0), { trackId: null, offsetMs: 0, durationMs: null, lines: [] });
});

test('the lyrics settings tab falls back to general where it does not exist', () => {
  assert.equal(normalizeLyricsSettingsTab('taskbar', true), 'taskbar');
  assert.equal(normalizeLyricsSettingsTab('taskbar', false), 'general');
  assert.equal(normalizeLyricsSettingsTab('floating', false), 'floating');
  assert.equal(normalizeLyricsSettingsTab('nope', true), 'general');
});
test('floating lyrics lock follows the main line height and the width slider stops at the screen width', () => {
  assert.equal(floatingLyricsLockPx(16), 22);
  assert.equal(floatingLyricsLockPx(40), 55);
  assert.equal(floatingLyricsLockPx(0), 2);
  assert.equal(floatingLyricsWidthMax(1920), 1920);
  assert.equal(floatingLyricsWidthMax(1366.6), 1366);
  assert.equal(floatingLyricsWidthMax(50), 100);
  assert.equal(floatingLyricsWidthMax(null), 8192);
  // 32 px font: lock 44 + two 8 px margins come off the screen.
  assert.equal(floatingLyricsMarginPx(32), 8);
  assert.equal(floatingLyricsWidthMax(1920, 32), 1920 - 60);
  // Lyric area 1.1 of the 1.5 text heights.
  assert.ok(Math.abs(FLOATING_LYRICS_AREA_SHARE - 1.1 / 1.5) < 1e-9);
});

test('floating lyrics D-pad taps once, repeats when held and saves once on release', () => {
  const queue = [];
  let now = 0;
  const timers = {
    setTimeout: (fn, ms) => { const t = { fn, at: now + ms, every: 0 }; queue.push(t); return t; },
    clearTimeout: (t) => { t.cancelled = true; },
    setInterval: (fn, ms) => { const t = { fn, at: now + ms, every: ms }; queue.push(t); return t; },
    clearInterval: (t) => { t.cancelled = true; },
  };
  const advance = (ms) => {
    const end = now + ms;
    for (;;) {
      const due = queue.filter((t) => !t.cancelled && t.at <= end).sort((a, b) => a.at - b.at)[0];
      if (!due) break;
      now = due.at;
      if (due.every) due.at += due.every; else due.cancelled = true;
      due.fn();
    }
    now = end;
  };
  let steps = 0;
  let saves = 0;
  const tap = pressFloatingLyricsNudge(() => steps++, () => saves++, timers);
  advance(FLOATING_LYRICS_NUDGE_REPEAT.delayMs - 1);
  tap();
  tap();
  assert.deepEqual([steps, saves], [1, 1]);
  advance(1000);
  assert.equal(steps, 1);

  steps = 0;
  const hold = pressFloatingLyricsNudge(() => steps++, () => saves++, timers);
  advance(300 + 350);
  assert.ok(Math.abs(FLOATING_LYRICS_NUDGE_REPEAT.intervalMs - 100 / 3) < 1e-9, 'one pixel per 33.3 ms');
  assert.equal(steps, 1 + 10);
  hold();
  advance(1000);
  assert.deepEqual([steps, saves], [11, 2]);
});

test('floating animation is saved apart and repaired to none', async () => {
  const { normalizeFloatingLyricsSettings: normalize, DEFAULT_FLOATING_LYRICS_SETTINGS: defaults } = await import('../src/lib/floating-lyrics-settings.js');
  assert.equal(defaults.animation, 'none');
  assert.equal(normalize({ animation: 'continuous' }).animation, 'continuous');
  assert.equal(normalize({ animation: 1 }).animation, 'none');
});
