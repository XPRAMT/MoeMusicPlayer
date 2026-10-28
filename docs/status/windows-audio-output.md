# 現況：Windows 音訊輸出

本文件是此領域「目前狀態」的快照，不是日誌；維護規則同根目錄 `AGENTS.md`「本文件維護規則」：就地改寫既有條目、不寫日期／量測數字／除錯經過，設計細節放 `docs/adr/`。

### Windows 音訊輸出（ADR 0020）

多裝置輸出（ADR 0020 P0）：`settings.json` 的 `outputDevices`；系統預設與自選裝置互斥（`active_devices`、`setRowEnabled`）；音訊端 `output_devices.rs`、`mirror.rs`，`RodioBackend::apply_devices` 比對差異；斷線以靜音心跳 `StallWatch` 偵測並重試。介面 `OutputDevicesSettings.svelte`＋`output-devices.js`（設定頁與快速設定共用；整張裝置卡片就是開關：點卡片空白處勾選／取消、卡片內控制項不觸發，原生 input 視覺隱藏但保留鍵盤與螢幕閱讀器，焦點框畫在卡片上，`output-device-card.js`），聲道映射是所有啟用裝置共用的一張矩陣，5 個方案槽（`outputPresets`）。裝置列的格式行在 Windows 與 Android 完全同一格式、只由 `src/lib/output-format.js` 產生（見 ADR 0020「Output path line and supported rates」）：播放中為「檔案取樣率 → 取樣率（步驟） → …．N 聲道」，步驟只有「DSEE HX」、「FFT」（rubato；rodio 備援轉換器為「基本轉換」並保留提示行）、「系統」（Windows 音訊引擎實際取樣率，交由 Windows 轉換時就只有這一步），例如「44.1 kHz → 48 kHz（FFT） → 48 kHz（系統）．2 聲道」，同步裝置以自己的系統取樣率結尾；獨佔為「96 kHz → 96 kHz（獨佔）．2 聲道．24bit．位元完美」（不是位元完美時省略最後一段、原因放提示行），DoP 為「DSD64 → 176.4 kHz（DoP） → 176.4 kHz（獨佔）．…」；閒置列列出全部可開啟的取樣率與聲道、不寫位元深度「44.1, 48, 88.2, 96 kHz．2 聲道」：共享混音取樣率加上獨佔 `IsFormatSupported` 接受的常見取樣率（`output_devices::supported_rates`／`device_supported_rates`，只由 `audio_list_output_devices` 在背景執行緒探測，依裝置快取到裝置通知或混音格式改變；探測失敗只列混音取樣率），經裝置清單的 `supportedRates` 送出。音量是 Windows 端點真實音量（`crates/player-platform-windows/src/endpoint_volume.rs`），不是對音訊乘增益；音量同步（`outputVolumeSync`）由後端 `src-tauri/src/volume_sync.rs` 隨時執行。

取樣率轉換一律 rubato FFT（使用者決定移除選項；設定頁該區只剩「DSEE HX」）；`settings.json` 的 `resamplingMode` 保留相容但不讀；引擎的 `ResamplingMode::WindowsBuiltin` 已無入口。不做裝置間漂移補償（理由見 ADR 0020；量測工具 `measure_real_device_clocks`，`MOE_CLOCK_DEVICES`／`MOE_CLOCK_SECONDS`）。

獨佔模式（ADR 0020 P1，模式鈕「獨佔優先」）：`outputExclusive`（`settings_get/set_output_exclusive`），只播第一台啟用裝置；實作 `crates/player-audio-windows/src/exclusive/`。開不起來時該曲以共享備援播放並說明原因（`exclusiveFallback`），下一首重試；勾「系統預設」時跟隨預設裝置，指定裝置時不跟隨；暫停立即釋放裝置，閒置 10 秒（`IDLE_RELEASE`）釋放。裝置通知 `device_notify.rs` → `audio-devices-changed`。實機測試（`--ignored`，靜音）：`real_device_*`、`real_default_device_is_followed`。沒有 DAC，無聽感驗證；ASIO 未做。
