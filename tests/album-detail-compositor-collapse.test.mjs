import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  createCollapseProgressWriter,
  getAlbumDetailCollapseProgress,
  getAlbumDetailCollapseRange,
  getAlbumDetailScrollState,
} from '../src/lib/album-detail-collapse.js';

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const albumDetail = read('src/lib/AlbumDetail.svelte');
const albumCard = read('src/lib/AlbumCard.svelte');
const pagedList = read('src/lib/PagedVirtualList.svelte');

test('the collapse has no CSS scroll timelines and no keyframe animations (Android WebView drops fling frames with them)', () => {
  for (const [name, source] of [['AlbumDetail', albumDetail], ['PagedVirtualList', pagedList]]) {
    assert.doesNotMatch(source, /animation-timeline|scroll-timeline|timeline-scope|animation-range/, name);
    assert.doesNotMatch(source, /supportsScrollDrivenAlbumOverlay|scroll-driven-collapse|album-detail-scroll-timeline/, name);
    assert.doesNotMatch(source, /@keyframes album-detail-(cover|summary|grid-header)-collapse/, name);
  }
  assert.doesNotMatch(albumDetail, /transition:|animation:/);
});

test('album cover is laid out at its base size and only scaled from the top-left corner', () => {
  const art = albumDetail.match(/\n  \.album-detail-art \{([^}]*)\}/)?.[1] ?? '';
  assert.match(art, /width:var\(--album-detail-cover-base-size,200px\)/);
  assert.match(art, /height:var\(--album-detail-cover-base-size,200px\)/);
  assert.match(art, /transform-origin:0 0/);
  assert.match(art, /will-change:transform/);
  assert.match(art, /transform:scale\(/);
  assert.match(art, /border-radius:0/);
  assert.doesNotMatch(art, /--album-detail-cover-size/);
  assert.match(albumDetail, /\.album-detail-heading \{[^}]*height:var\(--album-detail-heading-base-height\)/);
  assert.doesNotMatch(albumDetail, /--album-detail-heading-height|--album-detail-cover-size|--album-detail-cover-min-size/);
});

test('album index cards and album detail cover use square corners', () => {
  const cardArt = albumCard.match(/\.album-card-art \{([^}]*)\}/)?.[1] ?? '';
  const detailArt = albumDetail.match(/\n  \.album-detail-art \{([^}]*)\}/)?.[1] ?? '';
  assert.match(cardArt, /border-radius:0/);
  assert.match(detailArt, /border-radius:0/);
  assert.doesNotMatch(albumDetail, /album-detail-cover-radius/);
});

test('cover, summary and column header all read the one progress variable', () => {
  const art = albumDetail.match(/\n  \.album-detail-art \{([^}]*)\}/)?.[1] ?? '';
  const summary = albumDetail.match(/\.album-detail-summary \{([^}]*)\}/)?.[1] ?? '';
  const header = pagedList.match(/\.paged-virtual-frame\.leading-overlay \.paged-virtual-header \{([^}]*)\}/)?.[1] ?? '';
  for (const block of [art, summary, header]) assert.match(block, /var\(--album-detail-collapse-progress, 0\)/);
  assert.match(art, /transform:scale\(calc\(1 - \(1 - var\(--album-detail-cover-end-scale, 1\)\) \* var\(--album-detail-collapse-progress, 0\)\)\)/);
  assert.match(summary, /transform:translateY\(calc\(var\(--album-detail-collapse-range, 0px\) \* var\(--album-detail-collapse-progress, 0\) \* -0\.5\)\)/);
  assert.match(summary, /margin-left:calc\(var\(--album-detail-collapse-range, 0px\) \* var\(--album-detail-collapse-progress, 0\) \* -1\)/);
  assert.match(header, /transform: translateY\(calc\(var\(--paged-collapse-range, 0px\) \* var\(--album-detail-collapse-progress, 0\) \* -1\)\)/);
  assert.match(header, /position: absolute/);
  // The list writes no transform on the header itself any more.
  assert.doesNotMatch(pagedList, /transform:translateY\(\$\{-collapseOffset\}px\)/);
});

test('the progress is written only through the frame-coalesced writer, and nothing else is written per scroll', () => {
  assert.equal(albumDetail.split("style.setProperty('--album-detail-collapse-progress'").length - 1, 1);
  assert.equal(albumDetail.split('requestAnimationFrame(').length - 1, 1);
  assert.match(albumDetail, /createCollapseProgressWriter\(\{/);
  assert.match(albumDetail, /onViewportScrollTopChange=\{handleListScrollTopChange\}/);
  assert.doesNotMatch(albumDetail, /class="album-detail-heading"\s+style=/);
  // shellStyle holds layout inputs only; the progress never goes through Svelte state.
  const shellStyle = albumDetail.match(/let shellStyle = \$derived\(([\s\S]*?)\n  \);/)?.[1] ?? '';
  assert.doesNotMatch(shellStyle, /collapse-progress/);
  assert.doesNotMatch(albumDetail, /albumCoverScrollPeak|albumCollapseProgress/);
});

test('the progress is right at mount, on restore and when the range or the leading overlay changes', () => {
  // The list reports immediately (not in a frame) after restoring a scroll position and after measuring.
  assert.match(pagedList, /onViewportScrollTopChange\?\.\(safeRestoredScrollTop, restoredCollapseOffset, true\)/);
  assert.match(pagedList, /onViewportScrollTopChange\?\.\(scrollTop, collapseOffset, true\)/);
  assert.match(pagedList, /onViewportScrollTopChange\?\.\(scrollTop, nextCollapseOffset, immediate\)/);
  assert.match(pagedList, /leadingCollapseRange;\s+leadingOverlayHeight;[\s\S]*?tick\(\)\.then\(\(\) => \{ if \(viewport\) updateViewport\(true\); \}\)/);
  // A new shell style string replaces the inline style, so the progress is recomputed and written again.
  assert.match(albumDetail, /\$effect\(\(\) => \{\s+shellStyle;[\s\S]*?, true\);\s+collapseProgressWriter\.reapply\(\);/);
});

test('progress pure function: clamping, mapping and restore', () => {
  const range = getAlbumDetailCollapseRange(200);
  assert.equal(range, 120);
  assert.equal(getAlbumDetailCollapseProgress(0, range), 0);
  assert.equal(getAlbumDetailCollapseProgress(60, range), 0.5);
  assert.equal(getAlbumDetailCollapseProgress(120, range), 1);
  assert.equal(getAlbumDetailCollapseProgress(-5, range), 0);
  assert.equal(getAlbumDetailCollapseProgress(500, range), 1);
  assert.equal(getAlbumDetailCollapseProgress(10, 0), 0);
  assert.equal(getAlbumDetailCollapseProgress(Number.NaN, range), 0);
  // Native offset -> collapse offset -> progress, including a restored position far below the collapse range.
  for (const [native, expected] of [[0, 0], [30, 0.25], [120, 1], [4000, 1]]) {
    const { collapseOffset } = getAlbumDetailScrollState(native, range);
    assert.equal(getAlbumDetailCollapseProgress(collapseOffset, range), expected);
  }
});

test('the writer writes at most once per frame while scrolling and at once when told to', () => {
  const frames = [];
  const writes = [];
  const writer = createCollapseProgressWriter({
    requestFrame: (callback) => { frames.push(callback); return frames.length; },
    cancelFrame: (handle) => { frames[handle - 1] = null; },
    write: (progress) => writes.push(progress),
  });
  writer.set(0.1);
  writer.set(0.2);
  writer.set(0.3);
  assert.equal(frames.length, 1, 'one frame requested for many scroll events');
  assert.deepEqual(writes, []);
  frames[0]();
  assert.deepEqual(writes, [0.3]);
  writer.set(0.3);
  frames[1]();
  assert.deepEqual(writes, [0.3], 'unchanged values are not written');
  // Mount / restore: immediate, and it supersedes a pending frame.
  writer.set(0.6);
  writer.set(1, true);
  assert.deepEqual(writes, [0.3, 1]);
  frames[2]?.();
  assert.deepEqual(writes, [0.3, 1]);
  // After the inline style was replaced the same value is written again.
  writer.reapply();
  assert.deepEqual(writes, [0.3, 1, 1]);
  writer.set(0.5);
  writer.dispose();
  assert.equal(frames.at(-1), null, 'pending frame cancelled on destroy');
});

test('the summary reflow is contained so it cannot touch the cover, the header or the track list', () => {
  assert.match(albumDetail, /\.album-detail-heading \{[^}]*contain:layout style size/);
  assert.match(albumDetail, /\.album-detail-summary \{[^}]*contain:layout style/);
  assert.match(albumDetail, /\.album-detail-summary \{[^}]*overflow:hidden/);
  assert.match(albumDetail, /\.album-detail-summary \{[^}]*--album-detail-summary-height:calc\(var\(--album-detail-heading-base-height\) - var\(--album-detail-collapse-range, 0px\) \* var\(--album-detail-collapse-progress, 0\)\)[^}]*max-height:var\(--album-detail-summary-height\)/);
});
