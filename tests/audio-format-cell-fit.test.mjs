import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { createEllipsisFitter } from '../src/lib/ellipsis-fit.js';

// The audio format cell is an inline-flex box (shrink-to-fit, max-width 100%) holding the MiddleEllipsis host.
// Its host width is min(room, preferred width). Before the fix the preferred width was the width of the text the
// host currently showed, so a cell shortened while the column was 163 px stayed shortened after the column grew.
const charPx = 7;
const fitter = createEllipsisFitter({ getContext: () => ({ font: '', measureText: (value) => ({ width: [...value].length * charPx }) }) });
const text = 'FLAC．44.1 kHz．16 bit';

/** Run layout passes like ResizeObserver + MiddleEllipsis until the shown text settles. */
function settle(roomSequence, contentSized) {
  let shown = text;
  for (const room of roomSequence) {
    for (let pass = 0; pass < 5; pass += 1) {
      const preferred = contentSized ? Math.ceil(fitter.measureWidth('', text)) : fitter.measureWidth('', shown);
      const hostWidth = Math.round(Math.min(room, preferred));
      shown = fitter.fit(text, 'end', '', hostWidth);
    }
  }
  return shown;
}

test('a shortened audio format cell shows the full text again once the column is wide enough', () => {
  const full = [...text].length * charPx;
  const narrow = full - 20;
  const wide = full + 30;
  // The old layout reproduces the bug: shortened at the narrow width and stuck there.
  assert.notEqual(settle([narrow, wide], false), text);
  // Content-sized host: preferred width is the full text, so the wider room shows everything.
  assert.notEqual(settle([narrow], true), text);
  assert.equal(settle([narrow, wide], true), text);
});

test('the audio format cell sizes its ellipsis host from the full text, not the shown text', async () => {
  const cells = await readFile(new URL('../src/lib/TrackColumnCells.svelte', import.meta.url), 'utf8');
  assert.match(cells, /<MiddleEllipsis text=\{cell\.text\} mode=\{\$textOverflowStore\} contentSized=\{true\} \/>\s*\{#if cell\.hiRes\}/);
  const host = await readFile(new URL('../src/lib/MiddleEllipsis.svelte', import.meta.url), 'utf8');
  assert.ok(host.includes('sharedFitter.measureWidth(font, text)'));
  assert.ok(host.includes('style:width='));
});
