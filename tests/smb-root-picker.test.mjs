import './helpers/untr-fs.mjs';
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../src/lib/RemoteSourcesSettings.svelte', import.meta.url), 'utf8');

test('the SMB root picker offers only shares: clicking one picks it, no browsing in', () => {
  assert.match(source, /onclick=\{\(\) => form\?\.protocol === 'webdav' \? void loadPick\(folder\.path\) : pickShare\(folder\.path\)\}/);
  assert.match(source, /\{#if pickPath !== '' && form\?\.protocol === 'webdav'\}/);
  assert.match(source, /\{#if form\?\.protocol === 'webdav'\}\s*<button[^>]*onclick=\{usePickedFolder\}/);
  const pick = source.slice(source.indexOf('function pickShare'), source.indexOf('async function saveForm'));
  assert.match(pick, /share\.includes\('\/'\)\) return;/);
  assert.match(pick, /form\.path = share;/);
});

test('a deeper saved SMB root is kept unless a share is picked', () => {
  // Opening the picker for SMB does not touch form.path; only pickShare and the WebDAV button write it.
  const open = source.slice(source.indexOf('function openSharePicker'), source.indexOf('function usePickedFolder'));
  assert.doesNotMatch(open, /form\.path\s*=/);
  const use = source.slice(source.indexOf('function usePickedFolder'), source.indexOf('function pickShare'));
  assert.match(use, /form\.protocol !== 'webdav'\) return;/);
});
