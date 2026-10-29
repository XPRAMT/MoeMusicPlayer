import assert from 'node:assert/strict';
import test from 'node:test';
import { computeHeaderOpticalOffsetPx } from '../src/lib/header-text-offset.js';

// Metrics measured in the app (13px Consolas header, dpr 2.5): the zh-Hant fallback gives a taller line box.
test('header offset centres the ink in the line box, snapped to device pixels', () => {
  // zh-Hant: box 18.8, baseline 15.2, ink 11 up / 1 down -> ink centre 10.2, box centre 9.4 -> -0.8 (2 device px)
  assert.equal(computeHeaderOpticalOffsetPx({ boxHeight: 18.8, baseline: 15.2, ascent: 11, descent: 1, devicePixelRatio: 2.5 }), -0.8);
  // en: box 15.2, baseline 12, ink 9 up / 0 down -> ink centre 7.5, box centre 7.6 -> +0.1 rounds to 0
  assert.equal(computeHeaderOpticalOffsetPx({ boxHeight: 15.2, baseline: 12, ascent: 9, descent: 0, devicePixelRatio: 2.5 }), 0);
  // ja: ink 12 up / 1 down -> ink centre 6.5 -> +1.1, snapped to 1.2 (3 device px)
  assert.equal(computeHeaderOpticalOffsetPx({ boxHeight: 15.2, baseline: 12, ascent: 12, descent: 1, devicePixelRatio: 2.5 }), 1.2);
  assert.equal(computeHeaderOpticalOffsetPx({ boxHeight: 15.2, baseline: 12, ascent: 12, descent: 1, devicePixelRatio: 1 }), 1);
});

test('missing metrics give no offset', () => {
  assert.equal(computeHeaderOpticalOffsetPx({ boxHeight: NaN, baseline: 12, ascent: 9, descent: 0, devicePixelRatio: 2 }), 0);
  assert.equal(computeHeaderOpticalOffsetPx({ boxHeight: 15, baseline: 12, ascent: 9, descent: 0, devicePixelRatio: 0 }), 0);
});
