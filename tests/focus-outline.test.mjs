import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';

const ring = String.raw`outline:\s*1px solid color-mix\(in srgb, var\(--accent\) 70%, transparent\);`;

test('keyboard focus defaults to a 1px accent ring at 70% opacity with zero specificity', async () => {
  const css = await readFile(new URL('../src/app.css', import.meta.url), 'utf8');
  assert.match(css, new RegExp(String.raw`:where\(:focus-visible\)\s*\{\s*${ring}\s*\}`));
  // Buttons and inputs keep their own 2px ring.
  assert.match(css, /button:focus-visible,\s*input:focus-visible\s*\{\s*outline: 2px solid var\(--accent-text\);\s*outline-offset: 3px;/);
  // Mouse/touch focus still shows nothing.
  assert.match(css, /:focus:not\(:focus-visible\)\s*\{\s*outline: none;/);
});

test('list rows draw the same ring inside the row so the viewport never clips it', async () => {
  const css = await readFile(new URL('../src/app.css', import.meta.url), 'utf8');
  assert.match(css, new RegExp(String.raw`\.paged-virtual-row:focus-visible\s*\{\s*${ring}\s*outline-offset: -1px;\s*\}`));
});
