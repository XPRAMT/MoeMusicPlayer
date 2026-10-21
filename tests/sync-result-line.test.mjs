import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import './helpers/untr-fs.mjs';
import { formatSyncResultLine } from '../src/lib/sync-result-line.js';

test('a source line shows the added and removed counts and the errors only when there are some', () => {
  assert.equal(formatSyncResultLine('同步完成', 12, 3, 0), '同步完成；新增12個項目，移除3個項目。');
  assert.equal(formatSyncResultLine('同步完成', 0, 0, 0), '同步完成；新增0個項目，移除0個項目。');
  assert.equal(formatSyncResultLine('同步完成', 5, 1, 2), '同步完成；新增5個項目，移除1個項目，2個項目錯誤。');
  assert.equal(formatSyncResultLine('掃描未完成', 1_234, 0, 0), '掃描未完成；新增1,234個項目，移除0個項目。');
});

test('the counts are not known until the finished event: only the outcome shows', () => {
  assert.equal(formatSyncResultLine('同步完成', null, null, 0), '同步完成。');
  assert.equal(formatSyncResultLine('同步完成', null, null, 3), '同步完成；3個項目錯誤。');
});

test('the banner has no summary sentence for a normal run', () => {
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  assert.doesNotMatch(app, /formatProgressSummary/);
  assert.doesNotMatch(app, /個來源已檢查/);
});
