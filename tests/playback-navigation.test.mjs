import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import test from 'node:test';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const appSource = readFileSync(path.join(root, 'src/App.svelte'), 'utf8');

function functionSource(startText, endText) {
  const start = appSource.indexOf(startText);
  assert.notEqual(start, -1, `missing ${startText}`);
  const end = appSource.indexOf(endText, start + startText.length);
  assert.notEqual(end, -1, `missing boundary ${endText}`);
  return appSource.slice(start, end);
}

test('playing a library or playlist item does not change the current page', () => {
  const libraryPlay = functionSource('async function playTrack(', 'function openNowPlaying(');
  const playlistPlay = functionSource('async function playPlaylistEntry(', 'async function pickWindowsFolder(');

  assert.match(libraryPlay, /invokeCommand\('playback_play'/);
  assert.match(playlistPlay, /invokeCommand\('playback_play'/);
  assert.doesNotMatch(libraryPlay, /activeView\s*=/);
  assert.doesNotMatch(playlistPlay, /activeView\s*=/);
});

test('the footer artwork is the only Now Playing navigation control', () => {
  const navStart = appSource.indexOf('<nav class="primary-nav"');
  assert.notEqual(navStart, -1, 'missing primary navigation');
  const navEnd = appSource.indexOf('</nav>', navStart);
  assert.notEqual(navEnd, -1, 'primary navigation must close');
  const primaryNav = appSource.slice(navStart, navEnd);
  assert.doesNotMatch(primaryNav, /now-playing|正在播放/);
  assert.equal([...appSource.matchAll(/activeView\s*=\s*'now-playing'/g)].length, 1);

  const artClass = appSource.indexOf('class="dock-art"');
  assert.notEqual(artClass, -1, 'missing footer artwork control');
  const buttonStart = appSource.lastIndexOf('<button', artClass);
  const buttonEnd = appSource.indexOf('</button>', artClass);
  assert.ok(buttonStart >= 0 && buttonEnd > artClass, 'footer artwork must be a button');
  const artworkButton = appSource.slice(buttonStart, buttonEnd + '</button>'.length);
  assert.match(artworkButton, /onclick=\{openNowPlaying\}/);
  assert.match(artworkButton, /disabled=\{!playback\?\.currentTrack\}/);
  assert.match(artworkButton, /aria-label=/);

  const navigationHandler = functionSource('function openNowPlaying(', 'async function togglePlayback(');
  assert.match(navigationHandler, /activeView\s*=\s*'now-playing'/);
});
