import assert from 'node:assert/strict';
import test from 'node:test';
import { buildKeepZhiOverrides, convertTrackLyrics } from '../src/lib/lyrics-traditional.js';

test('simplified lyrics convert to traditional glyphs without replacing wording', async () => {
  const converted = await convertTrackLyrics({
    trackId: 'track-1',
    source: 'local',
    title: '头发',
    artist: null,
    album: null,
    durationMs: 1000,
    offsetMs: 0,
    synced: true,
    lines: [
      { startMs: 0, text: '头发与后面', translation: '软件界面', romanization: 'toufa' },
      { startMs: 1000, text: '', translation: null, romanization: null },
    ],
  });

  assert.equal(converted.lines[0].text, '頭髮與後面');
  assert.equal(converted.lines[0].translation, '軟件界面');
  assert.equal(converted.lines[0].romanization, 'toufa');
  assert.equal(converted.lines[1].text, '');
  assert.equal(converted.title, '头发');
  assert.notEqual(converted.lines[0].translation, '軟體界面');
});

test('conversion is line-based and preserves Japanese lines containing hiragana or katakana', async () => {
  const lyrics = {
    trackId: 'track-jp',
    source: 'local',
    title: '頭髮',
    artist: null,
    album: null,
    durationMs: 3000,
    offsetMs: 0,
    synced: true,
    lines: [
      { startMs: 0, text: '头发与后面', translation: '软件界面', romanization: null },
      { startMs: 1000, text: '头发を切った後', translation: '软体界面です', romanization: 'fa wu o kit ta' },
      { startMs: 2000, text: '头发とカタカナ', translation: '繁體中文', romanization: null },
    ],
  };

  const converted = await convertTrackLyrics(lyrics);

  assert.equal(converted.lines[0].text, '頭髮與後面');
  assert.equal(converted.lines[0].translation, '軟件界面');
  assert.equal(converted.lines[1].text, '头发を切った後');
  assert.equal(converted.lines[1].translation, '软体界面です');
  assert.equal(converted.lines[2].text, '头发とカタカナ');
  assert.equal(converted.lines[2].translation, '繁體中文');
  assert.equal(converted.lines[1].romanization, 'fa wu o kit ta');
  assert.equal(lyrics.lines[1].text, '头发を切った後');
});

const lyricsWith = (texts) => ({
  trackId: 'track-zhi', source: 'local', title: 't', artist: null, album: null, durationMs: 1000, offsetMs: 0, synced: true,
  lines: texts.map((text, index) => ({ startMs: index * 1000, text, translation: null, romanization: null })),
});

test('Simplified 只 is never converted to 隻, while other phrase conversions still apply', async () => {
  const input = ['只有', '这只有你', '一只猫', '两只老虎', '船只有限', '只字不提', '我只是想你', '头发', '后面的时间', '干净的头发'];
  const expected = ['只有', '這只有你', '一只貓', '兩只老虎', '船只有限', '只字不提', '我只是想你', '頭髮', '後面的時間', '乾淨的頭髮'];
  const converted = await convertTrackLyrics(lyricsWith(input));
  assert.deepEqual(converted.lines.map((line) => line.text), expected);
});

test('只 overrides restore 只 only where STPhrases turned it into 隻', () => {
  assert.deepEqual(buildKeepZhiOverrides('一只 一隻|只有 只有|这只 這隻|形单影只 形單影隻|头发 頭髮'), [
    ['一只', '一只'],
    ['这只', '這只'],
    ['形单影只', '形單影只'],
  ]);
});
