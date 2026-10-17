import assert from 'node:assert/strict';
import fs from 'node:fs';
import test from 'node:test';
import { HAND_WRITTEN_LANGUAGES, catalogUpToDate, extractKeys, translationGaps } from '../scripts/i18n-catalog.mjs';

const catalog = JSON.parse(fs.readFileSync(new URL('../src/lib/i18n/zh-CN.json', import.meta.url), 'utf8'));
const placeholders = (text) => [...text.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort().join(',');

test('the Simplified catalog is generated from every tr() call in src (run node scripts/i18n-catalog.mjs)', () => {
  assert.equal(catalogUpToDate().upToDate, true);
  assert.deepEqual(Object.keys(catalog).sort(), extractKeys().sort());
});

test('translations keep their placeholders and are never empty', () => {
  for (const [source, translated] of Object.entries(catalog)) {
    assert.ok(translated.trim().length > 0, `empty translation for ${source}`);
    assert.equal(placeholders(translated), placeholders(source), `placeholders differ: ${source}`);
  }
});

test('Mainland wording replaces the Taiwan terms in the Simplified catalog', () => {
  const text = Object.values(catalog).join('\n');
  for (const word of ['演出者', '播放清单', '预设', '拷贝', '激活', '工作钮', '设置档']) {
    assert.ok(!text.includes(word), `${word} should have been replaced`);
  }
  assert.equal(catalog['播放清單'], '播放列表');
  assert.equal(catalog['演出者'], '艺术家');
  assert.equal(catalog['設定'], '设置');
});

for (const language of HAND_WRITTEN_LANGUAGES) {
  const handWritten = JSON.parse(fs.readFileSync(new URL(`../src/lib/i18n/${language}.json`, import.meta.url), 'utf8'));

  test(`the ${language} catalog covers every tr() string (run node scripts/i18n-catalog.mjs --missing)`, () => {
    const { missing, extra } = translationGaps(language);
    assert.deepEqual(missing, [], `strings without a ${language} translation`);
    assert.deepEqual(extra, [], `${language} entries that no tr() uses any more`);
  });

  test(`${language} translations keep their placeholders and are never empty`, () => {
    for (const [source, translated] of Object.entries(handWritten)) {
      assert.ok(translated.trim().length > 0, `empty ${language} translation for ${source}`);
      assert.equal(placeholders(translated), placeholders(source), `placeholders differ: ${source}`);
    }
  });
}

test('the unknown placeholder reads 未知 in Japanese and unknown in English', () => {
  const ja = JSON.parse(fs.readFileSync(new URL('../src/lib/i18n/ja.json', import.meta.url), 'utf8'));
  const en = JSON.parse(fs.readFileSync(new URL('../src/lib/i18n/en.json', import.meta.url), 'utf8'));
  assert.equal(ja['未知'], '未知');
  assert.equal(en['未知'], 'unknown');
});
