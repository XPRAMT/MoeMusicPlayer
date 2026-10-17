import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { formatSlideshowFolderLabel } from '../src/lib/background-source-label.js';
import { systemWallpaperOpacityPercent } from '../src/lib/backdrop-contrast.js';

test('Android wallpaper opacity never collapses to an opaque overlay in automatic opacity mode', () => {
  assert.equal(systemWallpaperOpacityPercent({ automaticTextColor: 'white', clarityPercent: 62, opacityPercent: 40 }), 38);
  assert.equal(systemWallpaperOpacityPercent({ automaticTextColor: 'black', clarityPercent: 20, opacityPercent: 40 }), 80);
  assert.equal(systemWallpaperOpacityPercent({ automaticTextColor: null, clarityPercent: 62, opacityPercent: 40 }), 40);
  assert.equal(systemWallpaperOpacityPercent({ automaticTextColor: 'white', clarityPercent: 150, opacityPercent: 40 }), 0);
});

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
  assert.equal(formatSlideshowFolderLabel('/storage/emulated/0/Pictures/\u9280\u72fc'), '\u5167\u90e8\u5132\u5b58\u7a7a\u9593/Pictures/\u9280\u72fc');
  assert.equal(formatSlideshowFolderLabel('/storage/emulated/0'), '\u5167\u90e8\u5132\u5b58\u7a7a\u9593');
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
  assert.match(css, /opacity: calc\(1 - var\(--system-wallpaper-opacity/);
  assert.match(app, /--system-wallpaper-opacity: \$\{androidWallpaperOpacityPercent \/ 100\}/);
  const kotlin = readFileSync(new URL('../plugins/tauri-plugin-media-index/android/src/main/java/com/moemusicplayer/mediaindex/MediaIndexPlugin.kt', import.meta.url), 'utf8');
  assert.match(kotlin, /FLAG_SHOW_WALLPAPER/);
  assert.match(kotlin, /fun readSystemWallpaper/);
  assert.match(kotlin, /isExternalStorageManager\(\)/);
  assert.match(kotlin, /ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION/);
  const manifest = readFileSync(new URL('../src-tauri/gen/android/app/src/main/AndroidManifest.xml', import.meta.url), 'utf8');
  assert.match(manifest, /android\.permission\.MANAGE_EXTERNAL_STORAGE/);
  assert.match(app, /androidWallpaperHasImage/);
  // FLAG_SHOW_WALLPAPER fallback caused repaint trails on NX809J; it must stay off.
  assert.match(app, /const ANDROID_WALLPAPER_BACKDROP_ENABLED = false;/);
  assert.match(app, /const enabled = ANDROID_WALLPAPER_BACKDROP_ENABLED && /);
  const details = readFileSync(new URL('../src-tauri/src/track_details.rs', import.meta.url), 'utf8');
  assert.match(details, /fn write_in_place/);
  assert.match(details, /if let Some\(path\) = direct_path\(app, uri\)/);
});

test('track details Save asks before overwriting and cancel keeps edits', () => {
  const dialog = readFileSync(new URL('../src/lib/TrackDetailsDialog.svelte', import.meta.url), 'utf8');
  assert.match(dialog, /onclick=\{requestSave\}/);
  assert.match(dialog, /event\.key\.toLowerCase\(\) === 's'\) \{ event\.preventDefault\(\); requestSave\(\); \}/);
  assert.match(dialog, /if \(confirmingSave\) \{ cancelSave\(\); return; \}/);
  assert.match(dialog, /class="track-details-confirm floating-surface"/);
  assert.match(dialog, /\u5c07\u6703\u8986\u5beb\u6a94\u6848\u539f\u6709\u8cc7\u8a0a\uff0c\u662f\u5426\u7e7c\u7e8c\uff1f/);
  const confirmBody = dialog.slice(dialog.indexOf('function confirmSave'), dialog.indexOf('function focusOnMount'));
  assert.match(confirmBody, /void save\(\)/);
  const cancelBody = dialog.slice(dialog.indexOf('function cancelSave'), dialog.indexOf('function confirmSave'));
  assert.doesNotMatch(cancelBody, /draft|original|save\(/);
});

test('Android shared music uses All files access only (no READ_MEDIA_AUDIO flow)', () => {
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  const kotlin = readFileSync(new URL('../plugins/tauri-plugin-media-index/android/src/main/java/com/moemusicplayer/mediaindex/MediaIndexPlugin.kt', import.meta.url), 'utf8');
  const pluginManifest = readFileSync(new URL('../plugins/tauri-plugin-media-index/android/src/main/AndroidManifest.xml', import.meta.url), 'utf8');
  assert.doesNotMatch(pluginManifest, /READ_MEDIA_AUDIO/);
  assert.match(pluginManifest, /READ_EXTERNAL_STORAGE" android:maxSdkVersion="29"/);
  assert.match(pluginManifest, /WRITE_EXTERNAL_STORAGE" android:maxSdkVersion="29"/);
  assert.doesNotMatch(kotlin, /READ_MEDIA_AUDIO|requestMediaReadPermission/);
  assert.doesNotMatch(app, /android_media_request_permission|mediaPermissionGranted/);
  assert.match(app, /capabilities\?\.platform === 'android' && allFilesAccess && !allFilesAccess\.granted/);
});

test('Android sources are real paths with no per-item SAF/MediaStore grants (ADR 0014)', () => {
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  const ipc = readFileSync(new URL('../src/lib/ipc.ts', import.meta.url), 'utf8');
  const kotlin = readFileSync(new URL('../plugins/tauri-plugin-media-index/android/src/main/java/com/moemusicplayer/mediaindex/MediaIndexPlugin.kt', import.meta.url), 'utf8');
  const rust = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8');
  const pluginManifest = readFileSync(new URL('../plugins/tauri-plugin-media-index/android/src/main/AndroidManifest.xml', import.meta.url), 'utf8');
  assert.doesNotMatch(app, /MediaStore<\/span>|<span>SAF<\/span>|android_media_list_volumes|android_media_add_volume|android_saf_pick_source/);
  assert.doesNotMatch(ipc, /android_media_list_volumes|android_media_add_volume|android_saf_pick_source|MediaStoreVolumeOption/);
  assert.match(app, /invokeCommand\('android_pick_folder_source', \{\}\)/);
  assert.doesNotMatch(kotlin, /takePersistableUriPermission|createWriteRequest|RecoverableSecurityException|fun pickSafTree|fun scanMediaStore|fun pickMyLoveFolder/);
  assert.match(kotlin, /fun pickDirectoryPath/);
  assert.match(rust, /android_path_migration::migrate_legacy_sources/);
  assert.doesNotMatch(rust, /pick_saf_tree|has_saf_permission|has_saf_write_permission|resolve_saf_playlist_entry|write_audio_cache_lease/);
  assert.doesNotMatch(pluginManifest, /READ_MEDIA_AUDIO/);
  const service = readFileSync(new URL('../plugins/tauri-plugin-media-index/android/src/main/java/com/moemusicplayer/mediaindex/PlaybackService.kt', import.meta.url), 'utf8');
  assert.match(service, /uri\.startsWith\("file:\/\/\/"\) \|\| uri\.startsWith\("content:\/\/"\)/);
  assert.doesNotMatch(service, /requires an authorized content URI/);
});
