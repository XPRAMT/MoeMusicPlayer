import assert from 'node:assert/strict';
import test from 'node:test';
import { BROWSE_HOLD_MS, browseButtonIndex, browseCueAction, browseHoldAfterScroll, browseSeekPositionMs, findBrowseCentreRow } from '../src/lib/lyrics-browse.js';
import { buildLyricsLayout, buildTimedLyricTimeline } from '../src/lib/lyrics-window.js';

const preferences = { primaryFontSizePx: 20, auxiliaryScalePercent: 90, showTranslation: false, showRomanization: false, primaryLineHeight: 1, auxiliaryLineHeight: 1 };

function layoutOf(texts) {
  const timeline = buildTimedLyricTimeline(texts.map((text, index) => ({ text, startMs: index * 1000 })));
  return buildLyricsLayout(timeline, preferences, 0);
}

test('the centre row is the row under the viewport centre, the head spacer excluded', () => {
  const layout = layoutOf(['a', 'b', 'c', 'd']);
  const row = layout.heights[0];
  const edge = 200; // head spacer = half the viewport height
  // scrollTop 0: the centre sits on the first row's top edge.
  assert.equal(findBrowseCentreRow(layout, 0, 400, edge), 0);
  assert.equal(findBrowseCentreRow(layout, row * 1.5, 400, edge), 1);
  assert.equal(findBrowseCentreRow(layout, row * 2.5, 400, edge), 2);
  // Past the end: the last row.
  assert.equal(findBrowseCentreRow(layout, row * 100, 400, edge), 3);
});

test('empty rows are skipped and an empty layout has no centre row', () => {
  const layout = layoutOf(['a', '', 'c']);
  const row = layout.heights[0];
  assert.equal(findBrowseCentreRow(layout, row * 1.5, 400, 200), 2);
  assert.equal(findBrowseCentreRow(layoutOf([]), 0, 400, 200), -1);
});

test('the play button shows on the centre row only while browsing and never on the current line', () => {
  assert.equal(browseButtonIndex({ browsing: true, centreIndex: 5, currentIndex: 2 }), 5);
  assert.equal(browseButtonIndex({ browsing: true, centreIndex: 2, currentIndex: 2 }), -1);
  assert.equal(browseButtonIndex({ browsing: false, centreIndex: 5, currentIndex: 2 }), -1);
  assert.equal(browseButtonIndex({ browsing: true, centreIndex: -1, currentIndex: 2 }), -1);
  assert.equal(browseButtonIndex({ browsing: true, centreIndex: 0, currentIndex: -1 }), 0);
});

test('a line whose seek target is at or past the end of the track gets no play button', () => {
  const base = { browsing: true, centreIndex: 5, currentIndex: 2 };
  // The reported case: the last line starts at 3:49 in a 228 250 ms track.
  assert.equal(browseButtonIndex({ ...base, seekPositionMs: 229_000, durationMs: 228_250 }), -1);
  assert.equal(browseButtonIndex({ ...base, seekPositionMs: 228_250, durationMs: 228_250 }), -1, 'exactly the end');
  assert.equal(browseButtonIndex({ ...base, seekPositionMs: 228_249, durationMs: 228_250 }), 5);
  // Unknown or zero duration: not limited.
  assert.equal(browseButtonIndex({ ...base, seekPositionMs: 229_000, durationMs: null }), 5);
  assert.equal(browseButtonIndex({ ...base, seekPositionMs: 229_000, durationMs: 0 }), 5);
  assert.equal(browseButtonIndex({ ...base, seekPositionMs: 229_000, durationMs: Number.NaN }), 5);
  assert.equal(browseButtonIndex({ ...base, seekPositionMs: 229_000 }), 5);
});

test('a seek lands where the line becomes current: its start shifted by the display offset, never before it', () => {
  assert.equal(browseSeekPositionMs(161_000, 0), 161_000);
  // findActiveLyricIndex: current when position + offset >= start.
  assert.equal(browseSeekPositionMs(161_000, 500), 160_500);
  assert.equal(browseSeekPositionMs(161_000, -500), 161_500);
  assert.equal(browseSeekPositionMs(1000.4, 0), 1001, 'rounded up, so the line is already current');
  assert.equal(browseSeekPositionMs(200, 1000), 0);
  assert.equal(browseSeekPositionMs(Number.NaN, 0), 0);
});

test('the hold starts when the user is idle after a scroll and is not extended by later signals of it', () => {
  assert.equal(BROWSE_HOLD_MS, 3000);
  assert.equal(browseHoldAfterScroll(null, { userActive: true, now: 1000 }), null, 'still scrolling: no hold yet');
  assert.equal(browseHoldAfterScroll(null, { userActive: false, now: 1000 }), 4000);
  assert.equal(browseHoldAfterScroll(4000, { userActive: false, now: 1300 }), 4000, 'scrollend after the guard timer');
  assert.equal(browseHoldAfterScroll(4000, { userActive: true, now: 1300 }), null);
});

test('only the first line change after the hold returns (fake clock, the reported example)', () => {
  // A small simulation of LyricsView: scroll signals and line changes on one fake clock, no timers.
  const view = { browsing: true, holdUntil: null, returnedAt: null };
  const scroll = () => { view.holdUntil = null; };
  const scrollEnded = (now) => { view.holdUntil = browseHoldAfterScroll(view.holdUntil, { userActive: false, now }); };
  const lineChange = (now, options = {}) => {
    const action = browseCueAction({ browsing: view.browsing, cueChanged: true, userActive: false, holdUntil: view.holdUntil, now, ...options });
    if (action === 'return') { view.browsing = false; view.returnedAt = now; }
    return action;
  };
  scroll();
  scrollEnded(10_000); // scroll ends 0:10
  assert.equal(lineChange(11_000), 'keep', 'inside the hold: ignored');
  // The hold ends at 0:13 by itself: nothing happens (no timer, no return).
  assert.equal(view.browsing, true);
  assert.equal(lineChange(20_000), 'return', 'the next line after the hold pulls back');
  assert.equal(view.returnedAt, 20_000);
});

test('each new scroll restarts the hold', () => {
  let holdUntil = browseHoldAfterScroll(null, { userActive: false, now: 10_000 });
  holdUntil = null; // scrolled again at 0:12
  holdUntil = browseHoldAfterScroll(holdUntil, { userActive: false, now: 12_500 });
  assert.equal(browseCueAction({ browsing: true, cueChanged: true, userActive: false, holdUntil, now: 14_000 }), 'keep');
  assert.equal(browseCueAction({ browsing: true, cueChanged: true, userActive: false, holdUntil, now: 15_500 }), 'return');
});

test('line changes never return while the user scrolls or before the scroll has ended; seeks return at once', () => {
  const at = { browsing: true, cueChanged: true, now: 50_000 };
  assert.equal(browseCueAction({ ...at, userActive: true, holdUntil: 1000 }), 'keep');
  assert.equal(browseCueAction({ ...at, userActive: false, holdUntil: null }), 'keep', 'scroll not ended yet');
  assert.equal(browseCueAction({ ...at, cueChanged: false, userActive: false, holdUntil: 1000 }), 'keep');
  assert.equal(browseCueAction({ ...at, jumped: true, userActive: false, holdUntil: 60_000 }), 'return', 'a seek or track change');
  assert.equal(browseCueAction({ ...at, browsing: false, userActive: false, holdUntil: null }), 'none');
});
