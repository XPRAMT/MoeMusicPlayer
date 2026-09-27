[English](README.md) | [繁體中文](README_TW.md)

# MoeMusicPlayer

MoeMusicPlayer 是以本機優先為設計方向的 Windows 與 Android 音樂庫。曲庫資料投影保存在 SQLite，介面以分頁載入曲目，不會把整個曲庫一次送到前端。

## 功能

- 加入 Windows 音樂資料夾並在背景增量掃描。
- Android 可透過 MediaStore 選擇共享音樂儲存空間，或用系統文件選擇器（SAF）選擇資料夾。
- 來源暫時不可用或掃描未完成時，保留先前已索引的曲目。
- 以分頁結果瀏覽及搜尋已保存曲庫；介面不會載入整個曲庫。
- Windows 可播放本機 WAV、MP3、FLAC 與 Ogg Vorbis，並提供播放/暫停、跳轉與音量控制。

播放佇列、前後首、循環、隨機與 Android 播放目前尚未提供。

## 在 Windows 執行

1. 安裝 Node.js 20.19+（或 22.12+）、Rust 1.90+，以及 [Tauri 先決條件](https://v2.tauri.app/start/prerequisites/)。
2. 執行 `npm install`。
3. 執行 `npm run tauri -- dev`。
4. 開啟「來源設定」，輸入現有音樂資料夾路徑，選擇「加入並掃描」。

建置 Windows 執行檔請執行 `npm run tauri -- build --ci`，產物位於 `target/release/moemusicplayer.exe`。

## 建置 Android 版

安裝 Android SDK、NDK 與 Java 21，再執行：

```sh
npm install
npm run android:build:arm64
```

Debug APK 輸出至 `src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`。在裝置上開啟「來源設定」，授權 MediaStore 或選擇 SAF 資料夾。
