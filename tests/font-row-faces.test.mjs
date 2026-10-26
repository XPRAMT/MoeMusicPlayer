import assert from 'node:assert/strict';
import test from 'node:test';
import { createFontRowFaces, fontRowFamilyCss } from '../src/lib/font-row-faces.js';

const STACK = "'Segoe UI', sans-serif";
const toUrl = (path) => `http://asset.localhost/${encodeURIComponent(path)}`;

function faces() {
  const added = [];
  const rows = createFontRowFaces({ toUrl, addFace: (name, source) => added.push([name, source]) });
  return { rows, added };
}

test('a row with a subset uses its subset face at once: registered synchronously on its first render', () => {
  const { rows, added } = faces();
  const font = { family: 'Microsoft JhengHei', subset: 'D:/s/ab.ttf' };
  assert.equal(fontRowFamilyCss(font, rows.faceFor, STACK), `"MoeFontRow 1", ${STACK}`);
  assert.deepEqual(added, [['MoeFontRow 1', 'url("http://asset.localhost/D%3A%2Fs%2Fab.ttf")']]);
});

test('a subset is registered once, whichever row, picker or refreshed list draws it', () => {
  const { rows, added } = faces();
  const first = { family: 'A', subset: 'D:/s/a.ttf' };
  const refreshed = { family: 'A', subset: 'D:/s/a.ttf', displayName: 'A' };
  fontRowFamilyCss(first, rows.faceFor, STACK);
  assert.equal(fontRowFamilyCss(refreshed, rows.faceFor, STACK), `"MoeFontRow 1", ${STACK}`);
  assert.equal(fontRowFamilyCss({ family: 'B', subset: 'D:/s/b.ttf' }, rows.faceFor, STACK), `"MoeFontRow 2", ${STACK}`);
  assert.equal(added.length, 2);
  assert.equal(rows.size, 2);
});

test('a row without a subset (legacy-encoding or not made yet) draws in the installed family', () => {
  const { rows, added } = faces();
  assert.equal(fontRowFamilyCss({ family: '文鼎新藝體' }, rows.faceFor, STACK), `"文鼎新藝體", ${STACK}`);
  assert.equal(fontRowFamilyCss({ family: 'Say "Hi"' }, rows.faceFor, STACK), `"Say \\"Hi\\"", ${STACK}`);
  assert.equal(fontRowFamilyCss(null, rows.faceFor, STACK), STACK);
  assert.equal(added.length, 0);
});
