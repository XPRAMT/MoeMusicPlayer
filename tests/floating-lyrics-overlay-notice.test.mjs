import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { floatingLyricsOverlayNoticeVisible } from '../src/lib/floating-lyrics-settings.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const read = (file) => readFileSync(path.join(root, file), 'utf8');

test('the overlay-permission notice shows on Android only while the permission is known to be missing', () => {
  assert.equal(floatingLyricsOverlayNoticeVisible('android', { canDrawOverlays: false }), true);
  assert.equal(floatingLyricsOverlayNoticeVisible('android', {}), true);
  assert.equal(floatingLyricsOverlayNoticeVisible('android', { canDrawOverlays: true }), false, 'granted: nothing at all');
  assert.equal(floatingLyricsOverlayNoticeVisible('android', null), false, 'not read yet: no flash');
  assert.equal(floatingLyricsOverlayNoticeVisible('android', undefined), false);
  assert.equal(floatingLyricsOverlayNoticeVisible('windows', { canDrawOverlays: false }), false, 'Windows has no such block');
  assert.equal(floatingLyricsOverlayNoticeVisible(undefined, { canDrawOverlays: false }), false);
});

test('the notice is the first thing in 顯示, before 啟用', () => {
  const look = read('src/lib/LyricsLookSettings.svelte');
  const heading = look.indexOf('tr("顯示")}</h3>');
  const notice = look.indexOf('{@render displayNotice?.()}');
  const enable = look.indexOf('{@render enable?.()}');
  const translation = look.indexOf('tr("顯示譯文")');
  assert.ok(heading > 0 && notice > heading && enable > notice && translation > enable, 'heading, notice, 啟用, 顯示譯文');

  const app = read('src/App.svelte');
  const start = app.indexOf('idPrefix="{scope}floating-lyrics"');
  const end = app.indexOf('{#snippet position()}', start);
  const floating = app.slice(start, end);
  const noticeSnippet = floating.indexOf('{#snippet displayNotice()}');
  const enableSnippet = floating.indexOf('{#snippet enable()}');
  assert.ok(noticeSnippet > 0 && enableSnippet > noticeSnippet);
  const block = floating.slice(noticeSnippet, enableSnippet);
  assert.match(block, /\{#if floatingLyricsOverlayNoticeVisible\(capabilities\?\.platform, floatingLyricsAndroidStatus\)\}/);
  assert.match(block, /tr\("顯示在其他應用程式上層"\)/);
  assert.match(block, /onclick=\{openOverlayPermissionSettings\}/);
  assert.doesNotMatch(app, /tr\("已允許。"\)/, 'no 「已允許。」 line any more');
  assert.equal((app.match(/tr\("顯示在其他應用程式上層"\)/g) ?? []).length, 1, 'the block lives in one place only');
});

test('the permission status is read again when the app comes back (visibilitychange and window focus)', () => {
  const app = read('src/App.svelte');
  const start = app.indexOf("if (!isAndroidPlatform || settingsSection !== 'lyrics' || visibleLyricsSettingsTab !== 'floating') return;");
  assert.ok(start > 0);
  const effect = app.slice(start, app.indexOf('});', app.indexOf('return () =>', start)));
  assert.match(effect, /void refreshFloatingLyricsAndroidStatus\(\);/);
  assert.match(effect, /document\.addEventListener\('visibilitychange', onVisible\)/);
  assert.match(effect, /window\.addEventListener\('focus', onFocus\)/);
  assert.match(effect, /document\.removeEventListener\('visibilitychange', onVisible\)/);
  assert.match(effect, /window\.removeEventListener\('focus', onFocus\)/);
});
