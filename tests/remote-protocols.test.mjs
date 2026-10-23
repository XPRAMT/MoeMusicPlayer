import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { WEBDAV_ENTRY_VISIBLE } from '../src/lib/remote-protocols.js';

test('the WebDAV add-entry is offered (ADR 0035 P1)', () => {
  assert.equal(WEBDAV_ENTRY_VISIBLE, true);
});

test('a WebDAV account keeps its pinned certificate when edited, and an untrusted one is offered for pinning', () => {
  const source = readFileSync(new URL('../src/lib/RemoteSourcesSettings.svelte', import.meta.url), 'utf8');
  assert.match(source, /pin: account\.tls\.mode === 'pinned' \? account\.tls\.sha256 : null/);
  assert.match(source, /current\.pin \? \{ mode: 'pinned', sha256: current\.pin \}/);
  assert.match(source, /invokeCommand\('remote_certificate_probe'/);
  // A pinned account whose certificate changed is not asked again.
  assert.match(source, /!text\.includes\('與你先前接受的不同'\)/);
});

test('the settings component gates the WebDAV choice on the flag', () => {
  const source = readFileSync(new URL('../src/lib/RemoteSourcesSettings.svelte', import.meta.url), 'utf8');
  assert.match(source, /import \{ WEBDAV_ENTRY_VISIBLE \} from '\.\/remote-protocols\.js'/);
  assert.match(source, /\{#if WEBDAV_ENTRY_VISIBLE \|\| form\.protocol === 'webdav'\}\s*<div class="remote-protocol"/);
  assert.match(source, /protocol: 'smb', host: ''/);
});

test('without the protocol selector the dialog title names SMB', () => {
  const source = readFileSync(new URL('../src/lib/RemoteSourcesSettings.svelte', import.meta.url), 'utf8');
  assert.match(source, /WEBDAV_ENTRY_VISIBLE \|\| form\.protocol === 'webdav' \? \(form\.id \? tr\('編輯遠端伺服器'\) : tr\('新增遠端伺服器'\)\) : \(form\.id \? tr\('編輯 SMB 伺服器'\) : tr\('新增 SMB 伺服器'\)\)/);
});
