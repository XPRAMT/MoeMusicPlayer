import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const albumDetail = read('src/lib/AlbumDetail.svelte');
const albumCard = read('src/lib/AlbumCard.svelte');
const pagedList = read('src/lib/PagedVirtualList.svelte');

/** Declaration names used inside `@keyframes <name> { ... }` (nested keyframe blocks included). */
function keyframeProperties(source, name) {
  const start = source.indexOf(`@keyframes ${name} {`);
  assert.notEqual(start, -1, `missing @keyframes ${name}`);
  let depth = 0;
  let end = start;
  for (let index = source.indexOf('{', start); index < source.length; index += 1) {
    if (source[index] === '{') depth += 1;
    if (source[index] === '}') depth -= 1;
    if (depth === 0) { end = index; break; }
  }
  const body = source.slice(source.indexOf('{', start) + 1, end);
  const props = new Set();
  for (const block of body.matchAll(/\{([^{}]*)\}/g)) {
    for (const declaration of block[1].split(';')) {
      const property = declaration.split(':')[0].trim();
      if (property) props.add(property);
    }
  }
  return props;
}

test('album cover and heading collapse animate only transform (compositor), never layout properties', () => {
  assert.deepEqual([...keyframeProperties(albumDetail, 'album-detail-cover-collapse')], ['transform']);
  // Exception (ADR 0012): the summary text box reflows into the room the cover frees; nothing else besides transform.
  assert.deepEqual([...keyframeProperties(albumDetail, 'album-detail-summary-collapse')].sort(), ['margin-left', 'max-height', 'transform']);
  assert.deepEqual([...keyframeProperties(pagedList, 'album-detail-grid-header-collapse')], ['transform']);
  assert.doesNotMatch(albumDetail, /album-detail-cover-radius/);
  assert.doesNotMatch(albumDetail, /@keyframes album-detail-heading-collapse/);
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

test('scroll events write no album heading styles when CSS scroll timelines are supported', () => {
  // The list only reports scroll positions to the album detail in the fallback path.
  assert.match(albumDetail, /onViewportScrollTopChange=\{supportsScrollDrivenAlbumOverlay \? undefined : handleListScrollTopChange\}/);
  assert.equal(albumDetail.split('handleListScrollTopChange').length - 1, 2, 'declared once, referenced once');
  // The progress variable exists only in the fallback inline style.
  const shellStyle = albumDetail.match(/let shellStyle = \$derived\(([\s\S]*?)\n  \);/)?.[1] ?? '';
  assert.match(shellStyle, /supportsScrollDrivenAlbumOverlay \? '' : `;--album-detail-collapse-progress:/);
  assert.doesNotMatch(albumDetail, /class="album-detail-heading"\s+style=/);
  assert.match(albumDetail, /<div class="album-detail-heading">/);
});

test('sortable header moves with transform, not top, and is scripted only in the fallback path', () => {
  assert.doesNotMatch(pagedList, /top:\$\{Math\.max\(0, leadingOverlayHeight - collapseOffset\)\}px/);
  assert.match(pagedList, /top:\$\{leadingOverlayHeight\}px;right:\$\{headerGutter\}px;\$\{supportsScrollDrivenAlbumOverlay \? '' : `transform:translateY\(\$\{-collapseOffset\}px\);`\}/);
  assert.match(pagedList, /\.paged-virtual-frame\.leading-overlay\.scroll-driven-collapse \.paged-virtual-header \{\s*animation: album-detail-grid-header-collapse/);
});

test('the summary reflow is contained so it cannot touch the cover, the header or the track list', () => {
  assert.match(albumDetail, /\.album-detail-heading \{[^}]*contain:layout style size/);
  assert.match(albumDetail, /\.album-detail-summary \{[^}]*contain:layout style/);
  assert.match(albumDetail, /\.album-detail-summary \{[^}]*overflow:hidden/);
  assert.match(albumDetail, /\.album-detail-summary \{[^}]*max-height:calc\(var\(--album-detail-heading-base-height\) - var\(--album-detail-collapse-range, 0px\) \* var\(--album-detail-collapse-progress, 0\)\)/);
});
