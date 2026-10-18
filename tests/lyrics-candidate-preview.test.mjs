import assert from 'node:assert/strict';
import { test } from 'node:test';
import { effectiveCandidateFilter, filterLyricsCandidates, getCandidatePresentation } from '../src/lib/lyrics-candidate-preview.js';

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
    hasWordSyncedLyrics: false,
    hasTranslation: false,
    hasRomanization: false,
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

test('getCandidatePresentation distinguishes word-timed, line-timed, plain, and undecoded QRC', () => {
  const wordTimed = getCandidatePresentation(candidate({ hasWordSyncedLyrics: true }));
  assert.equal(wordTimed.formatLabel, '逐字歌詞');
  assert.equal(wordTimed.previewNotice, null);

  const lineTimed = getCandidatePresentation(candidate({}));
  assert.equal(lineTimed.formatLabel, '逐行歌詞');
  assert.equal(lineTimed.previewNotice, null);

  const plain = getCandidatePresentation(candidate({ hasSyncedLyrics: false, previewLines: [] }));
  assert.equal(plain.formatLabel, '純歌詞');
  assert.equal(plain.previewNotice, null);

  const qqPlain = getCandidatePresentation(candidate({
    provider: 'qqmusic',
    hasSyncedLyrics: false,
    previewLines: [],
  }));
  assert.equal(qqPlain.formatLabel, '純歌詞');
  assert.equal(qqPlain.previewNotice, null);

  const auxiliaries = getCandidatePresentation(candidate({ hasTranslation: true, hasRomanization: true }));
  assert.equal(auxiliaries.formatLabel, '逐行歌詞 · 譯 · 羅');

  const translationOnly = getCandidatePresentation(candidate({ hasTranslation: true }));
  assert.equal(translationOnly.formatLabel, '逐行歌詞 · 譯');

  const romanizationOnly = getCandidatePresentation(candidate({ hasRomanization: true }));
  assert.equal(romanizationOnly.formatLabel, '逐行歌詞 · 羅');

  const undecodedQrc = getCandidatePresentation(candidate({
    provider: 'qqmusic',
    hasSyncedLyrics: false,
    hasWordSyncedLyrics: true,
    previewLines: [],
  }));
  assert.equal(undecodedQrc.formatLabel, '逐字歌詞');
  assert.equal(undecodedQrc.previewNotice, '原始 QRC 尚未解碼，目前無法預覽；仍可使用「選擇」保存。');
});

test('the text of the search that produced the results does not filter them again', () => {
  // The providers answer "1 2 FanClub" with titles that spell it differently; filtering by the literal text hid them all.
  const answers = [
    candidate({ id: 'a', title: '1,2, Fanclub', artist: 'x', previewLines: [], reasons: [] }),
    candidate({ id: 'b', title: '12fanclub', artist: 'y', previewLines: [], reasons: [] }),
    candidate({ id: 'c', title: 'いーあるふぁんくらぶ', artist: 'z', previewLines: [], reasons: [] }),
  ];
  assert.equal(filterLyricsCandidates(answers, '1 2 FanClub').length, 0, 'the old behaviour hid everything');
  assert.equal(effectiveCandidateFilter('1 2 FanClub', '1 2 FanClub'), '');
  assert.equal(filterLyricsCandidates(answers, effectiveCandidateFilter(' 1 2 FanClub ', '1 2 FanClub')).length, 3);
  // Typing more (or something else) after the search filters the shown results again.
  assert.equal(effectiveCandidateFilter('1 2 FanClub gumi', '1 2 FanClub'), '1 2 FanClub gumi');
  assert.equal(effectiveCandidateFilter('gumi', ''), 'gumi', 'before any search the typed text filters as before');
  assert.equal(effectiveCandidateFilter('', ''), '');
  assert.equal(effectiveCandidateFilter(undefined, undefined), '');
});
