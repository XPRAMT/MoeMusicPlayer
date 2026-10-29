import assert from 'node:assert/strict';
import test from 'node:test';
import { fitEndEllipsis, fitMiddleEllipsis } from '../src/lib/middle-ellipsis.js';
import {
  createEllipsisFitter,
  hostWidthFromResizeEntry,
  invalidateEllipsisFonts,
  onEllipsisFontsInvalidated,
} from '../src/lib/ellipsis-fit.js';

// Deterministic fake canvas: width depends on the font (its size) and on each code point.
function fakeContext(log) {
  return {
    font: '',
    measureText(value) {
      log.push([this.font, value]);
      const size = Number(/(\d+)px/.exec(this.font)?.[1] ?? 10);
      let width = 0;
      for (const char of value) width += (char.codePointAt(0) > 255 ? 1 : 0.5) * size + (char === 'i' ? -1 : 0);
      return { width };
    },
  };
}

const FONT = '400 14px sans-serif';
function referenceMeasure(font) {
  const context = fakeContext([]);
  context.font = font;
  return (value) => context.measureText(value).width;
}

const strings = [
  '', 'a', '專輯', 'Hello world', 'iiiiiiiiiiiiiiiiiiiiii', '崩壞星穹鐵道-行於命途6 Experience the Paths Vol.6',
  '崩壞星穹鐵道'.repeat(40), 'x'.repeat(500), 'A😀BC😀😀😀DEFGH', '...', '....', '日本語とEnglishの混在テキスト',
  '😀'.repeat(30), 'Mixed 漢字 and ascii 與 emoji 🎵 ending', ' ', '  leading and trailing  ',
];
const widths = [Number.NaN, -5, 0, 0.5, 1, 3, 4.2, 7, 10, 14, 21, 33, 50, 77.7, 120, 199, 400, 1000, 99999];

test('cached fitter output equals the reference algorithm for every string, width and mode', () => {
  const fitter = createEllipsisFitter({ getContext: () => fakeContext([]) });
  const measure = referenceMeasure(FONT);
  for (const mode of ['middle', 'end']) {
    const reference = mode === 'end' ? fitEndEllipsis : fitMiddleEllipsis;
    for (const text of strings) {
      for (const width of widths) {
        // twice: the second answer comes from the result cache
        for (let pass = 0; pass < 2; pass++) {
          assert.equal(
            fitter.fit(text, mode, FONT, width),
            reference(text, width, measure),
            `${mode} ${JSON.stringify(text)} @ ${width} pass ${pass}`,
          );
        }
      }
    }
  }
});

test('a text that fits is returned without a search and its width is measured once', () => {
  const log = [];
  const fitter = createEllipsisFitter({ getContext: () => fakeContext(log) });
  assert.equal(fitter.fit('Hello', 'middle', FONT, 500), 'Hello');
  assert.equal(log.length, 1);
  assert.deepEqual(log[0], [FONT, 'Hello']);
  // a different width reuses the cached text width
  assert.equal(fitter.fit('Hello', 'end', FONT, 600), 'Hello');
  assert.equal(log.length, 1);
});

test('result cache: same text, font, mode and width costs nothing; each key part separates entries', () => {
  const log = [];
  const stats = { measureText: 0, widthHits: 0, fitHits: 0, fitMisses: 0 };
  const fitter = createEllipsisFitter({ getContext: () => fakeContext(log), stats });
  const text = 'a long title that does not fit in the column';
  const first = fitter.fit(text, 'middle', FONT, 80);
  const measured = log.length;
  assert.ok(measured > 1);
  assert.equal(fitter.fit(text, 'middle', FONT, 80), first);
  assert.equal(log.length, measured);
  assert.equal(stats.fitHits, 1);
  assert.equal(stats.fitMisses, 1);
  // another width, mode or font are different keys and are computed on their own
  const otherWidth = fitter.fit(text, 'middle', FONT, 120);
  assert.notEqual(otherWidth, first);
  assert.ok(fitter.fit(text, 'end', FONT, 80).endsWith('...'));
  const bigger = '400 28px sans-serif';
  assert.equal(fitter.fit(text, 'middle', bigger, 80), fitMiddleEllipsis(text, 80, referenceMeasure(bigger)));
  assert.equal(stats.fitMisses, 4);
});

test('the shared context sets its font only when it changes', () => {
  const sets = [];
  const context = {
    _font: '',
    get font() { return this._font; },
    set font(value) { sets.push(value); this._font = value; },
    measureText: (value) => ({ width: value.length }),
  };
  const fitter = createEllipsisFitter({ getContext: () => context });
  fitter.measureWidth('a', 'one');
  fitter.measureWidth('a', 'two');
  fitter.measureWidth('b', 'one');
  fitter.measureWidth('a', 'three');
  assert.deepEqual(sets, ['a', 'b', 'a']);
});

test('caches are bounded and evict the least recently used entry', () => {
  const log = [];
  const fitter = createEllipsisFitter({ getContext: () => fakeContext(log), widthCacheSize: 3, fitCacheSize: 3 });
  for (const text of ['a', 'b', 'c']) fitter.measureWidth(FONT, text);
  fitter.measureWidth(FONT, 'a'); // refresh a: order is now b, c, a
  fitter.measureWidth(FONT, 'd'); // evicts b
  assert.equal(fitter.widths.size, 3);
  const before = log.length;
  fitter.measureWidth(FONT, 'a');
  fitter.measureWidth(FONT, 'c');
  assert.equal(log.length, before); // still cached
  fitter.measureWidth(FONT, 'b');
  assert.equal(log.length, before + 1); // evicted, measured again

  for (let i = 0; i < 20; i++) fitter.fit(`text ${i}`, 'middle', FONT, 100);
  assert.equal(fitter.fits.size, 3);
});

test('without a canvas context the width falls back to the string length like before', () => {
  const fitter = createEllipsisFitter({ getContext: () => null });
  const reference = (value) => value.length;
  for (const text of strings) {
    for (const width of widths) {
      assert.equal(fitter.fit(text, 'middle', FONT, width), fitMiddleEllipsis(text, width, reference));
    }
  }
});

test('the width of a host comes from the resize entry, rounded like clientWidth', () => {
  assert.equal(hostWidthFromResizeEntry({ contentBoxSize: [{ inlineSize: 183.6 }] }), 184);
  assert.equal(hostWidthFromResizeEntry({ contentBoxSize: { inlineSize: 100.2 } }), 100);
  assert.equal(hostWidthFromResizeEntry({ contentRect: { width: 57.4 } }), 57);
  assert.equal(hostWidthFromResizeEntry({}), 0);
});

test('font invalidation reaches every listener until it is removed', () => {
  let calls = 0;
  const stop = onEllipsisFontsInvalidated(() => { calls++; });
  invalidateEllipsisFonts();
  assert.equal(calls, 1);
  stop();
  invalidateEllipsisFonts();
  assert.equal(calls, 1);
});
