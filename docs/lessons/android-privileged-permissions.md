# 經驗：Android root／Shizuku 權限與 USB 獨佔權限流程

決策見 ADR 0047（USB 獨佔「Setting, page and permission」）、ADR 0048（混音器取樣率、共用權限面板、Shizuku runner）、ADR 0051、ADR 0052；現況見 `docs/status/android-platform.md`。

## 最終做法

- USB 獨佔：選「獨佔」立即生效，不經我們的面板；由 App 自己在前景時跳系統 USB 權限對話框，決策是純函式 `UsbExclusiveRules.decideAsk`，前景由系統 `getMyMemoryState` importance 判斷，拒絕依裝置記到拔除為止。
- root／Shizuku：三個功能（真實混音器取樣率、藍牙取樣率跟隨、藍牙編碼優先順序）共用一個面板「需要 root 或 Shizuku」；入口一律「前往授權」，面板內按鈕「授權」；權限一取得就隱藏入口並自動關閉面板（全 App 規則：授權提示只在缺權限時出現）。

## 修復時間線（只留轉折）

| commit | 改了什麼 | 為什麼前一版不行 |
| --- | --- | --- |
| `64ddbc20` | 只在接著的 USB 裝置缺權限時才開說明面板（`exclusiveSwitchNeedsPanel`） | 原本每次按「獨佔」都開面板 |
| `c0114a24` | 拿掉面板，直接跳系統 USB 權限對話框；拒絕依裝置記住 | 選模式本身就是使用者的請求、系統對話框就是同意，中間的面板是多餘的一步（ADR 0039 面板規則不套用在這個權限） |
| `02c0c4fc` | 前景改問系統 process importance（≤ `IMPORTANCE_FOREGROUND`），每次檢查寫一行決策 log | 自己數 Activity 的前景次數會錯；只有播放服務在跑（125）或可見但不在最上層（200）時跳的對話框使用者無法回應 |
| `a17dce43`、`97348757`、`a22a4e8b` | 系統取樣率權限列移到「裝置」區；只在缺權限時顯示；寫成全 App 規則 | 已授權仍顯示權限列與說明 |
| `ddd1d16d` | 三個功能合成一個面板、一個藍牙提示，文字統一「前往授權」／「授權」 | 三處各自的提示與「設定」按鈕，位置與條件不一致 |
| `2ee3c7d5` | Shizuku runner：exit code 由獨立執行緒的 `waitFor()` 取得；runner 自己的錯誤不計入斷路器 | Shizuku 遠端程序執行中呼叫 `exitValue()` 會丟 `IllegalArgumentException`（不只 `IllegalThreadStateException`）；藍牙 helper 啟動約 1 秒，每次都失敗，三次後斷路器跳開到 App 重啟 |
| `1e5a8ce2` | root 或 Shizuku 可用時面板自動關閉（`mixerPanelShouldClose`） | 授權完成回到 App 面板還開著 |

反覆修的根本原因：一開始沒先定「何時、在哪、問誰」的單一規則，每個功能各自決定提示位置與條件；USB 權限則在「面板 vs 系統對話框」與「如何判斷前景」上換了三次。最後收斂成兩條規則：同意由系統對話框代表；提示只在缺權限時出現。

## 邊界情況

- 背景跳的 USB 權限對話框沒人能回應；要在 App 回到前景時（`onActivityResumed`）再檢查一次。
- 只跑播放服務的程序 importance 是 125，可見但不在最上層是 200，都不算前景。
- 拒絕（或對話框開到逾時）要記住到裝置拔除或程序重啟，否則每次檢查都再跳一次；「前往授權」可手動再問。
- 沒有 USB 音訊裝置時選獨佔立即生效（等接上再問）。
- 狀態輪詢絕不可觸發權限請求；Shizuku 與 root manager 的對話框只能由按鈕啟動。
- root 要使用者按過「檢查 root」才探測 `su`（避免每次啟動跳 root 授權）。
- Shizuku 程序 `exitValue()` 在執行中丟的例外類型和本機程序不同。
- 斷路器是程序層級的，跳開後要重啟 App 才恢復；runner 自己的錯誤不能計入。
- 「tripped」與「disabled」狀態的面板只能手動開關，不可自動關閉。

## 已排除的做法（不要再試）

- 按「獨佔」先開我們的說明面板再請求 USB 權限：多一步，且與系統對話框重複。
- 自己計數前景 Activity：與系統認定不一致。
- 已授權仍顯示「已授權」狀態列或說明。
- 每個 root／Shizuku 功能各自的提示與按鈕。
- 在 runner 讀完輸出流後呼叫 `exitValue()`。

## 驗證方式

- 自動測試：Kotlin `UsbExclusiveRulesTest`（`decideAsk` 各原因、前景 importance、路由表）、`MixerRateRules` 與 `BoundedProcess` 相關測試（Android Gradle 單元測試）；JS `node --test tests/android-output-devices.test.mjs tests/mixer-rate-source.test.mjs`。
- 實機（USB 獨佔）：接 USB DAC、選獨佔，應立即跳系統對話框；拒絕後不再自動跳，卡片出現「前往授權」；拔掉重接會再問一次；App 在背景時接上不跳，回前景才跳。每次檢查看 `adb logcat -d | grep "USB exclusive: auto-ask check"` 的決策行（`-> ask|skip (<reason>)`）。
- 實機（root／Shizuku）：沒授權時「裝置」區與「藍牙」區各只有一個「前往授權」；開面板按「授權」完成後回到 App，面板自動關閉、入口消失。Shizuku 路徑要實際跑一次藍牙 helper，確認不再出現 "process hasn't exited"、斷路器沒跳。

## 下次改這塊前必看

1. 新增需要權限的功能：提示只在缺權限時出現，狀態改變（回 App、授權完成）時即時重算。
2. 需要 root 或 Shizuku 的功能一律接到同一個面板，不另做提示。
3. 系統對話框只在真正前景時跳，前景問系統，不自己數。
4. 改 Shizuku／root 執行路徑時，確認錯誤分類：只有真實裝置失敗才計入斷路器。
5. 使用者測音訊時不要在裝置上跑重型 dump（AGENTS.md 硬性規則、ADR 0048）。
