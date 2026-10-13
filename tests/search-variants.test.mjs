import assert from 'node:assert/strict';
import test from 'node:test';
import { getSearchVariants } from '../src/lib/search-variants.js';

test('traditional and simplified queries produce both distinct search forms', async () => {
  const traditional = await getSearchVariants('蘋果後臺');
  const simplified = await getSearchVariants('苹果后台');

  assert.deepEqual(traditional, ['蘋果後臺', '苹果后台']);
  assert.deepEqual(simplified, ['苹果后台', '蘋果後臺']);
});

test('Latin and Japanese queries are preserved without Chinese conversion', async () => {
  assert.deepEqual(await getSearchVariants('M87 Black Hole'), ['M87 Black Hole']);
  assert.deepEqual(await getSearchVariants('東京と猫'), ['東京と猫']);
});

test('empty input has no search terms', async () => {
  assert.deepEqual(await getSearchVariants('   '), []);
});
