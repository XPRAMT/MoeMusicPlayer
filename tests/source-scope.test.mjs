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
