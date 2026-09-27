[English](README.md) | [繁體中文](README_TW.md)

# MoeMusicPlayer

MoeMusicPlayer is a local-first music library for Windows and Android. It stores a library projection in SQLite and loads tracks in pages, so the whole collection is not sent to the interface.

## Features

- Add Windows music folders and incrementally scan them in the background.
- On Android, select shared-audio storage through MediaStore or choose a folder with the system document picker (SAF).
- Keep previously indexed tracks when a source is unavailable or a scan is incomplete.
- Browse and search the saved library with paginated results; the interface never loads the whole collection.

Playback controls are shown, but audio playback is not available yet and the controls remain disabled.

## Run on Windows

1. Install Node.js 20.19+ (or 22.12+), Rust 1.90+, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).
2. Run `npm install`.
3. Run `npm run tauri -- dev`.
4. Open **Source Settings**, enter an existing music folder path, and choose **Add and scan**.

To build the Windows executable, run `npm run tauri -- build --ci`. It is written to `target/release/moemusicplayer.exe`.

## Build for Android

Install the Android SDK, NDK, and Java 21, then run:

```sh
npm install
npm run android:build:arm64
```

The debug APK is written to `src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`. On the device, open **Source Settings** to grant MediaStore access or select a SAF folder.
