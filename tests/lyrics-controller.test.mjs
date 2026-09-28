import assert from 'node:assert/strict';
import test from 'node:test';
import {
  buildTimedLyricTimeline,
  findActiveLyricIndex,
  getPlainLyricWindow,
  getTimedLyricWindow,
  PLAIN_LYRIC_ROW_HEIGHT,
  TIMED_LYRIC_ROW_HEIGHT,
} from '../src/lib/lyrics-window.js';
import { createLyricsController } from '../src/lib/lyrics-controller.js';

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

test('timed lyric viewport keeps at most the active line and six neighbors on each side', () => {
  const timeline = buildTimedLyricTimeline(Array.from({ length: 100_000 }, (_, index) => ({
    startMs: index * 1_000,
    text: `歌詞 ${index}`,
    translation: null,
    romanization: null,
  })));
  const middle = getTimedLyricWindow(timeline, 50_000);
  const beginning = getTimedLyricWindow(timeline, -1);
  const ending = getTimedLyricWindow(timeline, timeline.length - 1);

  assert.equal(middle.rows.length, 13);
  assert.equal(middle.rows[0].index, 49_994);
  assert.equal(middle.rows.at(-1).index, 50_006);
  assert.equal(middle.beforeHeight, 49_994 * TIMED_LYRIC_ROW_HEIGHT);
  assert.equal(middle.afterHeight, (100_000 - 50_007) * TIMED_LYRIC_ROW_HEIGHT);
  assert.equal(beginning.rows.length, 7);
  assert.equal(ending.rows.length, 7);
  assert.ok(middle.rows.length < timeline.length);
});

test('plain lyrics use a bounded scroll window without claiming synchronized timing', () => {
  const first = getPlainLyricWindow(10_000, PLAIN_LYRIC_ROW_HEIGHT * 100, PLAIN_LYRIC_ROW_HEIGHT * 5);
  const top = getPlainLyricWindow(10_000, 0, PLAIN_LYRIC_ROW_HEIGHT * 5);
  const bottom = getPlainLyricWindow(10_000, Number.MAX_SAFE_INTEGER, PLAIN_LYRIC_ROW_HEIGHT * 5);

  assert.equal(first.start, 94);
  assert.equal(first.end, 111);
  assert.ok(first.end - first.start <= 5 + 12);
  assert.equal(first.beforeHeight, 94 * PLAIN_LYRIC_ROW_HEIGHT);
  assert.equal(top.start, 0);
  assert.equal(bottom.end, 10_000);
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
