[English](README.md) | [繁體中文](README_TW.md)

# MoeMusicPlayer

MoeMusicPlayer is a local-first music library for Windows and Android. It stores a library projection in SQLite and loads tracks in pages, so the whole collection is not sent to the interface.

## Features

- Choose Windows music folders with the native folder picker and incrementally scan them in the background. Canceling the picker does not add a source.
- On Android, select shared-audio storage through MediaStore or choose a folder with the system document picker (SAF).
- Keep previously indexed tracks when a source is unavailable or a scan is incomplete.
- Browse and search the saved library with paginated results; the interface never loads the whole collection.
- Customize the background and accent colors in Settings. Preferences are saved in the app database, and text colors adjust for contrast.
- On Windows, play local WAV, MP3, FLAC, and Ogg Vorbis files with play/pause, seek, and volume controls.
- Import and export M3U and UTF-8 M3U8 playlists on Windows. Unmatched entries are retained; relative exports are limited to the selected shared root.
- Windows system media controls show the current track and support play, pause, stop, and seek. Previous/next remain disabled until queue navigation is implemented.

The playback queue, previous/next track, repeat, shuffle, and Android playback are not available yet.

## Run on Windows

1. Install Node.js 20.19+ (or 22.12+), Rust 1.90+, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).
2. Run `npm install`.
3. Run `npm run tauri -- dev`.
4. Open **Settings → Music Sources**, choose **Select folder and sync**, and select an existing music folder. Canceling leaves the source list unchanged.
5. Open **Settings → Appearance** to choose a background and accent color. The defaults are pure black and water blue.

To build the Windows executable, run `npm run tauri -- build --ci`. It is written to `target/release/moemusicplayer.exe`.

## Build for Android

Install the Android SDK, NDK, and Java 21, then run:

```sh
npm install
npm run android:build:arm64
```

The debug APK is written to `src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`. On the device, open **Settings → Music Sources** to grant MediaStore access or select a SAF folder. Appearance preferences are saved in the app database.
