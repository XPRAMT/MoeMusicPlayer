import assert from 'node:assert/strict';
import test from 'node:test';
import { getAutomaticSearchVariants, getSearchVariants } from '../src/lib/search-variants.js';

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

test('automatic lyric search sends traditional before simplified', async () => {
  assert.deepEqual(
    await getAutomaticSearchVariants('苹果头发', '演出者'),
    ['蘋果頭髮 演出者', '苹果头发 演出者'],
  );
});

test('automatic search converts Chinese fields independently and keeps Japanese intact', async () => {
  assert.deepEqual(
    await getAutomaticSearchVariants('さよならスーヴェニア', '汉字歌手'),
    ['さよならスーヴェニア 漢字歌手', 'さよならスーヴェニア 汉字歌手'],
  );
});

test('automatic non-Chinese or Japanese-only queries are searched once', async () => {
  assert.deepEqual(await getAutomaticSearchVariants('M87 Black Hole', 'hanser'), ['M87 Black Hole hanser']);
  assert.deepEqual(await getAutomaticSearchVariants('東京と猫', ''), ['東京と猫']);
});
