import assert from 'node:assert/strict';
import test from 'node:test';
import { adoptSavedLanguage, changeLanguage, followLanguageChange } from '../src/lib/language-sync.js';
import { getLanguageChoice, setLanguageForTesting } from '../src/lib/i18n.js';

function reloadSpy() {
  const spy = { count: 0, reload() { spy.count += 1; } };
  return spy;
}

test('adopting the saved choice reloads only when it differs from the loaded one', () => {
  setLanguageForTesting('system');
  const target = reloadSpy();
  assert.equal(adoptSavedLanguage('system', target), false);
  assert.equal(adoptSavedLanguage('garbage', target), false, 'a bad value is the system choice');
  assert.equal(target.count, 0);
  assert.equal(adoptSavedLanguage('zh-CN', target), true);
  assert.equal(target.count, 1);
});

test('changing the language saves first, then reloads; the same choice does nothing', async () => {
  setLanguageForTesting('zh-TW');
  const target = reloadSpy();
  const saved = [];
  await changeLanguage('zh-TW', async (choice) => saved.push(choice), target);
  assert.deepEqual(saved, []);
  assert.equal(target.count, 0);
  await changeLanguage('zh-CN', async (choice) => saved.push(choice), target);
  assert.deepEqual(saved, ['zh-CN']);
  assert.equal(target.count, 1);
  await assert.rejects(changeLanguage('system', async () => { throw new Error('disk full'); }, target));
  assert.equal(target.count, 1, 'a failed save must not reload');
});

test('a window follows a change made elsewhere', () => {
  setLanguageForTesting('zh-TW');
  const target = reloadSpy();
  followLanguageChange('zh-TW', target);
  assert.equal(target.count, 0);
  followLanguageChange('zh-CN', target);
  assert.equal(target.count, 1);
  assert.equal(getLanguageChoice(), 'zh-TW', 'the running page keeps its language until it reloads');
});
