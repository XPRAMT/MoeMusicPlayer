[English](README.md) | [繁體中文](README_TW.md)

# MoeMusicPlayer

MoeMusicPlayer is a local-first music library for Windows and Android. User preferences and registered source paths are stored in a versioned `settings.json`; tracks, playlist contents, source mappings, and scan state are stored in SQLite. The interface loads tracks in pages, so the whole collection is not sent to the renderer.

## Features

- Choose Windows music folders with the native folder picker and incrementally scan them in the background. Canceling the picker does not add a source.
- On Android, select shared-audio storage through MediaStore or choose a folder with the system document picker (SAF).
- Keep previously indexed tracks when a source is unavailable or a scan is incomplete.
- Browse and search the saved library with paginated results; the interface never loads the whole collection.
- Configure one shared metadata-column order and visibility for the library and playlists; row numbers and play actions stay fixed.
- Choose Now Playing layout A or B from the page or Settings; the default is artwork first, and the preference is saved.
- Customize the background and accent colors in Settings. Preferences are saved to `settings.json`, and text colors adjust for contrast.
- If settings cannot be recovered from the JSON backup, source synchronization pauses until you re-register and confirm the intended sources; existing indexed music remains available.
- On Windows, play local WAV, MP3, FLAC, and Ogg Vorbis files with play/pause, seek, and volume controls.
- Show the current track's embedded artwork, then same-folder `cover.jpg` or `folder.jpg`, using the original image bytes in Now Playing and the player bar.
- In Windows Now Playing, read local LRC sidecars and embedded lyrics first. When none are available, you can search NetEase and QQ candidates; show synchronized line highlighting when timing is available, apply only high-confidence matches automatically, and select and save a candidate manually. Save display preferences for translation and romanization visibility, inactive lyric text opacity, and primary and auxiliary font sizes.
- Import and export M3U and UTF-8 M3U8 playlists on Windows. A newly imported playlist file is registered as a source; older static playlists are not linked to a guessed file path. Unmatched entries are retained, and relative exports are limited to the selected shared root.
- Use queue navigation, shuffle, and repeat off/all/one modes on Windows. Playlist queue order retains repeated entries.
- Windows system media controls show the current track and support play, pause, stop, seek, and queue navigation when available.

Android playback is not available yet. Real Tauri window, system media flyout, and hardware media-key behavior still need platform acceptance.

## Run on Windows

1. Install Node.js 20.19+ (or 22.12+), Rust 1.90+, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).
2. Run `npm install`.
3. Run `npm run tauri -- dev`.
4. Open **Settings → Music Sources**, choose **Select folder and sync**, and select an existing music folder. Canceling leaves the source list unchanged.
5. Open **Settings → Appearance** to choose a background and accent color. The defaults are pure black and water blue. Open **Settings → Track Columns** to reorder or hide metadata columns for both lists. Choose layout A or B on the Now Playing page or in **Settings → Now Playing**. Preferences and registered source paths are saved in the app data directory as `settings.json`; SQLite stores the music library.

To build the Windows executable, run `npm run release:windows`. The command always builds under the repository's `target` directory, regardless of the caller's `CARGO_TARGET_DIR`, and writes the result to `release/moemusicplayer.exe`. A failed build leaves the previous executable in place.

## Build for Android

Install the Android SDK, NDK, and Java 21, then run:

```sh
npm install
npm run android:build:arm64
```

The debug APK is written to `src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`. On the device, open **Settings → Music Sources** to grant MediaStore access or select a SAF folder. Preferences are saved in `settings.json` in the app data directory.
