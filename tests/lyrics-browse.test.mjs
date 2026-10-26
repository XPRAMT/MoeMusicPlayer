import assert from 'node:assert/strict';
import test from 'node:test';
import { browseButtonIndex, browseReturnAction, browseSeekPositionMs, findBrowseCentreRow } from '../src/lib/lyrics-browse.js';
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

test('a seek lands where the line becomes current: its start shifted by the display offset, never before it', () => {
  assert.equal(browseSeekPositionMs(161_000, 0), 161_000);
  // findActiveLyricIndex: current when position + offset >= start.
  assert.equal(browseSeekPositionMs(161_000, 500), 160_500);
  assert.equal(browseSeekPositionMs(161_000, -500), 161_500);
  assert.equal(browseSeekPositionMs(1000.4, 0), 1001, 'rounded up, so the line is already current');
  assert.equal(browseSeekPositionMs(200, 1000), 0);
  assert.equal(browseSeekPositionMs(Number.NaN, 0), 0);
});

test('browsing ends on a line change, or after the scroll that was still running at that moment', () => {
  assert.equal(browseReturnAction({ browsing: false, cueChanged: true, pendingReturn: false, userActive: false }), 'none');
  assert.equal(browseReturnAction({ browsing: true, cueChanged: false, pendingReturn: false, userActive: false }), 'keep');
  assert.equal(browseReturnAction({ browsing: true, cueChanged: true, pendingReturn: false, userActive: false }), 'return');
  assert.equal(browseReturnAction({ browsing: true, cueChanged: true, pendingReturn: false, userActive: true }), 'defer');
  assert.equal(browseReturnAction({ browsing: true, cueChanged: false, pendingReturn: true, userActive: true }), 'defer');
  assert.equal(browseReturnAction({ browsing: true, cueChanged: false, pendingReturn: true, userActive: false }), 'return');
});
