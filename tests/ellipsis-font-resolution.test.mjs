import assert from 'node:assert/strict';
import test from 'node:test';
import { canvasFontFromStyle, canvasFontKey, createEllipsisFitter, letterSpacingPx } from '../src/lib/ellipsis-fit.js';

// Chromium's computed \`font\` is '' when the longhands come from var(--app-font-family).
const interfaceStyle = {
  font: '', fontStyle: 'normal', fontVariant: 'normal', fontWeight: '400', fontStretch: '100%',
  fontSize: '13px', fontFamily: '"Noto Sans TC", "Segoe UI", sans-serif', letterSpacing: 'normal',
};

test('the canvas font is built from the longhands when the font shorthand is empty', () => {
  assert.equal(canvasFontFromStyle(interfaceStyle), 'normal normal 400 normal 13px "Noto Sans TC", "Segoe UI", sans-serif');
  assert.notEqual(canvasFontKey(interfaceStyle), '');
  assert.equal(canvasFontFromStyle({ font: '12px serif' }), '12px serif');
  assert.equal(letterSpacingPx(interfaceStyle), 0);
  assert.equal(letterSpacingPx({ letterSpacing: '0.5px' }), 0.5);
});

test('the fitter measures with that font and adds letter spacing; a different font is measured again', () => {
  const fonts = [];
  const context = {
    _font: '10px sans-serif',
    get font() { return this._font; },
    set font(value) { if (value) this._font = value; fonts.push(value); },
    measureText(value) { return { width: [...value].length * (this._font.includes('Noto') ? 8 : 5) }; },
  };
  const fitter = createEllipsisFitter({ getContext: () => context });
  const key = canvasFontKey(interfaceStyle);
  assert.equal(fitter.measureWidth(key, 'FLAC'), 32);
  assert.equal(fitter.measureWidth(canvasFontKey({ ...interfaceStyle, letterSpacing: '1px' }), 'FLAC'), 36);
  // The interface font setting changes: the new key re-measures instead of reusing the old width.
  const other = canvasFontKey({ ...interfaceStyle, fontFamily: 'Arial' });
  assert.equal(fitter.measureWidth(other, 'FLAC'), 20);
  assert.ok(fonts.every((font) => font !== ''), 'never assigns an empty font');
});
