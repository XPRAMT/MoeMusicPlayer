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
  migrateLegacyLyricsTimingOffsets,
  saveLyricsTimingOffsetSec,
  setLyricsTimingOffsetBackendForTests,
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

/** A stand-in for the library database behind the Tauri commands (tenths, like the Rust table). */
function fakeDatabase() {
  const rows = new Map();
  const calls = [];
  return {
    rows,
    calls,
    available: () => true,
    async invoke(command, args) {
      calls.push(command);
      if (command === 'lyrics_timing_offset_get') return (rows.get(args.trackId) ?? 0) / 10;
      if (command === 'lyrics_timing_offset_set') {
        const tenths = Math.max(-100, Math.min(100, Math.round(args.delaySec * 10)));
        if (tenths === 0) rows.delete(args.trackId); else rows.set(args.trackId, tenths);
        return tenths / 10;
      }
      if (command === 'lyrics_timing_offsets_import') {
        let added = 0;
        for (const entry of args.entries) {
          if (!rows.has(entry.trackId)) { rows.set(entry.trackId, Math.round(entry.delaySec * 10)); added += 1; }
        }
        return added;
      }
      throw new Error(command);
    },
  };
}

function fakeLocalStorage(initial = {}) {
  const store = new Map(Object.entries(initial));
  globalThis.localStorage = {
    get length() { return store.size; },
    key: (index) => [...store.keys()][index] ?? null,
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => { store.set(key, String(value)); },
    removeItem: (key) => { store.delete(key); },
  };
  return store;
}

test('save/load go to the library database per track, not browser storage', async () => {
  const store = fakeLocalStorage();
  const db = fakeDatabase();
  setLyricsTimingOffsetBackendForTests(db);
  assert.equal(await loadLyricsTimingOffsetSec('track-a'), 0);
  assert.equal(await saveLyricsTimingOffsetSec('track-a', 1.25), 1.3);
  assert.equal(await loadLyricsTimingOffsetSec('track-a'), 1.3);
  assert.equal(await saveLyricsTimingOffsetSec('track-a', 0), 0);
  assert.equal(await loadLyricsTimingOffsetSec('track-a'), 0);
  assert.equal(await saveLyricsTimingOffsetSec('track-b', -2), -2);
  assert.equal(await loadLyricsTimingOffsetSec('track-b'), -2);
  assert.equal(store.size, 0, 'nothing is written to localStorage');
  assert.equal(await loadLyricsTimingOffsetSec(null), 0);
});

test('saves are written in order: the last value wins', async () => {
  fakeLocalStorage();
  const db = fakeDatabase();
  setLyricsTimingOffsetBackendForTests(db);
  await Promise.all([0.1, 0.2, 0.3, 0.4].map((value) => saveLyricsTimingOffsetSec('track-a', value)));
  assert.equal(await loadLyricsTimingOffsetSec('track-a'), 0.4);
});

test('old localStorage delays move into the database once and their keys are removed', async () => {
  const prefix = 'moemusicplayer.lyricsTimingOffsetSec:';
  const store = fakeLocalStorage({ [prefix + 'track-a']: '1.5', [prefix + 'track-b']: '0', other: 'kept' });
  const db = fakeDatabase();
  db.rows.set('track-c', 7);
  setLyricsTimingOffsetBackendForTests(db);
  assert.equal(await loadLyricsTimingOffsetSec('track-a'), 1.5, 'the first read already sees the moved value');
  assert.deepEqual([...store.keys()], ['other']);
  assert.equal(await loadLyricsTimingOffsetSec('track-c'), 0.7);
  await migrateLegacyLyricsTimingOffsets();
  assert.equal(db.calls.filter((call) => call === 'lyrics_timing_offsets_import').length, 1);
});

test('a failed import keeps the old keys for the next try', async () => {
  const prefix = 'moemusicplayer.lyricsTimingOffsetSec:';
  const store = fakeLocalStorage({ [prefix + 'track-a']: '2' });
  const db = fakeDatabase();
  const ok = db.invoke.bind(db);
  let fail = true;
  db.invoke = async (command, args) => {
    if (command === 'lyrics_timing_offsets_import' && fail) throw new Error('database closed');
    return ok(command, args);
  };
  setLyricsTimingOffsetBackendForTests(db);
  await migrateLegacyLyricsTimingOffsets();
  assert.equal(store.size, 1);
  fail = false;
  await migrateLegacyLyricsTimingOffsets();
  assert.equal(store.size, 0);
  assert.equal(await loadLyricsTimingOffsetSec('track-a'), 2);
});

test('outside Tauri nothing is stored', async () => {
  fakeLocalStorage();
  setLyricsTimingOffsetBackendForTests({ available: () => false, invoke: async () => { throw new Error('no'); } });
  assert.equal(await saveLyricsTimingOffsetSec('track-a', 1.25), 1.3);
  assert.equal(await loadLyricsTimingOffsetSec('track-a'), 0);
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
  assert.match(page, /emit\(LYRICS_TIMING_OFFSET_EVENT, \{ trackId: id, delaySec: stored \}\)/);
  assert.match(app, /loadLyricsTimingOffsetSec\(trackId\)\.then\(\(delaySec\) => emit\('lyrics-native-delay', \{ trackId, delaySec \}\)\)/);
  assert.doesNotMatch(page + app, /localStorage[^\n]*lyricsTimingOffset/);
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