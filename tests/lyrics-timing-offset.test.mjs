import './helpers/untr-fs.mjs';
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import {
  clampLyricsTimingOffsetSec,
  formatLyricsTimingOffsetSec,
  loadLyricsTimingOffsetSec,
  lyricsTimingDelaySecToOffsetMs,
  lyricsDisplayOffsetMs,
  LYRICS_TIMING_OFFSET_EVENT,
  saveLyricsTimingOffsetSec,
} from '../src/lib/lyrics-timing-offset.js';

test('clampLyricsTimingOffsetSec clamps, steps, and rejects non-finite', () => {
  assert.equal(clampLyricsTimingOffsetSec(0), 0);
  assert.equal(clampLyricsTimingOffsetSec(0.14), 0.1);
  assert.equal(clampLyricsTimingOffsetSec(0.15), 0.2);
  assert.equal(clampLyricsTimingOffsetSec(-10.04), -10);
  assert.equal(clampLyricsTimingOffsetSec(10.2), 10);
  assert.equal(clampLyricsTimingOffsetSec(-10.1), -10);
  assert.equal(clampLyricsTimingOffsetSec(10.1), 10);
  assert.equal(clampLyricsTimingOffsetSec(Number.NaN), 0);
  assert.equal(clampLyricsTimingOffsetSec('1.3'), 1.3);
});

test('lyricsTimingDelaySecToOffsetMs maps positive delay to negative offsetMs', () => {
  assert.equal(lyricsTimingDelaySecToOffsetMs(1.5), -1500);
  assert.equal(lyricsTimingDelaySecToOffsetMs(-0.2), 200);
  assert.equal(lyricsTimingDelaySecToOffsetMs(0), 0);
});

test('formatLyricsTimingOffsetSec keeps one decimal and sign', () => {
  assert.equal(formatLyricsTimingOffsetSec(0), '0.0 秒');
  assert.equal(formatLyricsTimingOffsetSec(1.2), '+1.2 秒');
  assert.equal(formatLyricsTimingOffsetSec(-0.5), '-0.5 秒');
});

test('timing panel overlays lyrics without extra header guidance', async () => {
  const source = await readFile(new URL('../src/lib/LyricsView.svelte', import.meta.url), 'utf8');
  assert.match(source, /class="lyrics-timing-panel floating-surface"/);
  assert.match(source, /\.lyrics-timing-panel \{[\s\S]*?position: absolute;[\s\S]*?top: 0;[\s\S]*?left: 0;/);
  assert.match(source, /<svelte:window onpointerdown=\{handleTimingPanelOutsidePointerDown\}/);
  assert.match(source, /timingPanelElement\?\.contains\(event\.target\)/);
  assert.match(source, /closest\(`\[aria-label="\$\{'同步歌詞延遲調整'\}"\]`\)/);
  assert.match(source, /<button[^>]*onclick=\{resetTimingOffset\}>重設<\/button>/);
  assert.doesNotMatch(source, /onclick=\{\(\) => \{ timingPanelOpen = false; \}\}>關閉/);
  assert.doesNotMatch(source, /正值延後歌詞，負值提前歌詞/);
  assert.doesNotMatch(source, /範圍 ±5 秒/);
});

test('save/load persists per track in localStorage', () => {
  const store = new Map();
  globalThis.localStorage = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => { store.set(key, String(value)); },
    removeItem: (key) => { store.delete(key); },
  };
  assert.equal(loadLyricsTimingOffsetSec('track-a'), 0);
  assert.equal(saveLyricsTimingOffsetSec('track-a', 1.25), 1.3);
  assert.equal(loadLyricsTimingOffsetSec('track-a'), 1.3);
  assert.equal(saveLyricsTimingOffsetSec('track-a', 0), 0);
  assert.equal(loadLyricsTimingOffsetSec('track-a'), 0);
  assert.equal(saveLyricsTimingOffsetSec('track-b', -2), -2);
  assert.equal(loadLyricsTimingOffsetSec('track-b'), -2);
  assert.equal(loadLyricsTimingOffsetSec('track-a'), 0);
});

test('every lyric surface uses lyricsDisplayOffsetMs (LRC offset + per-track delay)', async () => {
  assert.equal(lyricsDisplayOffsetMs(0, 0), 0);
  assert.equal(lyricsDisplayOffsetMs(0, 0.5), -500, 'delay +0.5 s shows lyrics later');
  assert.equal(lyricsDisplayOffsetMs(300, -1.2), 1500);
  assert.equal(lyricsDisplayOffsetMs(undefined, 1), -1000);
  assert.equal(lyricsDisplayOffsetMs(Number.NaN, 0), 0);
  assert.ok(!Object.is(lyricsDisplayOffsetMs(500, 0.5), -0));
  const page = await readFile(new URL('../src/lib/LyricsView.svelte', import.meta.url), 'utf8');
  // Windows taskbar / floating lyrics are native (ADR 0034): the same offset rule in Rust (shared vectors), the delay
  // sent by the main window with the track and on every change.
  const native = await readFile(new URL('../crates/player-lyrics-native/src/win.rs', import.meta.url), 'utf8');
  const app = await readFile(new URL('../src/App.svelte', import.meta.url), 'utf8');
  assert.match(page, /timedOffsetMs = \$derived\(lyricsDisplayOffsetMs\(viewState\.lyrics\?\.offsetMs, timingOffsetSec\)\)/);
  assert.match(native, /rules::display_offset_ms\(lyrics\.offset_ms, delay\)/);
  assert.doesNotMatch(page, /\+ lyricsTimingDelaySecToOffsetMs\(/, 'no second hand-made sum');
  assert.equal(LYRICS_TIMING_OFFSET_EVENT, 'lyrics-timing-offset-changed');
  assert.match(page, /emit\(LYRICS_TIMING_OFFSET_EVENT, \{ trackId, delaySec: timingOffsetSec \}\)/);
  assert.match(app, /emit\('lyrics-native-delay', \{ trackId, delaySec: loadLyricsTimingOffsetSec\(trackId\) \}\)/);
  // The cue lead is the page's animation lead only.
  assert.doesNotMatch(native, /LYRIC_CUE_LEAD/);
  assert.match(page, /displayCuePositions\(timedStartTimes, timedOffsetMs, cueLeading\)/);
});

test('歌詞同步 panel shows −stored; controls act on the shown value; timing for a stored value is unchanged', async () => {
  const { lyricsShiftShownSec, lyricsShiftStoredSec } = await import('../src/lib/lyrics-timing-offset.js');
  const { findActiveLyricIndex } = await import('../src/lib/lyrics-window.js');
  const { displayCuePositions } = await import('../src/lib/lyrics-cue-timing.ts');
  const { importTimestampShiftMs } = await import('../src/lib/lyrics-import.js');
  // Lyrics 2 s early: the user sets shown −2 → stored +2 → each line 2 s later (today's math, unchanged).
  assert.equal(lyricsShiftStoredSec(-2), 2);
  assert.equal(lyricsShiftShownSec(2), -2);
  assert.equal(lyricsShiftStoredSec(3), -3);
  assert.ok(Object.is(lyricsShiftShownSec(0), 0));
  assert.equal(formatLyricsTimingOffsetSec(lyricsShiftShownSec(0)), formatLyricsTimingOffsetSec(0));
  assert.ok(!formatLyricsTimingOffsetSec(lyricsShiftShownSec(-0)).startsWith('-'));
  // shown +0.1 ⇒ stored −0.1
  assert.equal(lyricsShiftStoredSec(lyricsShiftShownSec(1) + 0.1), 0.9);
  const starts = [10_000, 20_000];
  const offset = lyricsDisplayOffsetMs(null, lyricsShiftStoredSec(-2));
  assert.equal(offset, -2000);
  assert.equal(findActiveLyricIndex(starts.map((startMs) => ({ startMs })), 11_999, offset), -1);
  assert.equal(findActiveLyricIndex(starts.map((startMs) => ({ startMs })), 12_000, offset), 0);
  assert.deepEqual(displayCuePositions(starts, offset, false), [12_000, 22_000]);
  assert.equal(importTimestampShiftMs(null, 2), 2_000);
  const page = await readFile(new URL('../src/lib/LyricsView.svelte', import.meta.url), 'utf8');
  assert.match(page, /formatLyricsTimingOffsetSec\(timingShownSec\)/);
  assert.match(page, /value=\{timingShownSec\}/);
  assert.match(page, /lyricsShiftStoredSec\(timingShownSec \+ deltaSec\)/);
  assert.match(page, /lyricsShiftStoredSec\(Number\(target\.value\)\)/);
});