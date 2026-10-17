[English](README.md) | [繁體中文](README_TW.md)

# MoeMusicPlayer

A local-first music player and library for Windows and Android. Add music folders or M3U/M3U8 playlists, browse and search your collection, and keep playback preferences and music data on your device.

> **Source-available, not open source.** The source code is public and you may use and modify it for noncommercial purposes, but commercial use is not permitted. See [License](#license).

## Features

- **Music library:** Windows scans folders and playlist files in the background. Android scans folders and playlist files by their real paths after you turn on **All files access**. On both platforms you can exclude folders; their tracks are skipped by scans. Scans keep previously indexed tracks when a source is unavailable or incomplete. Video files are excluded.
- **Lists and playlists:** Browse large collections with paginated lists, search, sortable columns, and configurable column order and visibility. Track names use natural numeric sorting, so Track 2 appears before Track 10. Playlist sorting does not change the playlist's saved order. Click a row to play it; drag a queue row to reorder the queue. The Hi-Res badge appears for audio at 48 kHz or higher and 24-bit or higher. The artist index shows distinct track and album counts plus accumulated listening time; select an artist to browse their songs.
- **Playback:** Use shuffle, previous/next, and repeat off/all/one. Listening time is saved and informs the order of newly generated shuffle queues. Windows also supports system media controls; Android playback can continue in the background through its media notification.
- **Now Playing and lyrics:** Choose layout A (cover left, lyrics right) or B (lyrics left, cover right). View embedded artwork and read local LRC or embedded lyrics. Search NetEase and QQ lyric candidates, choose a match, and use synchronized line scrolling when timing is available. Translation, romanization, lyric appearance, and sync delay can be adjusted in Settings or Quick Settings. On Windows, **taskbar lyrics** (Settings → Lyrics) draw the current synchronized line in the free space of the taskbar. Artist names split by `|`, `\`, `/`, `;`, or `,` can be clicked to filter the library; spaces remain part of a name.
- **Interface language:** Settings → Main → Language: follow the system, Traditional Chinese, Simplified Chinese, Japanese or English.
- **Song details (Windows and Android):** The info button in the Now Playing top bar opens the current track's details: full path, audio properties, and editable title, artist, album, album artist, composer, genre, date, track/disc numbers, embedded lyrics, and one front cover (replace, export, or remove; optional JPG quality and short-edge resize). Changes are written only when you press **Save** (MP3 is saved as ID3v2.4). **Import** copies the online lyrics currently in use into the lyrics field. **Spectrogram** opens a separate window with an FFT spectrogram (a full-screen page on Android). **Paste** uses a clipboard image as the new cover; Android reads the clipboard natively. **Show in folder** (Windows) or **Open in file manager** (Android) shows the track's folder.
- **Appearance:** Choose a background and accent color. The main background can use a solid color, image, slideshow, current album cover, or (Windows only) the current Windows wallpaper. Slideshows include subfolders, play in order or randomly, and switch every 1–120 minutes. Choose automatic text color, which adapts to the background, or automatic image transparency. In transparency mode, select white or black text and set clarity; the player calculates image opacity to meet the corresponding brightness target. Main-interface and Now Playing cover appearance are configured separately. Android currently supports solid-color and album-cover backgrounds.
- **Audio options (Windows):** Play WAV, MP3, FLAC, AAC/M4A, Ogg Vorbis (`.ogg`/`.oga`), Ogg Opus (`.opus`, or Opus inside `.ogg`; mono and stereo), DSD (`.dsf`, stereo or mono, converted to 176.4 kHz PCM; DSD64 to DSD512), and Matroska audio (`.mka`, depending on its audio codec). Tags of DSF and Opus files can be edited in the song details panel. `.mp4` and `.mkv` video are not included in the library. In **Settings → Output**, choose the app's rubato FFT resampler or Windows built-in sample-rate conversion. Optional DSEE HX requires Sony Music Center; its proprietary filter is not included with this project.
- **Updates and credits:** **Settings → About** shows author [XPRAMT](https://github.com/XPRAMT), build information, and open-source packages with their versions, licenses, and official links. Windows x64 checks for updates at startup and offers **Ignore** (this notification only), **Automatically restart**, or **Complete the update on next launch**. Updates preserve `UserData`. Android arm64 checks the same GitHub Release for an APK; **Download and install** verifies it, then hands it to the system installer. Android first asks you to allow this app to install unknown apps.

## Run on Windows

The interface currently uses Traditional Chinese; the menu names below include the visible labels where useful.

1. Download and extract the [latest Windows release](https://github.com/XPRAMT/MoeMusicPlayer/releases/latest). Keep `moemusicplayer.exe` and `moemusicplayer-updater.exe` together.
2. Start the player and open **Settings → Music Sources** (`設定 → 音樂來源`) to add a folder or playlist.
3. To open audio files from File Explorer, register the supported formats in **Settings → About** (`設定 → 關於`, **登錄音訊格式**), then choose MoeMusicPlayer in Windows **Default apps**. Opening a file replaces the current queue with supported audio files from the same folder; subfolders are not scanned.

The Windows portable build stores `settings.json` and the SQLite library in a `UserData` folder beside the executable. Keep that folder with the two executables. Moving the executables to a different folder uses a different data folder. Android stores data in its platform-managed app directory.

For a diagnostic report that can be shared with Codex, run `moemusicplayer.exe --diagnostics-json` from a terminal. The JSON includes build and process information, startup database/audio status, file sizes, and recent panic records. It does not include music metadata, settings contents, or SQLite records. It does include local executable and data paths, so review the report before sharing it.

## Develop on Windows

Install Node.js 20.19+ (or 22.12+), Rust 1.90+, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/), then run:

```sh
npm install
npm run tauri -- dev
```

Build the portable Windows executables with:

```sh
npm run release:windows
```

Run the frontend checks and Rust workspace tests with `npm run check` and `cargo test --workspace --locked --offline`.

The fixed output is `release/moemusicplayer.exe` and `release/moemusicplayer-updater.exe`. Keep both beside `release/UserData/`. Update assets are packaged under `release/update-assets/`; packaging does not publish a release.

## Build for Android

Install the Android SDK, NDK, and Java 21, then run:

```sh
npm install
npm run android:build:arm64
```

The debug APK is written to `src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`. Turn on **All files access** in **Settings → Music Sources**, then add a folder or playlist. `npm run release:android` builds the APK with a fixed build identity. It also writes the in-app update assets (`moemusicplayer-android-arm64.apk`, `moemusicplayer-android-update.json`) to `release/update-assets/`. It publishes only with `-Upload -ReleaseTag <tag>`. The APK must be signed with the same key as the installed app.

## License

MoeMusicPlayer is **source-available software, not open source**, licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE).

- You may use it and modify it for noncommercial purposes (personal use, hobby projects, research, education, and similar).
- You may redistribute it, with or without changes, under the same license. When you do, include the license text (or its URL) and the `Required Notice:` line from [NOTICE](NOTICE) so recipients can find the original source.
- Commercial use is not permitted.

Artwork: `icon/SilverWolfIcon.png`, `icon/DefaultCover.jpg` and `src/assets/hi-res-badge.svg` are unofficial illustrations made or redrawn with AI by the author and are provided under this license; no rights are granted to the character (Silver Wolf) or the Hi-Res Audio logo and trademark they depict, which belong to their owners (see [NOTICE](NOTICE)). Third-party software the project depends on keeps its own license; **Settings → About** and `licenses/credits.json` list them with versions and links.
