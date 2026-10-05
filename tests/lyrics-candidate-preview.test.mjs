import assert from 'node:assert/strict';
import { test } from 'node:test';
import { filterLyricsCandidates, getCandidatePresentation } from '../src/lib/lyrics-candidate-preview.js';

/** @returns {import('../src/lib/ipc').LyricsCandidate} */
function candidate(partial) {
  return {
    id: 'c1',
    provider: 'netease',
    title: 'Title',
    artist: 'Artist',
    album: 'Album',
    durationMs: 1000,
    score: 1,
    confidence: 'high',
    hasSyncedLyrics: true,
    previewLines: ['line one', 'line two'],
    reasons: ['title match'],
    ...partial,
  };
}

test('filterLyricsCandidates returns all when query empty', () => {
  const list = [candidate({ id: 'a' }), candidate({ id: 'b', title: 'Other' })];
  assert.equal(filterLyricsCandidates(list, '').length, 2);
  assert.equal(filterLyricsCandidates(list, '   ').length, 2);
});

test('filterLyricsCandidates matches title artist album preview reasons', () => {
  const list = [
    candidate({ id: 'a', title: '春日', artist: 'A' }),
    candidate({ id: 'b', title: 'X', artist: '夏夜', album: 'EP' }),
    candidate({ id: 'c', title: 'Y', previewLines: ['冬日的風'] }),
    candidate({ id: 'd', title: 'Z', reasons: ['romanization hit'] }),
  ];
  assert.deepEqual(filterLyricsCandidates(list, '春').map((c) => c.id), ['a']);
  assert.deepEqual(filterLyricsCandidates(list, '夏夜').map((c) => c.id), ['b']);
  assert.deepEqual(filterLyricsCandidates(list, '冬日').map((c) => c.id), ['c']);
  assert.deepEqual(filterLyricsCandidates(list, 'romanization').map((c) => c.id), ['d']);
  assert.equal(filterLyricsCandidates(list, 'nope').length, 0);
});

test('getCandidatePresentation still labels synced lyrics', () => {
  const presentation = getCandidatePresentation(candidate({}));
  assert.equal(presentation.formatLabel, '同步歌詞');
  assert.equal(presentation.previewNotice, null);
});