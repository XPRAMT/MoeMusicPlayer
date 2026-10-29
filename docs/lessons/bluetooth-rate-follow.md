# 經驗：Android 藍牙取樣率跟隨、混音器取樣率、編碼優先順序

決策見 ADR 0048（真實混音器取樣率）、ADR 0051（藍牙取樣率跟隨）、ADR 0052（編碼優先順序）；現況見 `docs/status/android-platform.md`。

## 最終做法

- 混音器取樣率：有 root 或已授權 Shizuku 時，`MixerRateLookup` 以 `dumpsys media.audio_flinger`（失敗再 `media.metrics`）讀播放軌所在執行緒的實際取樣率；第一次立即查、找不到就短退避重試，限速＋斷路器＋開關。
- 取樣率跟隨：以 `app_process` 跑的特權 helper（`BluetoothRateHelper`）呼叫 `setCodecConfigPreference`，每次請求帶「目前的編碼」＋`CODEC_PRIORITY_HIGHEST`，只改取樣率；`pickRate` 依規則選編碼提供的取樣率。

## 修復時間線（只留轉折）

| commit | 改了什麼 | 為什麼前一版不行 |
| --- | --- | --- |
| `d406e288`、`59c167c2` | 第一版：dump 讀混音器取樣率；Windows 編譯修正（啟動呼叫只限 Android） | Android 專用呼叫沒加 cfg，Windows 編譯失敗 |
| `6b065443` | 修 flinger dump 解析：執行緒區塊內的不縮排行只結束軌表；`Type` 欄可為空；`pid/ uid` 被拆成兩個 token；metrics 值結尾的 `)}` | 依測試樣本寫的解析器遇到真實 A2DP 擷取就讀不到軌表，記成 `unknown` |
| `41e28aff` | helper 自己準備 main looper、從 `ActivityThread.systemMain()` 建 system context、把 service manager 交給藍牙模組；錯誤帶步驟名 | 裸 `app_process` 沒跑 `ActivityThread.main`，`BluetoothAdapter.getDefaultAdapter()` 回 null（「no Bluetooth adapter」） |
| `cffffc58`、`8f6df790` | 跟隨：最小間隔 1 s、合併請求、開啟時立即套用目前曲目、還原等編碼廣播；查詢：第一次立即、150/300/600/1200 ms 退避、1 s 間隔＋每分鐘 30 次上限、設定頁每 500 ms 讀快取 | 固定 1 s settle＋5 s 間隔讓值晚約 3.4 秒才出現，而 dump 本身只要約 20 ms |
| `dfd15d34` | 取樣率請求帶目前編碼＋最高優先 | 複製到的 `codecPriority` 是 0（沒有使用者偏好），系統約 1.5 秒後改選自己偏好的 aptX Adaptive 96 kHz，請求的取樣率丟失 |
| `b6543a4e` | 編碼未知時主動用 helper `state` 讀 | 編碼廣播不是 sticky，App 啟動、服務重啟或耳機早已連線時收不到，路徑列只顯示「（藍牙）」 |
| `63263316`、`28822729` | DSD 轉 PCM 以轉換器的 176400 Hz 跟隨，只略過原生 DSD；曲目高於所有可選取樣率時選最高的 | 原本略過所有 DSD，編碼停在上一首的取樣率；176.4 kHz 選「最近」會落到不合理的值 |

反覆修的根本原因：規則先依文件與想像寫，真實裝置（NX809J＋WF-1000XM6）的 dump 格式、藍牙堆疊的優先權行為、`app_process` 的初始化、廣播時機都和假設不同。每個修正都來自一次實機 log。這類工作預期要磨合，重點是每次都把實機證據寫進 ADR 和測試樣本。

## 邊界情況

- 這支 ROM 每個取樣率開一個 A2DP 混音執行緒，同一時間不同軌的執行緒取樣率可以不同。
- flinger dump 的執行緒區塊內有不縮排的行（「Bluetooth latency modes are enabled」等）；一般軌的 `Type` 欄是空的；`Client(pid/uid)` 印成兩個 token。
- `media.metrics` 的 `audio.track.<portId>` → `threadId`，`audio.thread.N` 的 `readParameters` 才有 `sampleRate`；`audiothread` 摘要是已銷毀的舊執行緒。
- `codecPriority` 0 代表沒有使用者偏好，堆疊會重選編碼。請求一律帶目前編碼＋最高優先，副作用是釘住目前編碼。
- Qualcomm aptX 服務（`com.qualcomm.qtil.aptxals`）也會呼叫 `setCodecConfigPreference`；不嘗試控制它。
- 編碼廣播不是 sticky；LDAC `codecSpecific1` 0 是「堆疊預設」，不是自適應。
- DSD 轉 PCM 的取樣率是 176400 Hz（`BluetoothRateRules.DSD_PCM_OUTPUT_HZ`），不是曲庫記的 DSD 位元率；只有原生 DSD 直通略過。
- `pickRate`：完全相符 → 高於所有可選則取最高 → 同家族（44.1k／48k）最近 → 整體最近。
- Shizuku 程序的 `exitValue()` 問題見 `android-privileged-permissions.md`；helper 啟動約 1 秒。
- Android 13 以前沒有相關編碼類別，直接略過；USB 獨佔路由也略過。

## 已排除的做法（不要再試）

- 固定 settle 延遲與長間隔：只讓值變晚，不保護任何東西。
- 複製目前設定（含 `codecPriority` 0）只改取樣率。
- 只靠編碼廣播得知目前編碼。
- 略過所有 DSD 曲目。
- 輪詢 dump：狀態讀取只回快取值。

## 驗證方式

- 自動測試：Kotlin `BluetoothRateRulesTest`（`rateRequest` 保留編碼、最高優先；`pickRate`；`trackRate` DSD）、`BluetoothCodecRulesTest`、`BluetoothLdacModeReaderTest`、`MixerRateRules`／dump 解析測試（以真實擷取為樣本）；JS `node --test tests/mixer-rate-source.test.mjs tests/output-format.test.mjs tests/android-output-devices.test.mjs`。
- helper 手動診斷（adb、root；步驟見 ADR 0051「Diagnostic by hand」）：`state <address> 0` 應印 `INFO codec=... sampleRate=... selectable=...` 與 `RESULT STATE ...`；失敗是 `RESULT ERR <step>: <text>`。
- 實機：開取樣率跟隨，依序播 44.1 kHz、96 kHz、DSD（轉 PCM）、192 kHz 曲目，設定頁路徑列的編碼取樣率依 `pickRate` 改變、編碼不跳；logcat 看到請求確認（沒有「not confirmed within 3000 ms」）。混音器取樣率在 AudioTrack 建立後約 20–50 ms 內出現。
- 注意：使用者正在聽音訊測試時，不要在裝置上跑 `dumpsys media.audio_flinger` 這類重型 dump（AGENTS.md 硬性規則）。

## 下次改這塊前必看

1. 解析系統輸出前先取一份真實擷取，加進測試樣本。
2. 任何 `setCodecConfigPreference` 請求都要想清楚 `codecPriority`。
3. 延遲與限速只為保護裝置而設，要寫明保護的是什麼。
4. Android 專用的呼叫在 Rust 端要加平台 cfg，改完至少確認 Windows 能編譯（`cargo check`）。
5. 未在實機驗證的項目寫在 ADR 的「Not verified」，不要當成已完成。
