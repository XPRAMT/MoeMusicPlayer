import assert from 'node:assert/strict';
import test from 'node:test';
import { fitMiddleEllipsis } from '../src/lib/middle-ellipsis.js';

const sample = '崩壞星穹鐵道-行於命途6 Experience the Paths Vol.6';

function widthOf(value) {
  let width = 0;
  for (const char of value) width += char.codePointAt(0) > 255 ? 2 : 1;
  return width;
}

test('short text stays intact', () => {
  assert.equal(fitMiddleEllipsis('專輯', 40, widthOf), '專輯');
  assert.equal(fitMiddleEllipsis('', 40, widthOf), '');
});

test('long mixed text keeps the head and the tail', () => {
  const fitted = fitMiddleEllipsis(sample, 24, widthOf);
  assert.match(fitted, /^崩壞星.+···.+Vol\.6$/);
  assert.ok(!fitted.includes('行於命途'));
  assert.ok(widthOf(fitted) <= 24);
});
