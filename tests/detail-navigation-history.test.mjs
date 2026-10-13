import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import test from 'node:test';
import {
  readDetailNavigationRoute,
  withDetailNavigationRoute,
} from '../src/lib/detail-navigation-history.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const appSource = readFileSync(path.join(root, 'src/App.svelte'), 'utf8').replace(/\r\n/g, '\n');

test('detail history metadata preserves host state and reads artist/album routes', () => {
  const artistRoute = { view: 'artist-detail', artistKey: 'artist:hanser', preserveIndex: true };
  const hostState = { hostValue: 42 };
  const artistState = withDetailNavigationRoute(hostState, artistRoute);
  const albumState = withDetailNavigationRoute(artistState, { view: 'album-detail', albumKey: 'album:1', preserveIndex: false });

  assert.equal(albumState.hostValue, 42);
  assert.deepEqual(readDetailNavigationRoute(artistState), artistRoute);
  assert.deepEqual(readDetailNavigationRoute(albumState), {
    view: 'album-detail',
    albumKey: 'album:1',
    preserveIndex: false,
  });
});

test('invalid and unrelated browser history states are ignored', () => {
  assert.equal(readDetailNavigationRoute(null), null);
  assert.equal(readDetailNavigationRoute('external state'), null);
  assert.equal(readDetailNavigationRoute({ __moeDetailNavigation: { view: 'artist-detail' } }), null);
  assert.equal(readDetailNavigationRoute({ __moeDetailNavigation: { view: 'unknown' } }), null);
});

test('artist and album detail transitions are represented in browser history and restored by popstate', () => {
  assert.match(appSource, /window\.addEventListener\('popstate', handleDetailHistoryPopState\)/);
  assert.match(appSource, /window\.removeEventListener\('popstate', handleDetailHistoryPopState\)/);
  assert.match(appSource, /window\.history\.replaceState\([\s\S]*?currentDetailNavigationRoute\(\)[\s\S]*?\);\s*window\.history\.pushState/);
  assert.match(appSource, /function handleDetailHistoryPopState\(event: PopStateEvent\)[\s\S]*?restoreDetailNavigationRoute\(route\)/);

  const artistOpen = appSource.slice(appSource.indexOf('function openArtistEntry('), appSource.indexOf('function navigateToArtistSection('));
  const albumOpen = appSource.slice(appSource.indexOf('function openAlbumEntry('), appSource.indexOf('function navigateToAlbumSection('));
  assert.match(artistOpen, /pushDetailNavigationRoute\(\{ view: 'artist-detail', artistKey: artist\.key, preserveIndex \}\)/);
  assert.match(albumOpen, /pushDetailNavigationRoute\(\{ view: 'album-detail', albumKey: album\.key, preserveIndex \}\)/);
  assert.match(appSource, /window\.history\.back\(\)/);
  assert.match(appSource, /route\.view === 'artist-detail' && route\.artistKey/);
  assert.match(appSource, /route\.view === 'album-detail' && route\.albumKey/);
});
