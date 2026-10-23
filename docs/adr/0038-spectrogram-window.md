# 頻譜圖視窗

Status: 已採納

## Context

頻譜圖視窗（含 A/B 比較、配色、計算方式）原本只記在 AGENTS.md 與 ADR 0013 的片段。此 ADR 記錄其目前設計。

## 目前設計

頻譜圖：獨立 Tauri 視窗（label `spectrogram`，`?view=spectrogram&track=` 掛 `SpectrogramView`，重複開啟以 `spectrogram-track` 換曲；Android 為全螢幕頁）。可開啟 A／B 任意音訊檔（`spectrogram_pick_file`／`spectrogram_render_file`）或拖放；A/B 比較有並排、疊加（切換、空白鍵、B 透明度、差異圖）與分割線，共用座標軸與設定、縮放平移同步（觸控 `pinchView`），比較偏好存 `moe.spectrogramCompare`（`src/lib/spectrogram-compare.js`）。色階條在右側（直版視窗改在下方，`portrait`）；配色 Spek（依 Spek 原始碼核對色階後重新實作 `spek_color`，不複製其 GPL 程式碼）／Fire／Magma／Inferno／Viridis／灰階；計算對齊 Spek（像素欄為多個不重疊 FFT 的 dB 平均、`20·log10(|X|/N)`、預設 FFT 4096，截止提示門檻 −112 dB）；色階上限固定 0 dB（舊 `dbCeiling` 忽略），下限可調；頻率軸線性到取樣率一半；聲道預設左聲道。設定存 `moe.spectrogramSettings.v2`（`src/lib/spectrogram-settings.js`）。工具列依實際寬度換行，預設只顯示第一行，▼／▲ 展開後續行（按鈕 grid 置中、左右內距 0），收合項目設為 inert，縮放與比較模式增減控件時重新量測；雙擊圖面或按 0 重置縮放。DSF 與 WVDSD 以 `player-tags` 的 `dsd_input`（`DsdPcmReader`）固定轉 352.8 kHz、0 dB 增益後分析（只支援 ≤ 2 聲道、44.1 kHz 系列；DFF 不支援）；Opus 以 `OpusReader`（Android 不支援 Opus 頻譜）。Release GUI 驗收待做。
