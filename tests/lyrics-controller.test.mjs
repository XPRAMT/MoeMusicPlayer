import assert from 'node:assert/strict';
import test from 'node:test';
import {
  buildTimedLyricTimeline,
  buildLyricsLayout,
  findLyricsRowAtOffset,
  findActiveLyricIndex,
  getPlainLyricWindow,
  getTimedLyricWindow,
} from '../src/lib/lyrics-window.js';
import {
  DEFAULT_LYRICS_PREFERENCES,
  getDisplayActiveLyricIndex,
  getLyricLineOpacity,
  normalizeLyricsPreferences,
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

test('timed lyric viewport keeps at most the active line and twenty neighbors on each side', () => {
  const preferences = DEFAULT_LYRICS_PREFERENCES;
  const timeline = buildTimedLyricTimeline(Array.from({ length: 100_000 }, (_, index) => ({
    startMs: index * 1_000,
    text: index % 7 === 0 ? '  ' : `歌詞 ${index}`,
    translation: null,
    romanization: null,
  })));
  const layout = buildLyricsLayout(timeline, preferences, preferences.lineGapPx);
  const middle = getTimedLyricWindow(layout, 50_000);
  const beginning = getTimedLyricWindow(layout, -1);
  const ending = getTimedLyricWindow(layout, timeline.length - 1);

  assert.ok(middle.rows.length <= 41);
  assert.ok(middle.rows.includes(50_000));
  assert.equal(middle.beforeHeight, layout.offsets[middle.start]);
  assert.equal(middle.afterHeight, layout.totalHeight - layout.offsets[middle.end]);
  assert.ok(beginning.rows.length <= 41);
  assert.ok(ending.rows.length <= 41);
  assert.ok(middle.rows.length < timeline.length);
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
    inactiveOpacityPercent: 1,
    primaryFontSizePx: 99,
    auxiliaryFontSizePx: 4,
    lineGapPx: 99,
  }), {
    showTranslation: true,
    showRomanization: false,
    inactiveOpacityPercent: 10,
    primaryFontSizePx: 36,
    auxiliaryFontSizePx: 9,
    lineGapPx: 64,
  });
  assert.equal(normalizeLyricsPreferences({ inactiveOpacityPercent: 100.4 }).inactiveOpacityPercent, 100);
  assert.equal(normalizeLyricsPreferences({ lineGapPx: -1 }).lineGapPx, 0);
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
  const primary = base.primaryFontSizePx * 1.35 + 14;
  const oneAux = primary + 2 + base.auxiliaryFontSizePx * 1.25;
  const twoAux = primary + 4 + 2 * base.auxiliaryFontSizePx * 1.25;

  assert.equal(layout.heights[0], primary);
  assert.equal(layout.heights[1], primary);
  assert.equal(layout.heights[2], oneAux);
  assert.equal(layout.heights[3], oneAux);
  assert.equal(layout.heights[4], twoAux);
  assert.equal(layout.heights[5], 0);
  assert.deepEqual([...layout.visibleIndices], [0, 1, 2, 3, 4]);
  assert.equal(layout.gaps[4], 0);
  assert.equal(layout.totalHeight, [...layout.heights].reduce((sum, height) => sum + height, 0) + 4 * 24);

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
  assert.equal(getLyricsOffset(layout, 2), initialNext + 18);
  assert.equal(layout.totalHeight, getLyricsOffset(layout, 3));
});

test('layout binary search skips empty rows, honors gap extremes and keeps all-empty documents bounded', () => {
  const lines = [
    { text: '甲', translation: null, romanization: null },
    { text: '', translation: null, romanization: null },
    { text: '乙', translation: null, romanization: null },
  ];
  const noGap = buildLyricsLayout(lines, DEFAULT_LYRICS_PREFERENCES, 0);
  const maxGap = buildLyricsLayout(lines, DEFAULT_LYRICS_PREFERENCES, 64);
  assert.ok(Math.abs(maxGap.totalHeight - noGap.totalHeight - 64) < 0.001);
  assert.equal(findLyricsRowAtOffset(maxGap, maxGap.offsets[1]), 2);
  assert.equal(maxGap.gaps[0], 64);
  assert.equal(maxGap.gaps[2], 0);

  const empty = buildLyricsLayout(Array.from({ length: 100_000 }, () => ({ text: ' ', translation: null, romanization: null })), DEFAULT_LYRICS_PREFERENCES, 64);
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

test('plain, stopped, and cue-less lyrics stay fully opaque; playing and paused keep the cue', () => {
  const dimOpacity = 0.35;
  assert.equal(getDisplayActiveLyricIndex(2, 'playing'), 2);
  assert.equal(getDisplayActiveLyricIndex(2, 'paused'), 2);
  assert.equal(getDisplayActiveLyricIndex(2, 'stopped'), -1);
  assert.equal(getDisplayActiveLyricIndex(-1, 'playing'), -1);
  assert.equal(getLyricLineOpacity(1, 2, 35), dimOpacity);
  assert.equal(getLyricLineOpacity(2, 2, 35), 1);
  assert.equal(getLyricLineOpacity(1, -1, 35), 1);
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
        candidates: [{ id: 'candidate-1', provider: 'qqmusic', title: '候選', artist: '歌手', album: null, durationMs: null, score: 0.84, confidence: 'medium', reasons: [], previewLines: [], hasSyncedLyrics: true }],
        status: 'candidates',
        error: null,
      }),
      search: async () => ({
        lyrics: null,
        candidates: [{ id: 'candidate-1', provider: 'qqmusic', title: '候選', artist: '歌手', album: null, durationMs: null, score: 0.84, confidence: 'medium', reasons: [], previewLines: [], hasSyncedLyrics: true }],
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
  };
  assert.deepEqual(getCandidatePresentation(candidate), {
    formatLabel: 'QRC 尚未解碼',
    previewNotice: '原始 QRC 尚未解碼，目前無法預覽；仍可使用「使用這份」保存。',
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
  assert.equal(controller.getState().candidates.length, 0);
  assert.equal(controller.getState().lyrics?.source, 'local');
  controller.dispose();
});

