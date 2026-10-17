import assert from 'node:assert/strict';
import test from 'node:test';
import {
  formatMessage,
  getActiveLanguage,
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
  assert.equal(resolveSystemLanguage(['ja-JP', 'zh-CN']), 'zh-CN', 'the first Chinese tag decides');
  assert.equal(resolveSystemLanguage(['en-US']), 'zh-TW');
  assert.equal(resolveSystemLanguage(undefined), 'zh-TW');
});

test('an explicit choice beats the system language and bad choices fall back to system', () => {
  assert.equal(resolveLanguage('zh-TW', ['zh-CN']), 'zh-TW');
  assert.equal(resolveLanguage('zh-CN', ['zh-TW']), 'zh-CN');
  assert.equal(resolveLanguage('system', ['zh-CN']), 'zh-CN');
  assert.equal(normalizeLanguageChoice('fr'), 'system');
  assert.equal(normalizeLanguageChoice('zh-CN'), 'zh-CN');
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
  setLanguageForTesting('zh-TW');
});
