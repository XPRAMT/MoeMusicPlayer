import assert from 'node:assert/strict';
import test from 'node:test';
import {
  formatMessage,
  getActiveLanguage,
  htmlLang,
  normalizeLanguageChoice,
  resolveLanguage,
  resolveSystemLanguage,
  setLanguageForTesting,
  tr,
} from '../src/lib/i18n.js';

test('the system language is Simplified only for Simplified Chinese tags', () => {
  assert.equal(resolveSystemLanguage(['zh-CN']), 'zh-CN');
  assert.equal(resolveSystemLanguage(['zh-Hans-CN']), 'zh-CN');
  assert.equal(resolveSystemLanguage(['zh-SG']), 'zh-CN');
  assert.equal(resolveSystemLanguage(['zh']), 'zh-CN');
  assert.equal(resolveSystemLanguage(['zh-TW']), 'zh-TW');
  assert.equal(resolveSystemLanguage(['zh-HK']), 'zh-TW');
  assert.equal(resolveSystemLanguage(['zh-Hant-TW']), 'zh-TW');
  assert.equal(resolveSystemLanguage(['ja-JP', 'zh-CN']), 'ja', 'the first tag that has an interface decides');
  assert.equal(resolveSystemLanguage(['zh-CN', 'ja-JP']), 'zh-CN');
  assert.equal(resolveSystemLanguage(['ja']), 'ja');
  assert.equal(resolveSystemLanguage(['en-US']), 'en');
  assert.equal(resolveSystemLanguage(['fr-FR', 'en-GB']), 'en', 'tags without an interface are skipped');
  assert.equal(resolveSystemLanguage(['fr-FR']), 'en', 'a system with none of our languages uses English');
  assert.equal(resolveSystemLanguage(undefined), 'zh-TW');
  assert.equal(resolveSystemLanguage([]), 'zh-TW');
});

test('an explicit choice beats the system language and bad choices fall back to system', () => {
  assert.equal(resolveLanguage('zh-TW', ['zh-CN']), 'zh-TW');
  assert.equal(resolveLanguage('zh-CN', ['zh-TW']), 'zh-CN');
  assert.equal(resolveLanguage('system', ['zh-CN']), 'zh-CN');
  assert.equal(resolveLanguage('ja', ['en']), 'ja');
  assert.equal(resolveLanguage('en', ['zh-CN']), 'en');
  assert.equal(normalizeLanguageChoice('fr'), 'system');
  assert.equal(normalizeLanguageChoice('zh-CN'), 'zh-CN');
  assert.equal(normalizeLanguageChoice('ja'), 'ja');
  assert.equal(normalizeLanguageChoice('en'), 'en');
});

test('placeholders are filled from arrays and objects and unknown ones stay', () => {
  assert.equal(formatMessage('共 {0} 首，{1}', [3, '好']), '共 3 首，好');
  assert.equal(formatMessage('{name} 完成', { name: 'A' }), 'A 完成');
  assert.equal(formatMessage('{0} {1}', [1]), '1 {1}');
  assert.equal(formatMessage('原文', undefined), '原文');
});

test('tr falls back to the source text and uses the catalog of the active language', () => {
  setLanguageForTesting('zh-TW');
  assert.equal(getActiveLanguage(), 'zh-TW');
  assert.equal(tr('尚未收錄的字串 {0}', [1]), '尚未收錄的字串 1');
  setLanguageForTesting('zh-CN');
  assert.equal(getActiveLanguage(), 'zh-CN');
  assert.equal(tr('尚未收錄的字串 {0}', [1]), '尚未收錄的字串 1', 'a missing entry shows the source');
  setLanguageForTesting('ja');
  assert.equal(tr('設定'), '設定');
  assert.equal(tr('{0} 曲', [3]), '{0} 曲'.replace('{0}', '3'), 'a source string with no entry shows the source');
  setLanguageForTesting('en');
  assert.equal(tr('設定'), 'Settings');
  assert.equal(tr('開啟專輯「{0}」，{1} 首曲目', ['A', 3]), 'Open album “A”, 3 tracks');
  setLanguageForTesting('zh-TW');
});

test('htmlLang names the Chinese scripts', () => {
  assert.equal(htmlLang('zh-CN'), 'zh-Hans');
  assert.equal(htmlLang('zh-TW'), 'zh-Hant');
  assert.equal(htmlLang('ja'), 'ja');
  assert.equal(htmlLang('en'), 'en');
});
