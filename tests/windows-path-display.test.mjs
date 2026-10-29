import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { displayWindowsPath } from '../src/lib/windows-path-display.js';

test('extended-length prefixes are dropped for display only', () => {
  assert.equal(displayWindowsPath(String.raw`\\?\D:\Music\hanser.m3u8`), String.raw`D:\Music\hanser.m3u8`);
  assert.equal(displayWindowsPath(String.raw`\\?\UNC\nas\share\Music`), String.raw`\\nas\share\Music`);
  assert.equal(displayWindowsPath(String.raw`\\?\unc\nas\share`), String.raw`\\nas\share`);
  assert.equal(displayWindowsPath(String.raw`D:\Music\MyLove.m3u8`), String.raw`D:\Music\MyLove.m3u8`);
  assert.equal(displayWindowsPath(String.raw`\\nas\share`), String.raw`\\nas\share`);
  assert.equal(displayWindowsPath('smb://nas/share/Music'), 'smb://nas/share/Music');
  assert.equal(displayWindowsPath(undefined), '');
});

test('source sections show the display path and put each picker in its card title row', () => {
  const source = readFileSync(new URL('../src/lib/SourceSections.svelte', import.meta.url), 'utf8');
  assert.match(source, /use:appTooltip=\{displayWindowsPath\(row\.location\)\}>\{displayWindowsPath\(row\.location\)\}/);
  for (const [title, button] of [['資料夾', '選擇資料夾'], ['播放清單', '選擇播放清單'], ['已排除的來源', '選擇排除資料夾']]) {
    const heading = new RegExp(`<div class="configured-sources-heading">\\s*<div class="configured-sources-title"><strong>\\{tr\\("${title}"\\)\\}</strong>[^\\n]*\\n\\s*<button[^\\n]*\\{tr\\("${button}"\\)\\}</button>`);
    assert.match(source, heading, title);
  }
  assert.doesNotMatch(source, /source-folder-picker/);
});
