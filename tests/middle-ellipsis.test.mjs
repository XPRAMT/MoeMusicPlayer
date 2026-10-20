import assert from 'node:assert/strict';
import test from 'node:test';
import { fitEndEllipsis, fitMiddleEllipsis } from '../src/lib/middle-ellipsis.js';

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
  assert.match(fitted, /^崩壞星.+....+Vol\.6$/);
  assert.ok(!fitted.includes('行於命途'));
  assert.ok(widthOf(fitted) <= 24);
});

test('end elision keeps the beginning and appends an ellipsis', () => {
  const fitted = fitEndEllipsis(sample, 24, widthOf);
  assert.ok(fitted.startsWith('崩壞星穹鐵道'));
  assert.ok(fitted.endsWith('...'));
  assert.ok(widthOf(fitted) <= 24);
});

test('end elision preserves complete Unicode code points', () => {
  const codePointWidth = (value) => value.endsWith('...')
    ? Array.from(value.slice(0, -3)).length + 1
    : Array.from(value).length;
  const fitted = fitEndEllipsis('A😀BC', 3, codePointWidth);
  assert.equal(fitted, 'A😀...');
});
