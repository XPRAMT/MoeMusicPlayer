import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import './helpers/untr-fs.mjs';
import { getScopeIndexFromPosition, normalizeSourceScope, shouldShowSourceScopeSwitch, SOURCE_SCOPES } from '../src/lib/source-scope.js';

test('the switch needs both a local and a remote source that are enabled', () => {
  const local = { kind: 'windowsFilesystem', enabled: true };
  const remote = { kind: 'remote', enabled: true };
  assert.equal(shouldShowSourceScopeSwitch([local, remote]), true);
  assert.equal(shouldShowSourceScopeSwitch([local]), false);
  assert.equal(shouldShowSourceScopeSwitch([remote]), false);
  assert.equal(shouldShowSourceScopeSwitch([]), false);
  assert.equal(shouldShowSourceScopeSwitch([local, { ...remote, enabled: false }]), false);
  assert.equal(shouldShowSourceScopeSwitch([{ ...local, enabled: false }, remote]), false);
  // A playlist file kept on a server is a remote source too (its kind is a playlist file, the account marks it).
  const remotePlaylist = { kind: 'playlist_file', remoteAccountId: 'acct', enabled: true };
  assert.equal(shouldShowSourceScopeSwitch([local, remotePlaylist]), true);
  assert.equal(shouldShowSourceScopeSwitch([remotePlaylist]), false);
  assert.equal(shouldShowSourceScopeSwitch([{ kind: 'playlist_file', enabled: true }, remote]), true);
});

test('the stops run local, all, remote from left to right and a position picks the nearest third', () => {
  assert.deepEqual([...SOURCE_SCOPES], ['local', 'all', 'remote']);
  assert.equal(getScopeIndexFromPosition(2, 78), 0);
  assert.equal(getScopeIndexFromPosition(39, 78), 1);
  assert.equal(getScopeIndexFromPosition(77, 78), 2);
  assert.equal(getScopeIndexFromPosition(-20, 78), 0);
  assert.equal(getScopeIndexFromPosition(200, 78), 2);
  assert.equal(getScopeIndexFromPosition(10, 0), 1);
});

test('an unknown stored value means everything', () => {
  assert.equal(normalizeSourceScope('local'), 'local');
  assert.equal(normalizeSourceScope('remote'), 'remote');
  assert.equal(normalizeSourceScope('all'), 'all');
  assert.equal(normalizeSourceScope(undefined), 'all');
  assert.equal(normalizeSourceScope('x'), 'all');
});

test('the top bar shows the switch only when it applies and every list reloads on a change', () => {
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  assert.match(app, /\{#if showSourceScopeSwitch\}[\s\S]*<SourceScopeSwitch/);
  assert.match(app, /settings_set_library_source_scope[\s\S]{0,200}libraryListRevision \+= 1/);
});

test('a scope switch reloads the lists but leaves the play queue as it is', () => {
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  const change = app.match(/async function changeLibrarySourceScope[\s\S]*?\n  \}\n/)?.[0] ?? '';
  assert.match(change, /playlistListRevision \+= 1/);
  assert.doesNotMatch(change, /playbackQueueResetKey/);
});

test('every list builds its queue from its own page query, so the queue holds exactly the rows it shows', () => {
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  const artist = app.match(/async function playArtistTrack[\s\S]*?\n  \}\n/)?.[0] ?? '';
  assert.match(artist, /kind: 'artist'/);
  assert.match(artist, /artistNames: \[\.\.\.selectedArtist\.artistNames\]/);
  const playlist = app.match(/async function playPlaylistEntry[\s\S]*?\n  \}\n/)?.[0] ?? '';
  assert.match(playlist, /kind: 'playlist'[\s\S]*sort,/);
  const entries = readFileSync(new URL('../src/lib/PlaylistEntryList.svelte', import.meta.url), 'utf8');
  assert.match(entries, /onPlay\(entry, playlistId, sort\)/);
});

test('the scope is an argument of each query, never state kept by a database handle', () => {
  const database = readFileSync(new URL('../crates/player-db/src/database.rs', import.meta.url), 'utf8');
  const lib = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8');
  const actor = readFileSync(new URL('../src-tauri/src/android_playback.rs', import.meta.url), 'utf8');
  for (const source of [database, lib, actor]) {
    assert.doesNotMatch(source, /set_source_scope|\.source_scope\(\)/);
  }
  assert.doesNotMatch(database, /source_scope: Arc</);
  // The switch only saves the setting: every command reads it where it runs.
  const switchCommand = lib.match(/fn settings_set_library_source_scope[\s\S]*?\n\}\n/)?.[0] ?? '';
  assert.match(switchCommand, /library_source_scope = scope/);
  assert.doesNotMatch(switchCommand, /database/);
  assert.match(actor, /queue_for_track\(&self\.database, scope,/);
});
