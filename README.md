[English](README.md) | [繁體中文](README_TW.md)

# MoeMusicPlayer

**A simple, fast and professional local music player for Windows and Android.** Your music stays on your device: add folders or M3U/M3U8 playlists, and MoeMusicPlayer builds a searchable library without any online account.

> **Source-available, not open source.** The source code is public and you may use and modify it for noncommercial purposes, but commercial use is not permitted. See [License](#license).

## Highlights

- **Simple:** add a folder, press play. Lists, search, sorting and playlists work the way you expect, the Now Playing page shows the cover and the lyrics side by side, and every setting is in one place.
- **Fast:** large libraries are browsed through paginated, virtualized lists, so thousands of songs scroll smoothly. Scanning reads tags on several threads: scanning is quick when the files are already in the operating system's file cache (a repeat scan), while the very first scan has to read every file from the disk and takes as long as your drive needs: much longer on a hard disk than on an SSD. Closing the window during a scan saves what was read, and the next scan continues from there.
- **Professional:** edit tags and cover art in place, view a spectrogram, play DSD, and on Windows send the sound bit-perfect through WASAPI exclusive mode or to several devices at once.
- **Automatic lyrics:** the player finds lyrics for you. It reads a local `.lrc` file or embedded lyrics first; if there is none, it searches NetEase and QQ and applies a result by itself only when the title matches closely, the length differs by at most 2 seconds and the lyrics are synchronized. You can always search, pick another result or reload by hand, and a lyric you picked is never overwritten by an automatic one.

## Features on both Windows and Android

- **Music library:** add folders and M3U/M3U8 playlists, exclude folders from scans, and keep previously indexed tracks when a source is unavailable or a scan is incomplete. Video files are not added.
- **Remote libraries (SMB):** add music folders and `.m3u`/`.m3u8` playlists that live on an SMB server (Settings → Music sources, one tab per server; passwords are stored encrypted on the device). Songs play straight from the server through a disk cache (0.1–5 GB, set in Settings → Music sources), covers, lyrics and tags are read from the server too, and a song on the server can be opened and edited in the details panel. The favorite button can add or remove songs in a playlist file on the server, keeping the file's own path style and one `.bak` of the previous version. A file reached through several sources (a playlist and a folder on the same server) is one track, never two. A top-bar switch shows only local songs, everything or only remote songs. Scanning is fast: a library on an SMB server is scanned by reading only each file's tags and audio properties, never the cover pictures. A FLAC file keeps its picture in separate blocks of known length, so the reader skips over it: about 32 KiB and 2 small requests per song. MP3, M4A, DSF and Opus keep the picture inside the tag, so the whole cover has to be read before it is discarded, and large covers make those files slower to scan over a network; FLAC is therefore the best format for a remote library. The SMB client has its own transport with `TCP_NODELAY` and one write per packet, which removed a stall of about 45 ms per round trip (an idle request went from about 187 ms to about 4.5 ms on the author's setup). A repeat scan, with the files in the server's cache, is much faster than the first one, when the NAS has to read the tags from its disks. Results depend on your server, disks and network. Later scans read only new or changed files (unchanged files are skipped by size and modified time).
- **Lists and playlists:** paginated lists with search, sortable columns, configurable column order and visibility, natural number sorting (Track 2 before Track 10), and group headings with a quick index. Click a row to play; drag a queue row to reorder the queue. Columns include the source (This device or the server name) and the file's modified time; artist and album indexes show track counts and accumulated listening time. The Hi-Res badge appears for 48 kHz / 24-bit or better.
- **Playback:** shuffle (weighted by your listening time), previous/next, repeat off/all/one, resume after a restart, a sleep timer, and swipe gestures on touch screens (swipe the cover or the lyrics for the next or previous song).
- **Now Playing:** layout A (cover left, lyrics right) or B; zoom the cover from 0.5× to 10×; adjust lyric size, spacing, shadow or outline, translation and romanization, Traditional-Chinese display of Simplified lyrics, and the sync delay of each song.
- **Song details and tag editing:** open the info button in the Now Playing top bar to see the path and audio properties and to edit title, artist, album, album artist, composer, genre, date, track/disc numbers, embedded lyrics and one front cover (replace, paste, export, copy or remove). Changes are written only when you press **Save**. **Import** puts the lyrics in use into the file.
- **Spectrogram:** an FFT spectrogram with selectable palette, window function, FFT size and dB range; pinch or scroll to zoom. Windows opens it in its own window, Android as a full-screen page.
- **DSD:** `.dsf` files are converted to 176.4 kHz PCM with an adjustable gain. Windows adds a 352.8 kHz output and DoP (see below).
- **Appearance and language:** background and accent color; five main-background modes: solid color, single image, slideshow, album cover and the system wallpaper (on Android the wallpaper needs **All files access**); automatic text color or automatic image transparency. The interface is available in Traditional Chinese, Simplified Chinese, Japanese and English (Settings → Main → Language).
- **Backup and restore:** Settings → About → Backup and restore exports what you tick (library, settings, server accounts under a passphrase, favorites; never album covers) into one zip, and imports that zip or a single file from it (`settings.json`, the library database, `MyLove.m3u8`). An import is applied the next time the app starts, and the previous data is kept in a `before-import` folder.
- **Updates:** the player checks the latest GitHub Release and offers the new version (see [Install](#install)). Settings → About shows the build and the open-source packages used.

## Windows and Android differences

| | Windows (x64) | Android (arm64) |
|---|---|---|
| Formats | WAV, MP3, FLAC, AAC/M4A, Ogg Vorbis, Ogg Opus, WavPack (`.wv`, lossless and hybrid, with a matching `.wvc`; also DSD-in-WavPack), DSD (`.dsf`), Matroska audio (`.mka`) | What Android's media framework plays: the common formats above, plus WavPack and DSD (`.dsf`) through the app's own decoders |
| Output | Shared mode, **WASAPI exclusive mode** (bit-perfect), **DoP** for DSD, **several output devices at once** with per-device volume, delay and channel mapping | The system audio output |
| DSEE HX upscaling | Optional (needs Sony Music Center; its filter is not included) | No |
| Lyrics display | Lyrics page **and taskbar lyrics** | Lyrics page |
| System integration | System media controls, open files from File Explorer, play from the default-app list | Media notification and lock screen controls, background playback |
| Music sources | Folders and playlists anywhere, SMB servers; background scanning | Folders and playlists by real path after you turn on **All files access** |
| Spectrogram | Separate window with A/B comparison of two files | Full-screen page |
| Data location | `UserData` folder beside the program (portable) | The app's private data folder |
| Updates | Ignore, install automatically (next launch) or install and restart | Download and install through the system installer |

## Install

### Windows

Requires 64-bit Windows 10 or 11 with the Microsoft Edge WebView2 Runtime (built into Windows 11 and most up-to-date Windows 10 installations).

1. Download `moemusicplayer-windows-x86_64.zip` from the [latest release](https://github.com/XPRAMT/MoeMusicPlayer/releases/latest).
2. Extract `moemusicplayer.exe` into any folder you own (for example `D:\MoeMusicPlayer`) and run it. The program is portable: it keeps `settings.json` and the library in a `UserData` folder next to the executable, so moving the executable to another folder starts a fresh data set.
3. Open **Settings → Music Sources** (`設定 → 音樂來源`) and add a folder or a playlist. The library is built in the background.
4. Optional: to open audio files from File Explorer, register the formats in **Settings → About** (`設定 → 關於`, **登錄音訊格式**) and choose MoeMusicPlayer in Windows **Default apps**.

Updates: at startup the player checks the latest release and offers **Ignore** (the same version is not offered again unless you check by hand in Settings → About), **Install automatically** (the new program takes effect the next time you start it) or **Install and restart**. `UserData` is never touched.

### Android

Requires Android 7.0 or later on a 64-bit (arm64) phone or tablet.

1. If you installed a version older than 26.10.21, uninstall it first: the app is now signed with a different key, and Android refuses to update an app across keys (uninstalling removes its data, so export a backup in Settings → About first). Then download `moemusicplayer-android-arm64.apk` from the [latest release](https://github.com/XPRAMT/MoeMusicPlayer/releases/latest) and open it. Android asks you to allow your browser or file manager to install unknown apps; allow it once and confirm the installation.
2. Open the app, go to **Settings → Music Sources**, turn on **All files access** for MoeMusicPlayer (Android takes you to the system page), then add a folder or a playlist.
3. On Android 13 or later, allow notifications so the media notification and lock-screen controls can appear.

Updates: the app checks the same release. **Download and install** verifies the file and hands it to the system installer; the first time, Android asks you to allow MoeMusicPlayer to install unknown apps.

For a diagnostic report that can be shared with Codex, run `moemusicplayer.exe --diagnostics-json` from a terminal on Windows. The JSON includes build and process information, startup database/audio status, file sizes, and recent panic records. It does not include music metadata, settings contents, or SQLite records. It does include local executable and data paths, so review the report before sharing it.

## Develop on Windows

Install Node.js 20.19+ (or 22.12+), Rust 1.90+, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/), then run:

```sh
npm install
npm run tauri -- dev
```

Build the portable Windows executable with:

```sh
npm run release:windows
```

Run the frontend checks and Rust workspace tests with `npm run check` and `cargo test --workspace --locked --offline`.

The fixed output is `release/moemusicplayer.exe`; keep it beside `release/UserData/`. The release zip is packaged under `release/update-assets/`; packaging does not publish a release. An update is identified only by the version (the build date, tag `v26.10.21`), so publish at most one release a day.

## Build for Android

Install the Android SDK, NDK, and Java 21, then run:

```sh
npm install
npm run android:build:arm64:release
```

This builds the smallest release APK (about 30 MB) and an AAB for Google Play, and copies them to `release/moemusicplayer-android-arm64-release.apk` and `.aab`. Both are signed with the release key taken from the `MOE_ANDROID_KEYSTORE`, `MOE_ANDROID_KEY_ALIAS` and `MOE_ANDROID_KEYSTORE_PASSWORD` environment variables (without a keystore the debug key is used); the version code is the build date as a number. `npm run android:build:arm64` builds a larger debug APK that can be inspected with WebView debugging. Turn on **All files access** in **Settings → Music Sources**, then add a folder or playlist. `npm run release:android` writes `release/update-assets/moemusicplayer-android-arm64.apk` and publishes only with `-Upload -ReleaseTag <tag>`.

## License

MoeMusicPlayer is **source-available software, not open source**, licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE).

- You may use it and modify it for noncommercial purposes (personal use, hobby projects, research, education, and similar).
- You may redistribute it, with or without changes, under the same license. When you do, include the license text (or its URL) and the `Required Notice:` line from [NOTICE](NOTICE) so recipients can find the original source.
- Commercial use is not permitted.
