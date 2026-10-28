import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { cutToLines, createLinesHysteresis, linesAvailable, rawLinesAvailable, summaryCentered } from '../src/lib/album-artists-fit.js';

const source = readFileSync(new URL('../src/lib/AlbumDetail.svelte', import.meta.url), 'latin1');
const css = source.slice(source.indexOf('<style>'));

test('lines available: floor with the half pixel allowance, at least one', () => {
  assert.equal(linesAvailable(200, 40, 5, 18.8), 8);
  assert.equal(linesAvailable(100, 90, 5, 18.8), 1);
  assert.equal(linesAvailable(18.8 * 3 + 45 - 0.5, 40, 5, 18.8), 3);
  assert.ok(rawLinesAvailable(200, 40, 5, 18.8) > 8);
  assert.equal(linesAvailable(200, 40, 5, 0), 1);
});

test('hysteresis: first value at once, a change needs two consecutive runs, no flip-flop', () => {
  const h = createLinesHysteresis();
  assert.equal(h.next(4.5), 4);
  assert.equal(h.next(5.5), 4);
  assert.equal(h.next(5.5), 5);
  const alternating = createLinesHysteresis();
  alternating.next(4.5);
  for (let i = 0; i < 10; i += 1) assert.equal(alternating.next(i % 2 ? 4.5 : 5.5), 4);
  const wiggle = createLinesHysteresis();
  wiggle.next(5.01);
  for (let i = 0; i < 6; i += 1) assert.equal(wiggle.next(i % 2 ? 4.999 : 5.01), 5);
  h.reset();
  assert.equal(h.next(2.2), 2);
});

// 10 characters per line; the ellipsis counts as a character.
const measure = (text) => Math.max(1, Math.ceil(text.length / 10));

test('cut keeps the full text when it fits and restores it when space grows', () => {
  const full = 'あいうえおかきくけこさしすせそ';
  assert.equal(cutToLines(full, 2, measure), full);
  const cut = cutToLines(full, 1, measure);
  assert.notEqual(cut, full);
  assert.equal(cutToLines(full, 3, measure), full);
});

test('cut ends with an ellipsis, never exceeds the lines and strips trailing separators', () => {
  const full = 'AAAAAAAA．BBBBBBBB．CCCCCCCC．DDDDDDDD';
  for (let lines = 1; lines <= 3; lines += 1) {
    const cut = cutToLines(full, lines, measure);
    assert.ok(cut.endsWith('…'));
    assert.ok(measure(cut) <= lines);
    assert.ok(!/[．\s]…$/.test(cut));
  }
  assert.equal(cutToLines(full, 1, measure), 'AAAAAAAA…');
  assert.equal(cutToLines('AAAAAAAA．BBBBBBBB', 1, measure), 'AAAAAAAA…');
});

test('album detail artists: static source rules', () => {
  const artistsRule = css.match(/\.album-detail-artists[^{]*\{[^}]*\}/g).join('\n');
  assert.ok(!/mask-image/.test(css));
  assert.ok(!/line-clamps*:/.test(css));
  assert.match(artistsRule, /text-align:justify/);
  assert.match(artistsRule, /word-break:break-all/);
  assert.match(artistsRule, /overflow-wrap:anywhere/);
  assert.match(css, /\.album-detail-summary \{[^}]*gap:5px/);
  assert.match(css, /row-gap:5px/);
  assert.match(css, /column-gap:15px/);
  assert.match(css, /height:var\(--album-detail-summary-height\); max-height:var\(--album-detail-summary-height\)/);
  const script = source.slice(0, source.indexOf('<style>'));
  assert.equal((script.match(/requestAnimationFrame/g) ?? []).length, 1);
  assert.match(script, /write: \(progress\) => \{[^}]*fitArtists\(\)/);
});

test('summary is centred only while the artist list is shown whole, tied to the fit result', () => {
  const full = 'あいうえおかきくけこさしすせそ';
  assert.equal(summaryCentered(full, cutToLines(full, 3, measure)), true);
  assert.equal(summaryCentered(full, cutToLines(full, 1, measure)), false);
  assert.match(css, /\.album-detail-summary:global\(\[data-centered\]\) \{ justify-content:center; \}/);
  const script = source.slice(0, source.indexOf('<style>'));
  assert.match(script, /const shown = cutToLines\([^)]*\);[\s\S]*toggleAttribute\('data-centered', summaryCentered\(artistsFull, shown\)\)/);
  assert.equal((script.match(/requestAnimationFrame/g) ?? []).length, 1);
  assert.ok(!/addEventListener\('scroll'/.test(script));
});
