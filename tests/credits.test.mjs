import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const credits = JSON.parse((await readFile(new URL('../licenses/credits.json', import.meta.url), 'utf8')).replace(/^﻿/, ''));

test('credits list names the notable shipped libraries', () => {
  const required = ['Tauri', 'Svelte', 'libwavpack', 'libopus', 'SQLite', 'Lofty', 'Symphonia', 'rubato', 'smb', 'sspi', 'Media3', 'wasapi', 'rustls', 'ring', 'Rodio', 'CPAL', 'OpenCC', 'ICU4X', 'Tabler', 'image', 'zip', 'RustCrypto', 'Kotlin'];
  for (const name of required) {
    assert.ok(credits.some((c) => c.name.toLowerCase().includes(name.toLowerCase())), `missing credit: ${name}`);
  }
});

test('every credit has https link, license and purpose, and no GPL', () => {
  for (const c of credits) {
    assert.match(c.repositoryUrl, /^https:\/\//, c.name);
    assert.ok(c.license && c.purpose && c.name, c.name);
    assert.doesNotMatch(c.license, /GPL/i, c.name);
  }
});

test('BSD-3 native code is attributed in NOTICE', async () => {
  const notice = await readFile(new URL('../NOTICE', import.meta.url), 'utf8');
  assert.match(notice, /David Bryant/);
  assert.match(notice, /Xiph\.Org/);
});
