import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { createActiveTrackArtworkController, nowPlayingArtworkObjectUrl } from '../src/lib/active-track-artwork.ts';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const png = () => { const b = new Uint8Array(16); b.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]); return b.buffer; };

test('the Now Playing cover keeps the previous image while the next one loads', () => {
  assert.equal(nowPlayingArtworkObjectUrl({ trackId: 'b', status: 'loading', objectUrl: 'blob:a' }), 'blob:a');
  assert.equal(nowPlayingArtworkObjectUrl({ trackId: 'b', status: 'loading', objectUrl: null }), null);
  assert.equal(nowPlayingArtworkObjectUrl({ trackId: 'b', status: 'ready', objectUrl: 'blob:b' }), 'blob:b');
  for (const status of ['missing', 'too-large', 'error', 'empty']) {
    assert.equal(nowPlayingArtworkObjectUrl({ trackId: 'b', status, objectUrl: null }), null, status);
  }
});

test('rapid track changes only ever show the latest track cover, after it has decoded', async () => {
  const pending = new Map();
  const shown = [];
  let urls = 0;
  const controller = createActiveTrackArtworkController({
    fetchBytes: (id) => new Promise((resolve) => pending.set(id, resolve)),
    createObjectUrl: async () => `blob:${++urls}`,
    revokeObjectUrl() {},
    onChange: (state) => shown.push(nowPlayingArtworkObjectUrl(state)),
  });
  controller.setTrack('a');
  pending.get('a')(png());
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(shown.at(-1), 'blob:1');
  controller.setTrack('b');
  assert.equal(shown.at(-1), 'blob:1', 'previous cover stays while b loads');
  controller.setTrack('c');
  assert.equal(shown.at(-1), 'blob:1', 'previous cover stays while c loads');
  pending.get('b')(png()); // stale result
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(shown.at(-1), 'blob:1', 'a stale result for b never replaces the cover');
  for (let i = 0; i < 20 && !pending.has('c'); i += 1) await new Promise((r) => setTimeout(r, 0));
  pending.get('c')(png());
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(controller.snapshot().trackId, 'c');
  assert.equal(shown.at(-1), controller.snapshot().objectUrl);
  assert.notEqual(shown.at(-1), 'blob:1');
  controller.dispose();
});

test('the Now Playing page uses the retained cover for its layout; the dock cover is unchanged', () => {
  const app = readFileSync(path.join(root, 'src/App.svelte'), 'utf8');
  assert.match(app, /artworkVisible=\{nowPlayingArtworkUrl !== null\}/);
  assert.match(app, /class:has-artwork=\{nowPlayingArtworkUrl !== null\}/);
  assert.match(app, /class="cover-stage-image" draggable="false" src=\{nowPlayingArtworkUrl\}/);
  assert.match(app, /\(activeArtwork\.status === 'ready' \|\| activeArtwork\.status === 'loading'\) && activeArtwork\.objectUrl\}\s*<img class="dock-art-image" src=\{activeArtwork\.objectUrl\}/);
  const decoder = readFileSync(path.join(root, 'src/lib/decoded-image-object-url.js'), 'utf8');
  assert.match(decoder, /await image\.decode\(\)/);
});
