import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { pointerFocusReleaseKind } from '../src/lib/pointer-focus.js';

test('pointer presses release focus from controls but not from text fields', () => {
  assert.equal(pointerFocusReleaseKind({ tagName: 'BUTTON' }), 'blur');
  assert.equal(pointerFocusReleaseKind({ tagName: 'INPUT', type: 'range' }), 'blur');
  assert.equal(pointerFocusReleaseKind({ tagName: 'INPUT', type: 'checkbox' }), 'blur');
  assert.equal(pointerFocusReleaseKind({ tagName: 'INPUT', type: 'radio' }), 'blur');
  assert.equal(pointerFocusReleaseKind({ tagName: 'SELECT' }), 'after-change');
  for (const type of ['text', 'search', 'number', 'password', '']) {
    assert.equal(pointerFocusReleaseKind({ tagName: 'INPUT', type }), 'keep', type);
  }
  assert.equal(pointerFocusReleaseKind({ tagName: 'TEXTAREA' }), 'keep');
  assert.equal(pointerFocusReleaseKind({ tagName: 'DIV', isContentEditable: true }), 'keep');
});

test('the release is installed once, centrally, and only follows pointer presses', async () => {
  const main = await readFile(new URL('../src/main.ts', import.meta.url), 'utf8');
  const src = await readFile(new URL('../src/lib/pointer-focus.js', import.meta.url), 'utf8');
  const css = await readFile(new URL('../src/app.css', import.meta.url), 'utf8');
  assert.match(main, /installPointerFocusRelease\(document\);/);
  assert.match(src, /addEventListener\('pointerdown', onDown, true\)/);
  assert.match(src, /addEventListener\('pointerup', onUp, true\)/);
  assert.doesNotMatch(src, /'keydown'|'focusin'/); // keyboard focus is never touched
  assert.match(src, /if \(document\.activeElement !== control\) return;/); // focus moved by a handler stays
  assert.match(src, /dialog\[open\], \[aria-modal="true"\]/); // modals keep focus inside
  assert.match(src, /setTimeout\(\(\) => releaseFocus\(control\), 0\)/); // after drag end and click handlers
  assert.match(css, /:focus:not\(:focus-visible\)/); // mouse focus shows no ring; Tab still does
});
