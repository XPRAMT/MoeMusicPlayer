import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const app = await readFile(new URL('../src/App.svelte', import.meta.url), 'utf8');
const nav = app.slice(app.indexOf('<nav class="now-playing-track-info"'), app.indexOf('</nav>', app.indexOf('<nav class="now-playing-track-info"')));

test('the playback top bar shows nothing for a missing artist or album (no placeholder, no separator)', () => {
  // The artist and album blocks are only rendered when the field has a value, so their ::after
  // separator goes with them; the title block is always there (only the title when both are missing).
  assert.match(nav, /\{#if nowPlayingArtists\.length > 0\}[^]*?data-track-block="artist"[^]*?\{\/if\}/);
  assert.match(nav, /\{#if trackFieldValue\(playback\?\.currentTrack, 'album'\)\}\s*<span class="now-playing-track-block now-playing-album-block"[^]*?\{\/if\}/);
  const title = nav.indexOf('data-track-block="title"');
  assert.ok(title > nav.lastIndexOf('{/if}', title), 'the title block is not conditional');
  assert.doesNotMatch(nav, /沒有演出者分類資料|>—</);
});