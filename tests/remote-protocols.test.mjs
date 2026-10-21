import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { WEBDAV_ENTRY_VISIBLE } from '../src/lib/remote-protocols.js';

test('the WebDAV add-entry is hidden for now', () => {
  assert.equal(WEBDAV_ENTRY_VISIBLE, false);
});

test('the settings component gates the WebDAV choice on the flag', () => {
  const source = readFileSync(new URL('../src/lib/RemoteSourcesSettings.svelte', import.meta.url), 'utf8');
  assert.match(source, /import \{ WEBDAV_ENTRY_VISIBLE \} from '\.\/remote-protocols\.js'/);
  assert.match(source, /\{#if WEBDAV_ENTRY_VISIBLE \|\| form\.protocol === 'webdav'\}\s*<div class="remote-protocol"/);
  assert.match(source, /protocol: 'smb', host: ''/);
});
