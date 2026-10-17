import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { formatSlideshowFolderLabel } from '../src/lib/background-source-label.js';

test('slideshow folder label prettifies Android SAF tree URIs and keeps desktop paths', () => {
  assert.equal(formatSlideshowFolderLabel(null), null);
  assert.equal(formatSlideshowFolderLabel('D:\\Pictures\\Wall'), 'D:\\Pictures\\Wall');
  assert.equal(
    formatSlideshowFolderLabel('content://com.android.externalstorage.documents/tree/primary%3APictures%2FWallpapers'),
    '\u5167\u90e8\u5132\u5b58\u7a7a\u9593/Pictures/Wallpapers',
  );
  assert.equal(
    formatSlideshowFolderLabel('content://com.android.externalstorage.documents/tree/1A2B-3C4D%3ADCIM'),
    '1A2B-3C4D/DCIM',
  );
  assert.equal(formatSlideshowFolderLabel('content://x/tree/primary%3A'), '\u5167\u90e8\u5132\u5b58\u7a7a\u9593');
  assert.equal(formatSlideshowFolderLabel('content://x/tree/%E0%A4%A'), 'content://x/tree/%E0%A4%A');
});

test('Android background modes are enabled and wallpaper uses the native transparent backdrop', () => {
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  const css = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');
  assert.doesNotMatch(app, /platform === 'android' && \(mode === 'singleImage'/);
  assert.doesNotMatch(app, /option\.mode === 'slideshow' \|\| option\.mode === 'systemWallpaper'\)\}/);
  assert.match(app, /appearance_set_android_wallpaper_backdrop/);
  assert.match(app, /classList\.toggle\('has-android-wallpaper-backdrop'/);
  assert.match(app, /main-backdrop is-system-wallpaper/);
  assert.match(css, /:root\.has-android-wallpaper-backdrop body \{\s*background: transparent;/);
  assert.match(css, /opacity: calc\(1 - var\(--main-background-opacity/);
  const kotlin = readFileSync(new URL('../plugins/tauri-plugin-media-index/android/src/main/java/com/moemusicplayer/mediaindex/MediaIndexPlugin.kt', import.meta.url), 'utf8');
  assert.match(kotlin, /FLAG_SHOW_WALLPAPER/);
  assert.doesNotMatch(kotlin, /WallpaperManager\.getInstance|getWallpaperFile\(/);
  const manifest = readFileSync(new URL('../src-tauri/gen/android/app/src/main/AndroidManifest.xml', import.meta.url), 'utf8');
  assert.doesNotMatch(manifest, /MANAGE_EXTERNAL_STORAGE|READ_EXTERNAL_STORAGE/);
});
