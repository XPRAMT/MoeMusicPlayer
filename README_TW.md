[English](README.md) | [繁體中文](README_TW.md)

# MoeMusicPlayer

MoeMusicPlayer 是適用於 Windows 與 Android 的本機優先音樂播放器與曲庫。你可以加入音樂資料夾或 M3U/M3U8 播放清單、瀏覽及搜尋曲庫，並將播放偏好與音樂資料保存在裝置上。

## 功能

- **音樂來源：** Windows 可在背景掃描資料夾與播放清單檔。Android 可使用共享音訊儲存空間，或透過系統文件選擇器（SAF）選擇資料夾。來源暫時無法使用或掃描未完成時，會保留先前已索引的曲目；影片不會加入曲庫。
- **曲庫與播放清單：** 以分頁清單瀏覽大型曲庫、搜尋歌曲、排序欄位，並調整欄位順序與顯示狀態。曲名採自然數字排序，所以 Track 2 會排在 Track 10 前。播放清單排序不會改變已保存的曲目順序。點擊歌曲列即可播放；拖曳播放佇列中的歌曲列可調整順序。音訊取樣率達 48 kHz 且位元深度達 24 bit 時會顯示 Hi-Res 標誌。演出者索引會列出不重複歌曲數、專輯數與累計播放時長；點選演出者即可瀏覽其歌曲。
- **播放控制：** 提供隨機播放、上一首／下一首，以及循環關閉／全部循環／單曲循環。聆聽時長會保存，並影響新建立的隨機播放順序。Windows 支援系統媒體控制；Android 可透過媒體通知在背景播放。
- **正在播放與歌詞：** 可選 A（左側封面、右側歌詞）或 B（左側歌詞、右側封面）排列。顯示音檔內嵌封面，並優先讀取本機 LRC 或內嵌歌詞；也可搜尋網易雲與 QQ 歌詞候選、手動選擇結果，有時間資料時會同步捲動歌詞。譯文、羅馬拼音、歌詞外觀與同步延遲可在「設定」或「快速設定」調整。演出者名稱可用 `|`、`\`、`/`、`;`、`,` 分隔並點選以篩選曲庫；一般空白會保留在人名中。
- **歌曲資訊（Windows）：** 播放頁頂欄的資訊按鈕可開啟目前曲目的詳細資訊：完整路徑、音訊屬性，以及可編輯的標題、演出者、專輯、專輯演出者、作曲、類型、日期、音軌／碟號、內嵌歌詞與一張封面（取代、匯出或移除；可設定 JPG 品質與短邊自動縮放）。按「儲存」才會寫入檔案（MP3 一律存為 ID3v2.4）。「導入」可把目前使用中的線上歌詞放入歌詞欄位；「頻譜圖」會在獨立視窗顯示 FFT 頻譜圖。
- **外觀：** 可自訂背景與主色。主介面背景可選純色、單張圖片、幻燈片、目前專輯封面，或（僅 Windows）目前的 Windows 桌布。幻燈片會搜尋子資料夾，可依序或隨機播放，並每 1–120 分鐘切換。文字顏色可依背景自動切換；也可選擇自動調整圖片透明度。在此模式可選白字或黑字並設定清晰度，播放器會計算圖片透明度以符合目標亮度。主介面與播放頁封面外觀分開設定。Android 目前支援純色與專輯封面背景。
- **音訊選項（Windows）：** 支援播放 WAV、MP3、FLAC、AAC/M4A、Ogg Vorbis（`.ogg`／`.oga`）及 Matroska 音訊（`.mka`，可播放格式依檔案內的音訊編碼而定）。`.mp4` 與 `.mkv` 影片不會加入曲庫。在「設定 → 輸出」可選用 rubato FFT 取樣率轉換，或 Windows 內建轉換。DSEE HX 為選用功能，需要安裝 Sony Music Center；本專案不包含 Sony 專有濾鏡。
- **更新與開源致謝：**「設定 → 關於」顯示作者 [XPRAMT](https://github.com/XPRAMT)、建置資訊，以及開源套件版本、授權與官方連結。Windows x64 啟動時會檢查更新，並提供「忽略」（僅略過此次通知）、「自動重新啟動」或「下次啟動完成更新」；更新會保留 `UserData`。

## 在 Windows 使用

1. 下載並解壓縮[最新版 Windows 版本](https://github.com/XPRAMT/MoeMusicPlayer/releases/latest)，將 `moemusicplayer.exe` 與 `moemusicplayer-updater.exe` 放在一起。
2. 啟動播放器，到「設定 → 音樂來源」加入資料夾或播放清單。
3. 若要從檔案總管開啟音樂檔，先在「設定 → 關於」登錄支援的音訊格式，再到 Windows「預設應用程式」選擇 MoeMusicPlayer。開啟音檔會以同資料夾的支援音訊檔取代目前佇列，不會掃描子資料夾。

Windows 便攜版會將 `settings.json` 與 SQLite 曲庫保存在執行檔旁的 `UserData` 資料夾。請讓此資料夾與兩個執行檔放在一起；將執行檔移到其他資料夾會使用另一份資料。Android 則使用系統管理的應用程式資料目錄。

若要提供 Codex 可讀的診斷報告，請在終端機執行 `moemusicplayer.exe --diagnostics-json`。JSON 會包含建置與程序資訊、上次啟動時的資料庫／音訊狀態、檔案大小及最近的 panic 記錄；不包含音樂標籤、設定內容或 SQLite 資料列。報告仍會包含本機執行檔與資料路徑，分享前請先檢查。

## 在 Windows 開發

安裝 Node.js 20.19+（或 22.12+）、Rust 1.90+ 與 [Tauri 先決條件](https://v2.tauri.app/start/prerequisites/)，再執行：

```sh
npm install
npm run tauri -- dev
```

建置 Windows 便攜版執行檔：

```sh
npm run release:windows
```

執行前端檢查與 Rust 工作區測試：`npm run check`、`cargo test --workspace --locked --offline`。

固定輸出為 `release/moemusicplayer.exe` 與 `release/moemusicplayer-updater.exe`，請與 `release/UserData/` 放在一起。更新用檔案會打包到 `release/update-assets/`；打包不會發布版本。

## 建置 Android 版

安裝 Android SDK、NDK 與 Java 21，再執行：

```sh
npm install
npm run android:build:arm64
```

Debug APK 輸出至 `src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`。安裝後到「設定 → 音樂來源」加入來源，並授權 MediaStore 或 SAF 存取。
