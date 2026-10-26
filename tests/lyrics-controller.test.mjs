import assert from 'node:assert/strict';
import test from 'node:test';
import {
  buildTimedLyricTimeline,
  buildLyricsLayout,
  findLyricsRowAtOffset,
  findActiveLyricIndex,
  getPlainLyricWindow,
  getTimedLyricWindow,
  getLyricsOffset,
  sameLyricWindow,
} from '../src/lib/lyrics-window.js';
import {
  DEFAULT_LYRICS_PREFERENCES,
  getDisplayActiveLyricIndex,
  getLyricLineOpacity,
  normalizeLyricsPreferences,
  lyricsAuxiliaryFontSizePx,
} from '../src/lib/lyrics-preferences.js';
import { createLyricsController } from '../src/lib/lyrics-controller.js';
import { getCandidatePresentation } from '../src/lib/lyrics-candidate-preview.js';

function lyric(trackId, source = 'local', text = '歌詞') {
  return {
    trackId,
    source,
    title: `曲目 ${trackId}`,
    artist: null,
    album: null,
    durationMs: 10_000,
    offsetMs: 0,
    synced: true,
    lines: [{ startMs: 0, text, translation: null, romanization: null }],
  };
}

function readyResult(trackId, source = 'local') {
  return { lyrics: lyric(trackId, source), candidates: [], status: 'ready', error: null };
}

function emptyResult() {
  return { lyrics: null, candidates: [], status: 'empty', error: null };
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

test('timed lyric timeline follows forward seek, backward seek, and per-track offset', () => {
  const lines = [
    { startMs: 0, text: '一', translation: null, romanization: null },
    { startMs: 1_200, text: '二', translation: null, romanization: null },
    { startMs: 2_600, text: '三', translation: null, romanization: null },
  ];
  const timeline = buildTimedLyricTimeline(lines);

  assert.equal(findActiveLyricIndex(timeline, 0), 0);
  assert.equal(findActiveLyricIndex(timeline, 2_900), 2);
  assert.equal(findActiveLyricIndex(timeline, 900, 300), 1);
  assert.equal(findActiveLyricIndex(timeline, 400), 0);
  assert.equal(findActiveLyricIndex(timeline, -1), -1);
});

test('timed lyric window follows the scroll viewport with overscan, not the active line', () => {
  const preferences = DEFAULT_LYRICS_PREFERENCES;
  const timeline = buildTimedLyricTimeline(Array.from({ length: 1_000 }, (_, index) => ({
    startMs: index * 1_000,
    text: index % 7 === 0 ? '  ' : `歌詞 ${index}`,
    translation: null,
    romanization: null,
  })));
  const layout = buildLyricsLayout(timeline, preferences, preferences.lineGapPx);
  const pad = 200;
  const viewport = 400;
  const rowAt = (offset) => findLyricsRowAtOffset(layout, offset);
  const top = getLyricsOffset(layout, 500);
  const middle = getTimedLyricWindow(layout, top + pad, viewport, pad);
  const visible = Array.from(layout.visibleIndices);
  const firstPos = visible.indexOf(rowAt(top));
  const lastPos = visible.indexOf(rowAt(top + viewport - 0.001));
  assert.deepEqual(middle.rows, visible.slice(firstPos - 6, lastPos + 7));
  assert.ok(middle.rows.every((index) => index % 7 !== 0), 'hidden rows of height 0 are never rendered');
  assert.equal(middle.beforeHeight, getLyricsOffset(layout, middle.start));
  assert.equal(middle.afterHeight, layout.totalHeight - getLyricsOffset(layout, middle.end));

  // In the head padding the first rows render; past the end the last rows render.
  const beginning = getTimedLyricWindow(layout, 0, viewport, pad);
  assert.equal(beginning.rows[0], visible[0]);
  assert.equal(beginning.beforeHeight, 0);
  const ending = getTimedLyricWindow(layout, layout.totalHeight + 2 * pad, viewport, pad);
  assert.equal(ending.rows.at(-1), visible.at(-1));
  assert.equal(ending.afterHeight, 0);

  // A glide target widens the window to the union, so rows it brings in are already rendered.
  const target = getLyricsOffset(layout, 520) + pad;
  const gliding = getTimedLyricWindow(layout, top + pad, viewport, pad, [target]);
  const atTarget = getTimedLyricWindow(layout, target, viewport, pad);
  assert.equal(gliding.start, middle.start);
  assert.equal(gliding.end, atTarget.end);
  assert.ok(sameLyricWindow(middle, getTimedLyricWindow(layout, top + pad + 1, viewport, pad)));
  assert.ok(!sameLyricWindow(middle, gliding));
});

test('plain lyrics use a bounded scroll window without claiming synchronized timing', () => {
  const preferences = DEFAULT_LYRICS_PREFERENCES;
  const lines = Array.from({ length: 10_000 }, (_, index) => ({
    startMs: null,
    text: `純歌詞 ${index}`,
    translation: null,
    romanization: null,
  }));
  const layout = buildLyricsLayout(lines, preferences, preferences.lineGapPx);
  const rowHeight = layout.heights[0] + layout.gaps[0];
  const first = getPlainLyricWindow(layout, layout.offsets[100], rowHeight * 5);
  const top = getPlainLyricWindow(layout, 0, rowHeight * 5);
  const bottom = getPlainLyricWindow(layout, Number.MAX_SAFE_INTEGER, rowHeight * 5);

  assert.equal(first.start, 94);
  assert.equal(first.end, 111);
  assert.ok(first.end - first.start <= 5 + 12);
  assert.equal(first.beforeHeight, layout.offsets[first.start]);
  assert.equal(top.start, 0);
  assert.equal(bottom.end, 10_000);
});

test('lyrics preferences use defaults and enforce the IPC-supported bounds', () => {
  assert.deepEqual(normalizeLyricsPreferences(null), DEFAULT_LYRICS_PREFERENCES);
  assert.deepEqual(normalizeLyricsPreferences({
    showTranslation: true,
    showRomanization: false,
    mainOpacityPercent: 0,
    secondaryOpacityPercent: 1,
    primaryFontSizePx: 99,
    auxiliaryFontSizePx: 4,
    lineGapPx: 99,
  }), {
    showTranslation: true,
    showRomanization: false,
    mainOpacityPercent: 1,
    secondaryOpacityPercent: 1,
    primaryFontSizePx: 50,
    auxiliaryScalePercent: 4,
    fontWeight: 500,
    fontFamily: null,
    lineGapPx: 50,
    textEffect: 'shadow',
    simplifiedToTraditional: true,
  });
  assert.equal(normalizeLyricsPreferences({ simplifiedToTraditional: true }).simplifiedToTraditional, true);
  assert.equal(normalizeLyricsPreferences({ secondaryOpacityPercent: 100.4 }).secondaryOpacityPercent, 100);
  assert.equal(normalizeLyricsPreferences({ mainOpacityPercent: 250 }).mainOpacityPercent, 100);
  // The former shared key is ignored, not converted; the new fields default (main 100, secondary 50).
  const former = normalizeLyricsPreferences({ inactiveOpacityPercent: 70 });
  assert.deepEqual([former.mainOpacityPercent, former.secondaryOpacityPercent], [100, 50]);
  assert.equal('inactiveOpacityPercent' in former, false);
  // No clamping between the two: the secondary may exceed the main.
  const crossed = normalizeLyricsPreferences({ mainOpacityPercent: 30, secondaryOpacityPercent: 90 });
  assert.deepEqual([crossed.mainOpacityPercent, crossed.secondaryOpacityPercent], [30, 90]);
  assert.equal(normalizeLyricsPreferences({ lineGapPx: -1 }).lineGapPx, 0);
  assert.equal(normalizeLyricsPreferences({ primaryFontSizePx: -1 }).primaryFontSizePx, 1);
  assert.equal(normalizeLyricsPreferences({ auxiliaryScalePercent: -1 }).auxiliaryScalePercent, 1);
  assert.equal(normalizeLyricsPreferences({ auxiliaryScalePercent: 250 }).auxiliaryScalePercent, 100);
  assert.equal(DEFAULT_LYRICS_PREFERENCES.auxiliaryScalePercent, 90, 'default matches the old 18 / 20 px');
  // Old saves stored an absolute size: round(aux / primary × 100), clamped to 1–100.
  assert.equal(normalizeLyricsPreferences({ primaryFontSizePx: 20, auxiliaryFontSizePx: 18 }).auxiliaryScalePercent, 90);
  assert.equal(normalizeLyricsPreferences({ primaryFontSizePx: 14, auxiliaryFontSizePx: 10 }).auxiliaryScalePercent, 71);
  assert.equal(normalizeLyricsPreferences({ auxiliaryFontSizePx: 18 }).auxiliaryScalePercent, 90, 'missing primary uses the default 20 px');
  assert.equal(normalizeLyricsPreferences({ primaryFontSizePx: 20, auxiliaryFontSizePx: 40 }).auxiliaryScalePercent, 100);
  assert.equal(normalizeLyricsPreferences({ primaryFontSizePx: 20, auxiliaryFontSizePx: 18, auxiliaryScalePercent: 55 }).auxiliaryScalePercent, 55);
  assert.ok(!('auxiliaryFontSizePx' in normalizeLyricsPreferences({ auxiliaryFontSizePx: 18 })));
  assert.equal(lyricsAuxiliaryFontSizePx({ primaryFontSizePx: 20, auxiliaryScalePercent: 90 }), 18);
  assert.equal(normalizeLyricsPreferences({ primaryFontSizePx: 0 }).primaryFontSizePx, 1, 'the font sizes start at 1');
});

test('variable row geometry adds only nonempty enabled auxiliary lines', () => {
  const base = {
    ...DEFAULT_LYRICS_PREFERENCES,
    showTranslation: true,
    showRomanization: true,
  };
  const rows = [
    { text: '原文', translation: null, romanization: null },
    { text: '原文', translation: ' ', romanization: '' },
    { text: '原文', translation: '翻譯', romanization: null },
    { text: '原文', translation: null, romanization: '拼音' },
    { text: '原文', translation: '翻譯', romanization: '拼音' },
    { text: '  ', translation: '', romanization: null },
  ];
  const layout = buildLyricsLayout(rows, base, 24);
  const primary = base.primaryFontSizePx * 1.35 + 6;
  const oneAux = primary + 2 + (base.primaryFontSizePx * base.auxiliaryScalePercent / 100) * 1.35;
  const twoAux = primary + 4 + 2 * (base.primaryFontSizePx * base.auxiliaryScalePercent / 100) * 1.35;

  assert.equal(layout.heights[0], primary);
  assert.equal(layout.heights[1], primary);
  assert.equal(layout.heights[2], oneAux);
  assert.equal(layout.heights[3], oneAux);
  assert.equal(layout.heights[4], twoAux);
  assert.equal(layout.heights[5], 0);
  assert.deepEqual([...layout.visibleIndices], [0, 1, 2, 3, 4]);
  assert.equal(layout.gaps[4], 0);
  assert.ok(Math.abs(layout.totalHeight - ([...layout.heights].reduce((sum, height) => sum + height, 0) + 4 * 24)) < 1e-9);

  const translationHidden = buildLyricsLayout(rows, { ...base, showTranslation: false }, 24);
  const romanizationHidden = buildLyricsLayout(rows, { ...base, showRomanization: false }, 24);
  assert.equal(translationHidden.heights[2], primary);
  assert.equal(romanizationHidden.heights[3], primary);
  assert.equal(buildLyricsLayout(rows, { ...base, showTranslation: false, showRomanization: false }, 24).heights[5], 0);
});

test('measured wrapped primary updates the shared Fenwick prefix without rebuilding the document', async () => {
  const { updateLyricsRowHeight, getLyricsOffset } = await import('../src/lib/lyrics-window.js');
  const layout = buildLyricsLayout([
    { text: 'A short primary line', translation: null, romanization: null },
    { text: 'A primary line which wraps at the current viewport width', translation: 'translated', romanization: null },
    { text: 'following line', translation: null, romanization: null },
  ], DEFAULT_LYRICS_PREFERENCES, 24);
  const initialNext = getLyricsOffset(layout, 2);
  assert.equal(updateLyricsRowHeight(layout, 1, layout.heights[1] + 18), true);
  // Fenwick sums are floating point; compare within rounding.
  assert.ok(Math.abs(getLyricsOffset(layout, 2) - (initialNext + 18)) < 1e-9);
  assert.ok(Math.abs(layout.totalHeight - getLyricsOffset(layout, 3)) < 1e-9);
});

test('layout binary search skips empty rows, honors gap extremes and keeps all-empty documents bounded', () => {
  const lines = [
    { text: '甲', translation: null, romanization: null },
    { text: '', translation: null, romanization: null },
    { text: '乙', translation: null, romanization: null },
  ];
  const noGap = buildLyricsLayout(lines, DEFAULT_LYRICS_PREFERENCES, 0);
  const maxGap = buildLyricsLayout(lines, DEFAULT_LYRICS_PREFERENCES, 50);
  assert.ok(Math.abs(maxGap.totalHeight - noGap.totalHeight - 50) < 0.001);
  assert.equal(findLyricsRowAtOffset(maxGap, maxGap.offsets[1]), 2);
  assert.equal(maxGap.gaps[0], 50);
  assert.equal(maxGap.gaps[2], 0);

  const empty = buildLyricsLayout(Array.from({ length: 100_000 }, () => ({ text: ' ', translation: null, romanization: null })), DEFAULT_LYRICS_PREFERENCES, 50);
  assert.equal(empty.totalHeight, 0);
  assert.equal(findLyricsRowAtOffset(empty, 0), -1);
  assert.deepEqual(getPlainLyricWindow(empty, 0, 480).rows, []);

  const sparseLines = Array.from({ length: 100_000 }, (_, index) => ({
    text: [20_000, 50_000, 90_000].includes(index) ? `可見 ${index}` : '',
    translation: null,
    romanization: null,
  }));
  const sparseLayout = buildLyricsLayout(sparseLines, DEFAULT_LYRICS_PREFERENCES, 24);
  const sparseWindow = getPlainLyricWindow(sparseLayout, sparseLayout.offsets[50_000], 480);
  assert.deepEqual(sparseWindow.rows, [20_000, 50_000, 90_000]);
  assert.ok(sparseWindow.rows.length <= 3, 'long empty stretches do not inflate the virtual window');
});

test('plain virtual window uses variable prefix offsets and bounds rows at distant scroll positions', () => {
  const preferences = { ...DEFAULT_LYRICS_PREFERENCES, showTranslation: true, showRomanization: true };
  const lines = Array.from({ length: 100_000 }, (_, index) => ({
    text: index % 11 === 0 ? '' : `純歌詞 ${index}`,
    translation: index % 3 === 0 ? `譯文 ${index}` : null,
    romanization: index % 5 === 0 ? `拼音 ${index}` : null,
  }));
  const layout = buildLyricsLayout(lines, preferences, 24);
  const targetOffset = layout.offsets[80_000];
  const window = getPlainLyricWindow(layout, targetOffset, 400);

  assert.ok(window.start < 80_000 && window.end > 80_000);
  assert.ok(window.rows.length <= 13 + Math.ceil(400 / 52.8) + 2);
  assert.equal(window.beforeHeight, layout.offsets[window.start]);
  assert.equal(window.afterHeight, layout.totalHeight - layout.offsets[window.end]);
  assert.equal(findLyricsRowAtOffset(layout, targetOffset), 80_000);
});

test('the current line takes the main opacity, the others the secondary; no cue shows every line at the main', () => {
  assert.equal(getDisplayActiveLyricIndex(2, 'playing'), 2);
  assert.equal(getDisplayActiveLyricIndex(2, 'paused'), 2);
  assert.equal(getDisplayActiveLyricIndex(2, 'buffering'), 2, 'a seek while playing keeps the highlight');
  assert.equal(getDisplayActiveLyricIndex(2, 'stopped'), -1);
  assert.equal(getDisplayActiveLyricIndex(-1, 'playing'), -1);
  assert.equal(getLyricLineOpacity(1, 2, 100, 35), 0.35);
  assert.equal(getLyricLineOpacity(2, 2, 100, 35), 1);
  assert.equal(getLyricLineOpacity(1, -1, 100, 35), 1);
  // Absolute values: main 60 shows the current line at 0.6, and the secondary may be brighter than the main.
  assert.equal(getLyricLineOpacity(2, 2, 60, 35), 0.6);
  assert.equal(getLyricLineOpacity(1, 2, 60, 35), 0.35);
  assert.equal(getLyricLineOpacity(1, 2, 30, 80), 0.8);
  assert.equal(getLyricLineOpacity(2, 2, 30, 80), 0.3);
  assert.equal(getLyricLineOpacity(1, -1, 60, 35), 0.6);
});

test('local lyrics avoid remote search and repeated same-track updates do not reload them', async () => {
  const calls = [];
  const controller = createLyricsController({
    api: {
      getTrack: async (args) => { calls.push(['get', args]); return readyResult(args.trackId); },
      search: async (args) => { calls.push(['search', args]); return emptyResult(); },
      selectCandidate: async () => { throw new Error('unexpected candidate selection'); },
      cancelSearch: async (args) => { calls.push(['cancel', args]); },
    },
    onChange() {},
  });

  await controller.setTrack('track-local');
  await controller.setTrack('track-local');
  assert.deepEqual(calls, [['get', { trackId: 'track-local' }]]);
  controller.dispose();
});

test('remote lookup starts only after an empty local result and can be cancelled', async () => {
  const searchResult = deferred();
  const calls = [];
  const controller = createLyricsController({
    api: {
      getTrack: async (args) => { calls.push(['get', args]); return emptyResult(); },
      search: (args) => { calls.push(['search', args]); return searchResult.promise; },
      selectCandidate: async () => { throw new Error('unexpected candidate selection'); },
      cancelSearch: async (args) => { calls.push(['cancel', args]); },
    },
    onChange() {},
    createRequestId: () => 'request-1',
  });

  void controller.setTrack('track-empty');
  await Promise.resolve();
  await Promise.resolve();
  assert.deepEqual(calls.slice(0, 2), [
    ['get', { trackId: 'track-empty' }],
    ['search', { trackId: 'track-empty', requestId: 'request-1' }],
  ]);
  controller.cancelSearch();
  assert.deepEqual(calls.at(-1), ['cancel', { requestId: 'request-1' }]);
  searchResult.resolve(emptyResult());
  await Promise.resolve();
  assert.equal(controller.getState().isSearching, false);
  assert.equal(controller.getState().trackId, 'track-empty');
  controller.dispose();
});

test('track switch cancels the old lookup and ignores its late response', async () => {
  const oldSearch = deferred();
  const calls = [];
  const states = [];
  const controller = createLyricsController({
    api: {
      getTrack: async ({ trackId }) => trackId === 'old' ? emptyResult() : readyResult(trackId, 'manual'),
      search: (args) => { calls.push(['search', args]); return oldSearch.promise; },
      selectCandidate: async () => { throw new Error('unexpected candidate selection'); },
      cancelSearch: async (args) => { calls.push(['cancel', args]); },
    },
    onChange: (state) => states.push(state),
    createRequestId: () => 'old-request',
  });

  void controller.setTrack('old');
  await Promise.resolve();
  await Promise.resolve();
  const staleSearch = oldSearch.promise;
  await controller.setTrack('new');
  oldSearch.resolve(readyResult('old', 'netease'));
  await staleSearch;

  assert.deepEqual(calls, [
    ['search', { trackId: 'old', requestId: 'old-request' }],
    ['cancel', { requestId: 'old-request' }],
  ]);
  assert.equal(controller.getState().trackId, 'new');
  assert.equal(controller.getState().lyrics?.source, 'manual');
  assert.equal(states.some((state) => state.trackId === 'old' && state.lyrics?.source === 'netease'), false);
  controller.dispose();
});

test('manual candidate selection saves only for the still-active track', async () => {
  const calls = [];
  const controller = createLyricsController({
    api: {
      getTrack: async () => ({
        lyrics: null,
        candidates: [{ id: 'candidate-1', provider: 'qqmusic', title: '候選', artist: '歌手', album: null, durationMs: null, score: 0.84, confidence: 'medium', reasons: [], previewLines: [], hasSyncedLyrics: true, hasWordSyncedLyrics: false, hasTranslation: false, hasRomanization: false }],
        status: 'candidates',
        error: null,
      }),
      search: async () => ({
        lyrics: null,
        candidates: [{ id: 'candidate-1', provider: 'qqmusic', title: '候選', artist: '歌手', album: null, durationMs: null, score: 0.84, confidence: 'medium', reasons: [], previewLines: [], hasSyncedLyrics: true, hasWordSyncedLyrics: false, hasTranslation: false, hasRomanization: false }],
        status: 'candidates',
        error: null,
      }),
      selectCandidate: async (args) => { calls.push(['select', args]); return lyric(args.trackId, 'manual', '手動指定'); },
      cancelSearch: async (args) => { calls.push(['cancel', args]); },
    },
    onChange() {},
    createRequestId: () => 'request-2',
  });

  await controller.setTrack('track-candidate');
  await controller.selectCandidate('candidate-1');
  assert.deepEqual(calls, [['select', { trackId: 'track-candidate', candidateId: 'candidate-1' }]]);
  assert.equal(controller.getState().lyrics?.source, 'manual');
  assert.equal(controller.getState().lyrics?.lines[0].text, '手動指定');
  controller.dispose();
});

test('QQ QRC-only candidates explain unavailable preview and remain manually selectable', async () => {
  const candidate = {
    id: 'qrc-only',
    provider: 'qqmusic',
    title: 'QRC 歌詞',
    artist: '歌手',
    album: null,
    durationMs: null,
    score: 0.72,
    confidence: 'medium',
    reasons: ['標題相符'],
    previewLines: [],
    hasSyncedLyrics: false,
    hasWordSyncedLyrics: true,
    hasTranslation: false,
    hasRomanization: false,
  };
  assert.deepEqual(getCandidatePresentation(candidate), {
    formatLabel: '逐字歌詞',
    previewNotice: '原始 QRC 尚未解碼，目前無法預覽；仍可使用「選擇」保存。',
  });

  const calls = [];
  const controller = createLyricsController({
    api: {
      getTrack: async () => emptyResult(),
      search: async () => ({ lyrics: null, candidates: [candidate], status: 'candidates', error: null }),
      selectCandidate: async (args) => {
        calls.push(args);
        return { ...lyric(args.trackId, 'manual', ''), synced: false, lines: [] };
      },
      cancelSearch: async () => {},
    },
    onChange() {},
    createRequestId: () => 'qrc-request',
  });

  await controller.setTrack('qrc-track');
  await controller.selectCandidate(candidate.id);
  assert.deepEqual(calls, [{ trackId: 'qrc-track', candidateId: 'qrc-only' }]);
  assert.equal(controller.getState().lyrics?.source, 'manual');
  assert.equal(controller.getState().lyrics?.lines.length, 0);
  controller.dispose();
});

test('dispose cancels an active provider request and fences its result', async () => {
  const pending = deferred();
  const calls = [];
  const states = [];
  const controller = createLyricsController({
    api: {
      getTrack: async () => emptyResult(),
      search: () => pending.promise,
      selectCandidate: async () => { throw new Error('unexpected candidate selection'); },
      cancelSearch: async (args) => { calls.push(args); },
    },
    onChange: (state) => states.push(state),
    createRequestId: () => 'dispose-request',
  });

  void controller.setTrack('track-dispose');
  await Promise.resolve();
  await Promise.resolve();
  controller.dispose();
  pending.resolve(readyResult('track-dispose', 'netease'));
  await Promise.resolve();
  assert.deepEqual(calls, [{ requestId: 'dispose-request' }]);
  assert.equal(states.some((state) => state.lyrics?.source === 'netease'), false);
});
test('manual searchAgain keeps selection open when lyrics already exist', async () => {
  const pending = deferred();
  const existing = lyric('track-ready', 'local');
  const candidate = {
    id: 'candidate-manual',
    provider: 'netease',
    title: '候選曲',
    artist: '歌手',
    album: null,
    durationMs: 10000,
    score: 0.91,
    confidence: 'high',
    reasons: ['標題相符'],
    previewLines: ['預覽'],
    hasSyncedLyrics: true,
    hasWordSyncedLyrics: false,
    hasTranslation: false,
    hasRomanization: false,
  };
  const controller = createLyricsController({
    api: {
      getTrack: async () => readyResult('track-ready', 'local'),
      search: async (args) => {
        assert.equal(args.manual, true);
        return pending.promise;
      },
      selectCandidate: async () => existing,
      cancelSearch: () => {},
    },
    onChange: () => {},
  });

  await controller.setTrack('track-ready');
  assert.equal(controller.getState().phase, 'ready');
  assert.equal(controller.getState().lyrics?.source, 'local');

  const searching = controller.searchAgain();
  assert.equal(controller.getState().phase, 'searching');
  assert.equal(controller.getState().selectionMode, true);
  assert.equal(controller.getState().isSearching, true);

  pending.resolve({
    lyrics: existing,
    candidates: [candidate],
    status: 'candidates',
    error: null,
  });
  await searching;

  assert.equal(controller.getState().phase, 'candidates');
  assert.equal(controller.getState().selectionMode, true);
  assert.equal(controller.getState().candidates.length, 1);
  assert.equal(controller.getState().lyrics?.source, 'local');

  controller.dismissSelection();
  assert.equal(controller.getState().phase, 'ready');
  assert.equal(controller.getState().selectionMode, false);
  assert.equal(controller.getState().pickerClosed, true);
  assert.equal(controller.getState().candidates.length, 0);
  assert.equal(controller.getState().lyrics?.source, 'local');
  controller.dispose();
});

test('dismiss closes the picker when it opened without selection mode', async () => {
  const controller = createLyricsController({
    api: {
      getTrack: async () => ({ lyrics: null, candidates: [], status: 'empty', error: null }),
      search: async () => ({ lyrics: null, candidates: [], status: 'empty', error: null }),
      selectCandidate: async () => {
        throw new Error('unused');
      },
      cancelSearch: () => {},
      clearTrack: async () => ({ lyrics: null, candidates: [], status: 'empty', error: null }),
    },
    onChange: () => {},
  });
  await controller.setTrack('track-empty');
  assert.equal(controller.getState().selectionMode, false);
  assert.equal(controller.getState().pickerClosed, false);
  controller.dismissSelection();
  assert.equal(controller.getState().pickerClosed, true);
  assert.equal(controller.getState().phase, 'empty');
  assert.equal(controller.getState().selectionMode, false);
  controller.dispose();
});

test('manual search keeps the lyrics already on screen when the result omits them', async () => {
  const existing = lyric('track-ready', 'local');
  const controller = createLyricsController({
    api: {
      getTrack: async () => readyResult('track-ready', 'local'),
      search: async () => ({ lyrics: null, candidates: [], status: 'empty', error: null }),
      selectCandidate: async () => existing,
      cancelSearch: () => {},
    },
    onChange: () => {},
  });
  await controller.setTrack('track-ready');
  await controller.searchAgain();
  assert.equal(controller.getState().selectionMode, true);
  assert.equal(controller.getState().lyrics?.source, 'local');
  controller.dispose();
});



test('searchAgain forwards optional query to lyrics_search', async () => {
  const calls = [];
  const controller = createLyricsController({
    api: {
      getTrack: async () => emptyResult(),
      search: async (args) => { calls.push(['search', args]); return emptyResult(); },
      selectCandidate: async () => { throw new Error('unused'); },
      cancelSearch: async () => {},
    },
    onChange() {},
    createRequestId: () => 'request-q',
  });
  void controller.setTrack('track-q');
  await Promise.resolve();
  await Promise.resolve();
  calls.length = 0;
  await controller.searchAgain({ query: '  春日影  ' });
  assert.deepEqual(calls, [['search', { trackId: 'track-q', requestId: 'request-q', manual: true, query: '春日影' }]]);
  controller.dispose();
});

test('automatic track-change search leaves the Simplified query to the backend', async () => {
  const calls = [];
  const controller = createLyricsController({
    api: {
      getTrack: async () => emptyResult(),
      search: async (args) => { calls.push(args); return emptyResult(); },
      selectCandidate: async () => { throw new Error('unused'); },
      cancelSearch: async () => {},
    },
    onChange() {},
    createRequestId: () => 'request-auto-variants',
  });

  await controller.setTrack('track-auto');

  // ADR 0030: lyrics_search converts the track's title and artist to Simplified itself.
  assert.deepEqual(calls, [{ trackId: 'track-auto', requestId: 'request-auto-variants' }]);
  controller.dispose();
});

test('a manual Chinese query is sent as typed, without script alternates', async () => {
  const calls = [];
  const controller = createLyricsController({
    api: {
      getTrack: async () => emptyResult(),
      search: async (args) => { calls.push(args); return emptyResult(); },
      selectCandidate: async () => { throw new Error('unused'); },
      cancelSearch: async () => {},
    },
    onChange() {},
    createRequestId: () => 'request-manual-zh',
  });
  void controller.setTrack('track-manual-zh');
  await Promise.resolve();
  await Promise.resolve();
  calls.length = 0;
  await controller.searchAgain({ query: '蘋果頭髮' });
  assert.deepEqual(calls, [{ trackId: 'track-manual-zh', requestId: 'request-manual-zh', manual: true, query: '蘋果頭髮' }]);
  controller.dispose();
});

test('cleared getTrack status skips automatic provider search and keeps the picker closed', async () => {
  const calls = [];
  const controller = createLyricsController({
    api: {
      getTrack: async (args) => {
        calls.push(['get', args]);
        return { lyrics: null, candidates: [], status: 'cleared', error: null };
      },
      search: async (args) => {
        calls.push(['search', args]);
        return emptyResult();
      },
      selectCandidate: async () => { throw new Error('unexpected candidate selection'); },
      cancelSearch: async () => {},
    },
    onChange() {},
  });

  await controller.setTrack('track-cleared');
  assert.deepEqual(calls, [['get', { trackId: 'track-cleared' }]]);
  assert.equal(controller.getState().phase, 'empty');
  assert.equal(controller.getState().lyrics, null);
  assert.equal(controller.getState().lyricsRemoved, true);
  assert.equal(controller.getState().selectionMode, false);
  assert.equal(
    controller.getState().pickerClosed,
    true,
    'cleared tracks must not reopen the candidate picker on the next visit',
  );
  controller.dispose();
});

test('removeLyrics clears persisted lyrics, dismisses picker, and skips auto-search', async () => {
  let clearCalls = 0;
  let searchCalls = 0;
  const existing = lyric('t-remove', 'manual');
  const candidate = {
    id: 'candidate-remove',
    provider: 'netease',
    title: '移除測試',
    artist: '歌手',
    album: null,
    durationMs: 120000,
    score: 0.9,
    confidence: 'high',
    reasons: [],
    previewLines: ['line'],
    synced: true,
    hasTranslation: false,
    hasRomanization: false,
  };
  let pending;
  const searching = new Promise((resolve) => {
    pending = { resolve };
  });
  const controller = createLyricsController({
    api: {
      getTrack: async () => ({ lyrics: existing, candidates: [], status: 'ready', error: null }),
      search: async () => {
        searchCalls += 1;
        return searching;
      },
      selectCandidate: async () => existing,
      cancelSearch: async () => {},
      clearTrack: async ({ trackId }) => {
        clearCalls += 1;
        assert.equal(trackId, 't-remove');
        return { lyrics: null, candidates: [], status: 'cleared', error: null };
      },
    },
    onChange: () => {},
  });

  await controller.setTrack('t-remove');
  const searchPromise = controller.searchAgain();
  pending.resolve({
    lyrics: existing,
    candidates: [candidate],
    status: 'candidates',
    error: null,
  });
  await searchPromise;
  assert.equal(controller.getState().selectionMode, true);
  assert.ok(controller.getState().lyrics);

  const searchesBefore = searchCalls;
  await controller.removeLyrics();
  assert.equal(clearCalls, 1);
  assert.equal(controller.getState().selectionMode, false);
  assert.equal(controller.getState().pickerClosed, true);
  assert.equal(controller.getState().lyrics, null);
  assert.equal(controller.getState().lyricsRemoved, true);
  assert.equal(controller.getState().phase, 'empty');
  assert.equal(searchCalls, searchesBefore, 'explicit remove must not auto-search');
  controller.dispose();
});

test('removeLyrics on a song with its own lyrics shows them normally and still skips auto-search', async () => {
  let searchCalls = 0;
  const online = lyric('t-remove-local', 'manual');
  const embedded = lyric('t-remove-local', 'embedded');
  const controller = createLyricsController({
    api: {
      getTrack: async () => ({ lyrics: online, candidates: [], status: 'ready', error: null }),
      search: async () => {
        searchCalls += 1;
        return emptyResult();
      },
      selectCandidate: async () => online,
      cancelSearch: async () => {},
      // The backend keeps only the automatic search off: the file's embedded lyrics come back ready.
      clearTrack: async () => ({ lyrics: embedded, candidates: [], status: 'ready', error: null }),
    },
    onChange: () => {},
  });

  await controller.setTrack('t-remove-local');
  await controller.removeLyrics();
  const state = controller.getState();
  assert.equal(state.phase, 'ready');
  assert.equal(state.lyrics, embedded);
  assert.equal(state.lyricsRemoved, false, 'the lyrics pane stays when the file has lyrics');
  assert.equal(state.pickerClosed, true);
  assert.equal(searchCalls, 0, 'explicit remove must not auto-search');
  controller.dispose();
});

test('a cleared track whose file has lyrics loads them as ready without an automatic search', async () => {
  const calls = [];
  const embedded = lyric('track-cleared-local', 'embedded');
  const controller = createLyricsController({
    api: {
      getTrack: async (args) => {
        calls.push(['get', args]);
        return { lyrics: embedded, candidates: [], status: 'ready', error: null };
      },
      search: async (args) => {
        calls.push(['search', args]);
        return emptyResult();
      },
      selectCandidate: async () => { throw new Error('unexpected candidate selection'); },
      cancelSearch: async () => {},
    },
    onChange() {},
  });

  await controller.setTrack('track-cleared-local');
  assert.deepEqual(calls, [['get', { trackId: 'track-cleared-local' }]]);
  assert.equal(controller.getState().phase, 'ready');
  assert.equal(controller.getState().lyricsRemoved, false);
  controller.dispose();
});

test('load more appends the next lyric page until the source is exhausted', async () => {
  const candidate = (id) => ({
    id,
    provider: 'qqmusic',
    title: id,
    artist: '歌手',
    album: null,
    durationMs: null,
    score: 0.5,
    confidence: 'low',
    reasons: [],
    previewLines: [],
    hasSyncedLyrics: true,
    hasWordSyncedLyrics: false,
    hasTranslation: false,
    hasRomanization: false,
  });
  const controller = createLyricsController({
    api: {
      getTrack: async () => ({ lyrics: null, candidates: [], status: 'empty', error: null }),
      search: async () => ({
        lyrics: null,
        candidates: [candidate('first')],
        status: 'candidates',
        error: null,
        hasMore: true,
      }),
      selectCandidate: async () => lyric('t-more', 'manual', '手動'),
      cancelSearch: async () => {},
      clearTrack: async () => ({ lyrics: null, candidates: [], status: 'empty', error: null }),
      loadMore: async () => ({
        lyrics: null,
        candidates: [candidate('second')],
        status: 'candidates',
        error: null,
        hasMore: false,
      }),
    },
    onChange() {},
  });
  await controller.setTrack('t-more');
  assert.equal(controller.getState().hasMore, true);
  assert.deepEqual(controller.getState().candidates.map((item) => item.id), ['first']);
  await controller.loadMore();
  assert.deepEqual(controller.getState().candidates.map((item) => item.id), ['first', 'second']);
  assert.equal(controller.getState().hasMore, false);
  await controller.loadMore();
  assert.deepEqual(controller.getState().candidates.map((item) => item.id), ['first', 'second']);
  controller.dispose();
});


test('reload reads the current track again and keeps showing the old lyrics until the new ones arrive', async () => {
  let embedded = 'old';
  const states = [];
  const controller = createLyricsController({
    api: {
      getTrack: async ({ trackId }) => ({ lyrics: lyric(trackId, 'embedded', embedded), candidates: [], status: 'ready', error: null }),
      search: async () => emptyResult(),
      selectCandidate: async () => lyric('a'),
      cancelSearch: async () => {},
      clearTrack: async () => emptyResult(),
    },
    onChange: (state) => states.push(state),
  });
  await controller.setTrack('a');
  assert.equal(controller.getState().lyrics.lines[0].text, 'old');

  embedded = 'edited';
  const before = states.length;
  const reloading = controller.reload();
  assert.equal(controller.getState().lyrics.lines[0].text, 'old', 'the old lyrics stay while reloading');
  await reloading;
  assert.equal(controller.getState().lyrics.lines[0].text, 'edited');
  assert.ok(states.length > before);
  // Nothing to reload without a track.
  await controller.setTrack(null);
  await controller.reload();
  assert.equal(controller.getState().lyrics, null);
});
