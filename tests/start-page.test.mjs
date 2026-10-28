import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import {
  DEFAULT_START_PAGE,
  START_PAGE_SAVE_DELAY_MS,
  START_PAGE_SETTINGS_SECTIONS,
  START_PAGE_VIEWS,
  createStartPageSaver,
  lateRestoreStillWanted,
  normalizeStartPage,
  nowPlayingRestoreAction,
  startTimingMark,
  planStartRestore,
  resolveStartPage,
  startPageFromState,
  startPageKind,
  startPagesEqual,
} from '../src/lib/start-page.js';

const rustSource = readFileSync(new URL('../src-tauri/src/settings.rs', import.meta.url), 'utf8');

function rustList(name) {
  const match = rustSource.match(new RegExp(`pub const ${name}: \\[&str; \\d+\\] = \\[([^\\]]*)\\]`));
  assert.ok(match, `${name} is declared in settings.rs`);
  return [...match[1].matchAll(/"([^"]+)"/g)].map((item) => item[1]);
}

test('the page lists and the default page match the Rust settings', () => {
  assert.deepEqual([...START_PAGE_VIEWS], rustList('LAST_PAGE_VIEWS'));
  assert.deepEqual([...START_PAGE_SETTINGS_SECTIONS], rustList('LAST_PAGE_SETTINGS_SECTIONS'));
  const defaults = rustSource.match(/impl Default for LastPageSettings \{[\s\S]*?\n\}\n/);
  assert.ok(defaults);
  assert.match(defaults[0], /view: "library"\.to_owned\(\)/);
  for (const field of ['playlist_id', 'settings_section', 'artist_key', 'album_key']) {
    assert.match(defaults[0], new RegExp(`${field}: None`));
  }
  assert.match(defaults[0], /now_playing: false/);
  assert.deepEqual(DEFAULT_START_PAGE, {
    view: 'library',
    playlistId: null,
    settingsSection: null,
    artistKey: null,
    albumKey: null,
    nowPlaying: false,
  });
});

test('every view the app can show is a restorable view id, and the settings tabs too', () => {
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  const view = app.match(/type View = ([^;]+);/)[1];
  assert.deepEqual([...view.matchAll(/'([^']+)'/g)].map((item) => item[1]), [...START_PAGE_VIEWS]);
  const sections = app.match(/type SettingsSection = ([^;]+);/)[1];
  assert.deepEqual([...sections.matchAll(/'([^']+)'/g)].map((item) => item[1]), [...START_PAGE_SETTINGS_SECTIONS]);
});

test('a stored page round-trips through the normalizer', () => {
  const pages = [
    { view: 'library' },
    { view: 'artists' },
    { view: 'albums' },
    { view: 'queue', nowPlaying: true },
    { view: 'playlists', playlistId: 'playlist-1' },
    { view: 'artist-detail', artistKey: 'artist:key' },
    { view: 'album-detail', albumKey: 'album:key', nowPlaying: true },
    { view: 'settings', settingsSection: 'lyrics' },
  ];
  for (const page of pages) {
    const normalized = normalizeStartPage(page);
    assert.equal(normalized.view, page.view);
    assert.deepEqual(normalizeStartPage(JSON.parse(JSON.stringify(normalized))), normalized);
  }
  assert.equal(normalizeStartPage({ view: 'settings', settingsSection: 'lyrics' }).settingsSection, 'lyrics');
});

test('missing, damaged, unknown or half-filled values are the default page', () => {
  for (const raw of [undefined, null, 5, 'library', [], {}, { view: 7 }, { view: 'from-the-future', nowPlaying: true }]) {
    assert.deepEqual(normalizeStartPage(raw), DEFAULT_START_PAGE);
  }
  assert.deepEqual(normalizeStartPage({ view: 'playlists' }), DEFAULT_START_PAGE);
  assert.deepEqual(normalizeStartPage({ view: 'playlists', playlistId: '' }), DEFAULT_START_PAGE);
  assert.deepEqual(normalizeStartPage({ view: 'album-detail', albumKey: 3 }), DEFAULT_START_PAGE);
  assert.deepEqual(normalizeStartPage({ view: 'artist-detail', artistKey: 'x'.repeat(5000) }), DEFAULT_START_PAGE);
  assert.equal(normalizeStartPage({ view: 'settings', settingsSection: 'nope' }).settingsSection, null);
  // Only what the view needs is kept.
  const queue = normalizeStartPage({ view: 'queue', playlistId: 'p', albumKey: 'a', nowPlaying: 'yes' });
  assert.deepEqual(queue, { ...DEFAULT_START_PAGE, view: 'queue' });
});

test('the state of the app becomes the stored page', () => {
  assert.deepEqual(
    startPageFromState({ activeView: 'playlists', selectedPlaylistId: 'p1', settingsSection: 'lyrics', albumKey: 'a' }),
    { ...DEFAULT_START_PAGE, view: 'playlists', playlistId: 'p1' },
  );
  assert.deepEqual(
    startPageFromState({ activeView: 'settings', settingsSection: 'sources', nowPlayingOpen: true }),
    { ...DEFAULT_START_PAGE, view: 'settings', settingsSection: 'sources', nowPlaying: true },
  );
  // A detail view without its entry (still loading) is the default page rather than a broken value.
  assert.deepEqual(startPageFromState({ activeView: 'album-detail', albumKey: null }), DEFAULT_START_PAGE);
  assert.ok(startPagesEqual(startPageFromState({ activeView: 'library' }), DEFAULT_START_PAGE));
});

test('an explicit launch intent wins over the remembered page', () => {
  const stored = { view: 'albums', nowPlaying: true };
  const normal = planStartRestore(stored, { externalLaunch: false });
  assert.equal(normal.restore, true);
  assert.equal(normal.page.view, 'albums');
  const external = planStartRestore(stored, { externalLaunch: true });
  assert.equal(external.restore, false);
  // Nothing remembered, or the default page remembered: nothing to restore.
  assert.equal(planStartRestore(undefined).restore, false);
  assert.equal(planStartRestore({ view: 'library' }).restore, false);
  assert.equal(planStartRestore({ view: 'library', nowPlaying: true }).restore, true);
});

test('pages with a target wait for it; a missing target is the default page', () => {
  assert.equal(startPageKind(normalizeStartPage({ view: 'queue' })), 'direct');
  const playlist = normalizeStartPage({ view: 'playlists', playlistId: 'p1', nowPlaying: true });
  const artist = normalizeStartPage({ view: 'artist-detail', artistKey: 'a' });
  const album = normalizeStartPage({ view: 'album-detail', albumKey: 'b' });
  assert.deepEqual([playlist, artist, album].map(startPageKind), ['playlist', 'artist', 'album']);

  assert.equal(resolveStartPage(playlist, { playlistIds: ['p0', 'p1'] }), playlist);
  assert.deepEqual(resolveStartPage(playlist, { playlistIds: ['p0'] }), { ...DEFAULT_START_PAGE, nowPlaying: true });
  assert.deepEqual(resolveStartPage(playlist, {}), { ...DEFAULT_START_PAGE, nowPlaying: true });
  assert.equal(resolveStartPage(artist, { artistFound: true }), artist);
  assert.deepEqual(resolveStartPage(artist, { artistFound: false }), DEFAULT_START_PAGE);
  assert.equal(resolveStartPage(album, { albumFound: true }), album);
  assert.deepEqual(resolveStartPage(album, {}), DEFAULT_START_PAGE);
  const queue = normalizeStartPage({ view: 'queue' });
  assert.equal(resolveStartPage(queue, {}), queue);
});

test('a late restore does not undo the user navigating meanwhile', () => {
  assert.equal(lateRestoreStillWanted('library', 'library'), true);
  assert.equal(lateRestoreStillWanted('queue', 'library'), false);
});

test('the Now Playing page is open at once; it then completes, is cancelled by the user, or gives up', () => {
  assert.equal(nowPlayingRestoreAction({ hasCurrentTrack: false, timedOut: false }), 'wait');
  assert.equal(nowPlayingRestoreAction({ hasCurrentTrack: true, timedOut: false }), 'open');
  assert.equal(nowPlayingRestoreAction({ hasCurrentTrack: true, timedOut: true }), 'open');
  assert.equal(nowPlayingRestoreAction({ hasCurrentTrack: false, timedOut: true }), 'drop');
  // The user closed the page while it waited: nothing is closed again, nothing is reopened.
  assert.equal(nowPlayingRestoreAction({ hasCurrentTrack: false, timedOut: true, stillOpen: false }), 'cancel');
  assert.equal(nowPlayingRestoreAction({ hasCurrentTrack: true, timedOut: false, stillOpen: false }), 'cancel');
});

test('a missing page target keeps the Now Playing flag (the page opens over the library)', () => {
  const album = normalizeStartPage({ view: 'album-detail', albumKey: 'b', nowPlaying: true });
  assert.deepEqual(resolveStartPage(album, {}), { ...DEFAULT_START_PAGE, nowPlaying: true });
});

test('the Now Playing restore does not wait for the playback snapshot', () => {
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  const start = app.slice(app.indexOf('function startNowPlayingRestore'));
  const body = start.slice(0, start.indexOf('\n  }\n'));
  assert.match(body, /isNowPlayingOpen = true/);
  assert.doesNotMatch(body, /currentTrack/);
});

test('start timing marks print only when switched on', () => {
  const lines = [];
  const info = console.info;
  console.info = (line) => lines.push(line);
  try {
    startTimingMark('off');
    globalThis.localStorage = { getItem: (name) => (name === 'moe.startTiming' ? '1' : null) };
    startTimingMark('on');
  } finally {
    console.info = info;
    delete globalThis.localStorage;
  }
  assert.equal(lines.length, 1);
  assert.match(lines[0], /\[start-timing\] on @ \d+ ms/);
});

function fakeClock() {
  const timers = new Map();
  let next = 1;
  return {
    setTimer(callback, delay) {
      const id = next++;
      timers.set(id, { callback, delay });
      return id;
    },
    clearTimer(id) {
      timers.delete(id);
    },
    fireAll() {
      for (const [id, timer] of [...timers]) {
        timers.delete(id);
        timer.callback();
      }
    },
    count: () => timers.size,
    delays: () => [...timers.values()].map((timer) => timer.delay),
  };
}

test('saving is debounced: only the latest page is written after the quiet time', () => {
  const clock = fakeClock();
  const written = [];
  const saver = createStartPageSaver({ delayMs: START_PAGE_SAVE_DELAY_MS, save: (page) => written.push(page), ...clock });
  const albums = normalizeStartPage({ view: 'albums' });
  const queue = normalizeStartPage({ view: 'queue' });
  saver.schedule(albums);
  saver.schedule(queue);
  assert.equal(clock.count(), 1, 'one timer, restarted by each change');
  assert.deepEqual(clock.delays(), [START_PAGE_SAVE_DELAY_MS]);
  assert.deepEqual(written, []);
  clock.fireAll();
  assert.deepEqual(written, [queue]);
  // Writing the same page again does nothing.
  saver.schedule(queue);
  assert.equal(clock.count(), 0);
});

test('going back to the stored page before the timer fires writes nothing', () => {
  const clock = fakeClock();
  const written = [];
  const saver = createStartPageSaver({ delayMs: 400, save: (page) => written.push(page), ...clock });
  saver.schedule(normalizeStartPage({ view: 'albums' }));
  saver.schedule(DEFAULT_START_PAGE);
  clock.fireAll();
  assert.deepEqual(written, []);
});

test('flush writes a pending page at once (page hidden, closing) and cancels the timer', () => {
  const clock = fakeClock();
  const written = [];
  const saver = createStartPageSaver({ delayMs: 400, save: (page) => written.push(page), ...clock });
  assert.equal(saver.flush(), false, 'nothing pending');
  const settings = normalizeStartPage({ view: 'settings', settingsSection: 'about' });
  saver.schedule(settings);
  assert.equal(saver.hasPending(), true);
  assert.equal(saver.flush(), true);
  assert.deepEqual(written, [settings]);
  assert.equal(clock.count(), 0);
  assert.equal(saver.hasPending(), false);
  assert.equal(saver.flush(), false);
});

test('the restored page is the baseline: it is not written back', () => {
  const clock = fakeClock();
  const written = [];
  const restored = normalizeStartPage({ view: 'album-detail', albumKey: 'k' });
  const saver = createStartPageSaver({ delayMs: 400, save: (page) => written.push(page), baseline: restored, ...clock });
  saver.schedule(restored);
  assert.equal(clock.count(), 0);
  saver.setBaseline(DEFAULT_START_PAGE);
  saver.schedule(restored);
  assert.equal(clock.count(), 1);
  saver.cancel();
  clock.fireAll();
  assert.deepEqual(written, []);
});
