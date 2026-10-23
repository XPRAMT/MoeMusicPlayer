# 歌詞視窗改用原生 DirectComposition 繪製（2026-10-23 起合成層改為 Windows.UI.Composition，見文末）

狀態：已採納（2026-10-23，使用者決定「歌詞繪製採用原生視窗方案 不再使用 WebView2 方案」）

日期：2026-10-23，Asia/Taipei (UTC+08:00)

相關：[ADR 0018](0018-taskbar-lyrics.md)、[ADR 0031](0031-multi-monitor-taskbar-lyrics.md)、[ADR 0032](0032-floating-lyrics.md)、[ADR 0028](0028-frontend-playback-clock.md)

## 背景

使用者希望所有歌詞動畫以螢幕的完整更新率執行（120 Hz 螢幕 120 fps、60 Hz 螢幕 60 fps），而且主視窗、每條工作列各一個的工作列歌詞視窗（本機 2 條）與浮動歌詞可同時開啟。另一位代理以 CDP 對 Release 播放器量測：所有 WebView2 視窗共用一個 GPU 行程，該行程每個 vsync 合計最多約 3 次有損壞區的 present（120 Hz 時約每秒 340–360 次），4 個動畫中的視窗在 120 Hz 時各掉到約 90 fps；60 Hz 時使用者看得到頓挫。

本文件研究把 Windows 的歌詞視窗（工作列歌詞、浮動歌詞）改成原生繪製，讓動畫不經過 WebView2 GPU 行程。只做研究與獨立原型，沒有整合進 App。

## 候選技術比較

| 項目 | DirectComposition + Direct2D/DirectWrite | Windows.UI.Composition（`CreateDesktopWindowTarget`） | 翻頁模式 DXGI swap chain（premultiplied alpha，`CreateSwapChainForComposition`） |
|---|---|---|---|
| 動畫在合成器（DWM）上跑、與我們的執行緒無關 | 可以：`IDCompositionAnimation`（分段三次多項式＋`AddRepeat`）可綁到 OffsetX/Y、不透明度（effect group）、transform、clip。DWM 在每個合成幀計算值，我們的行程不必每幀工作。 | 可以，而且更豐富：`KeyFrameAnimation`、`ExpressionAnimation`（可用運算式表示「依時間的位置」）、`CompositionScopedBatch`。 | 不行：swap chain 只是內容；位移動畫要嘛另外疊 DComp/WinRT visual，要嘛自己每幀 Present（等於回到「我們自己每幀畫」）。 |
| 每台螢幕完整更新率 | 可以：動畫由各螢幕自己的合成幀驅動（實測見下）。 | 同左（底層同一個 DWM 合成器）。 | 自己 Present，要對每台螢幕的 vblank 節奏（`DCompositionWaitForCompositorClock` 或 waitable swap chain）。 |
| 透明、點擊穿透、置頂、`Shell_TrayWnd` 擁有、不搶焦點 | 可以（原型已驗證）：`WS_EX_NOREDIRECTIONBITMAP | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`、`WS_POPUP`、建立時指定擁有者；`WindowFromPoint` 不會回傳原型視窗。 | 可以（相同的 HWND 樣式；需要 `DispatcherQueueController` 與 WinRT 執行緒）。 | 可以（搭配 DComp target）。 |
| 混合 DPI | 可以：程序設 Per-Monitor V2，每個視窗依自己螢幕的 DPI 以實體像素繪製；換 DPI 時（`WM_DPICHANGED`）重新排版重畫一次。 | 同左。 | 同左。 |
| 文字效果 | D2D Shadow 效果（標準差＝0.3 em，與 CSS `text-shadow` 0.6 em 半徑等價）、自訂 `IDWriteTextRenderer` 取字形輪廓做 0.08 em 描邊（paint-order stroke fill）、`D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT` 彩色 emoji、`IDWriteFontFallbackBuilder` 對應 `'Segoe UI', 'Microsoft JhengHei'`。全部在原型實作。 | WinRT 本身不畫文字，仍要用 D2D/DWrite 畫進 `CompositionDrawingSurface`（經 `ICompositionDrawingSurfaceInterop`）；陰影可用 `DropShadow`（只吃遮罩，對文字要多一步）。 | 同 D2D。 |
| 依賴 | `windows` crate 已有的模組；無新增第三方相依。 | 需要 WinRT 專案元件（`Windows.UI.Composition` 系列 feature）、DispatcherQueue；行為與 DComp 相近但多一層 interop。 | — |

結論：DirectComposition + D2D/DWrite 最合適。WinRT Composition 能力相同但多一層 interop，且文字仍要 D2D；swap chain 只在要每幀重畫內容時才需要，而本案的動畫只是「已畫好的文字在動」，不需要。

## 原型

位置：`crates/lyrics-native-proto/`（獨立 crate，有自己的 `[workspace]` 與 `Cargo.lock`，不屬於 App workspace，不改根目錄 `Cargo.lock`；`windows` 0.62.2，與 App 相同版本，離線建置）。

```
cd crates/lyrics-native-proto
cargo build --release --offline
target\release\lyrics-native-proto.exe --windows 3 --mode anim --seconds 15 --effect shadow --owner-taskbar
powershell -File measure.ps1 -ProtoArgs "--windows 3 --mode anim --seconds 12 --owner-taskbar"
```

- 視窗：第 1、2 個放在第 1、2 台螢幕的工作列區（`--owner-taskbar` 時擁有者為該螢幕的 `Shell_TrayWnd`／`Shell_SecondaryTrayWnd`），其餘為主螢幕上的浮動視窗（32 DIP 字）。
- 內容：10 行中文／日文（含假名、漢字、`🌙 ✨ 🎧` 彩色 emoji）與譯文，兩列；以 U+3000（1 em）間隔接成連續條，譯文同寬格。
- 動畫規則與 `lyrics-strip.js` 相同：目前行開始時左緣在歌詞區中心，線性滑到下一行開始時下一行左緣抵達中心；不透明度：目前行 1，其餘 0.5（非目前透明度預設），換行前 200 ms 線性交叉淡化、剛好在下一行開始時完成。整段 31.3 秒，`AddRepeat` 無限循環。
- 三種模式：
  - `anim`：位移與每行不透明度全是 `IDCompositionAnimation`，提交一次後由 DWM 執行，本行程只有訊息迴圈。
  - `commit`：每個合成器時脈（`DCompositionWaitForCompositorClock`）由本行程算出位移與不透明度後 `SetOffsetX2`／`SetOpacity2`＋`Commit`（不重畫點陣）。
  - `redraw`：每幀清空並重新點陣化可見的行（含陰影模糊），類似 WebView 的最壞情況。
- `--shot <前綴>`：3 秒時以螢幕 DC 擷取各視窗區域（BMP），用來確認實際畫面。
- 已確認：透明背景、點擊穿透（`WindowFromPoint` 不命中）、擁有者為工作列、主螢幕 240 DPI 字級 29.1 px（工作列高 80 px 內兩列）、副螢幕 192 DPI 23.3 px、中日文與 emoji 正常、交叉淡化可見。

## 量測

環境：RTX 5070 Ti；DISPLAY1 3840×2160、240 DPI、**120 Hz**（主）；DISPLAY6 2560×1440、192 DPI、**60 Hz**（VDD 虛擬顯示器，IddCx）。Release 播放器同時執行中（主視窗、2 個工作列歌詞、1 個浮動歌詞，CDP 埠由另一位代理開啟），沒有關閉或終止播放器與 msedgewebview2。DWM 數字取自 `DCompositionGetStatistics`／`DCompositionGetTargetStatistics`（每台螢幕一個 target：presents＝DWM 實際送出的合成幀，refreshes＝vblank），CPU 取程序 `TotalProcessorTime` 差值（單核百分比；dwm.exe 無權限讀取），GPU 為效能計數器 `GPU Engine(*engtype_3D)` 依 pid 加總。每項 12 秒、陰影效果。

| 情境 | 原型 CPU | 主螢幕 DWM presents / refresh（120 Hz） | 副螢幕 DWM presents / refresh（60 Hz） | dwm GPU 3D | 原型 GPU 3D |
|---|---|---|---|---|---|
| 不開原型（基準） | — | 12.3 / 119.8 | 4.1 / 59.9 | 0.2 % | — |
| `anim` 1 視窗（主螢幕工作列） | 0.13–0.19 % | 120.0 / 120.0 | 8.3 / 60.0 | 3.6 % | 0 |
| `anim` 3 視窗 | 1.0 % | 119.8 / 119.8 | 69.9 / 59.9 ※ | 5.3 % | 0 |
| `commit` 3 視窗（本行程每幀提交） | 0.2–0.4 %（自己 120 次提交/秒，最長間隔 9.0 ms） | 120.0 / 120.0 | 67.6 / 59.9 ※ | 4.8 % | 0 |
| `redraw` 3 視窗（每幀重畫含模糊） | 41.5 %（只到 30 幀/秒，最長間隔 37.7 ms） | 76.3 / 120.1 | 34.1 / 60.0 | 1.7 % | 1.4 % |

※ 虛擬顯示器的 presents 略高於 refresh，推測是 IddCx 驅動的計數方式；重點是每個 vblank 都有合成幀。

- 基準時 DWM 只在有變化時合成（主螢幕 12 幀/秒）；開 `anim` 後，有動畫的螢幕每個 vblank 都合成一次（120 Hz → 120、60 Hz → 60），也就是動畫以兩台螢幕各自的完整更新率執行，而本行程幾乎不用 CPU。
- 合成在 dwm.exe 裡，與 WebView2 GPU 行程無關；原型不產生任何 swap chain present。
- 播放器各 WebView 頁面的 rAF（CDP 唯讀量測 4 秒）：不開原型、開 3 視窗、開 6 視窗都是主視窗 120.2、工作列歌詞 120.4、浮動歌詞 120.5 fps，最長間隔約 8.6–8.9 ms，沒有變化。**注意**：rAF 不等於實際 present；量測時播放器歌詞是否在連續動畫中未確認，WebView 的 present 數由另一位代理量測。
- `redraw` 證明「每幀重新點陣化含模糊的文字」在 CPU 上很貴（Direct2D 每幀重建陰影效果、240 DPI 大字），原生實作不能這樣做；應該像 `anim` 只在換歌、改設定或換 DPI 時畫一次。

## 畫質

- DirectWrite 灰階反鋸齒（透明表面不能用 ClearType，WebView2 透明視窗同樣是灰階），在 240／192 DPI 下以實體像素繪製，字形清晰度與 WebView2 相當；字型對應 `'Segoe UI', 'Microsoft JhengHei'` 後外觀接近（拉丁用 Segoe UI，CJK 與假名用正黑體，emoji 由系統備援到 Segoe UI Emoji）。
- 位移預設最近鄰取樣（每幀整數像素位移，120 Hz 時每幀約 1–3 px，不糊）；`--linear` 改雙線性取樣可做次像素位移，但移動中文字會略軟。沒有做肉眼並排比較。
- 陰影：D2D Shadow（高斯，σ＝0.3 em，顏色＝反色、α 0.75）與 Chromium 的 `text-shadow` 演算法不同，但數學上同樣是高斯模糊，外觀應接近；未做像素差異比較。

## Rust 可行性與注意事項

- 需要的 `windows` 0.62.2 feature：`Win32_Graphics_DirectComposition`、`Win32_Graphics_Direct2D`、`Win32_Graphics_Direct2D_Common`、`Win32_Graphics_DirectWrite`、`Win32_Graphics_Direct3D`、`Win32_Graphics_Direct3D11`、`Win32_Graphics_Dxgi`、`Win32_Graphics_Dxgi_Common`、`Win32_Graphics_Dwm`（量測用）、以及已有的 `Win32_UI_WindowsAndMessaging`、`Win32_UI_HiDpi`、`Win32_Graphics_Gdi`、`Win32_Foundation`。另需 `windows-core` 的 `#[implement]`（已在 `player-platform-windows` 使用）實作 `IDWriteTextRenderer` 取字形輪廓。都是同一個 crate 的 feature，不新增第三方相依，不涉及 GPL。
- 執行緒：所有歌詞視窗放在**一條**專用 UI 執行緒（D2D 單執行緒 factory、一個 D3D11 裝置、一個 `IDCompositionDesktopDevice`、每個視窗一個 target），自己的訊息迴圈。不要放在 Tauri 主執行緒。DComp/D2D 不需要 COM STA（`D3D11CreateDevice`、`D2D1CreateFactory`、`DWriteCreateFactory` 都不需 `CoInitialize`）；若用 WinRT Composition 才需要 DispatcherQueue。
- 裝置遺失：顯示卡驅動更新或 TDR 時 D3D 裝置失效；`IDCompositionDevice::CheckDeviceState` 或 `BeginDraw` 回 `DXGI_ERROR_DEVICE_REMOVED` 時要重建裝置、surface 並重新提交動畫（資料都在記憶體，重建代價小）。原型未處理。
- 不需要 `DwmFlush`；`commit` 模式用 `DCompositionWaitForCompositorClock`（Windows 11）就能與合成器同步。`anim` 模式完全不用等。
- 時間基準：DComp 動畫的時間 0 是「該次 Commit 被合成器套用的時刻」，所以播放位置對應要在提交時帶入目前位置（從目前位置開始的分段多項式），跳轉、暫停、繼續時重建動畫（與目前 CSS 動畫的重排一樣，事件驅動，不每幀）。暫停時改成常數值。
- 紋理：每行一張 surface（只需畫可見範圍加左右各一個歌詞區寬，沿用 `lyricsStripWindow` 的視窗化規則）；D3D11 紋理寬度上限 16384 px，不能把整首畫成一張。
- `WS_EX_LAYERED` 與 DComp 搭配可正常顯示（原型已驗證），點擊穿透因此不需要額外的 `WM_NCHITTEST` 技巧；浮動歌詞「固定／移動」模式的切換改成切換 `WS_EX_TRANSPARENT`，滑鼠位置監看沿用既有 100 ms 執行緒。

## 若採納的實作計畫

1. **資料流**：主視窗已把歌詞（`taskbar-lyrics-data`）、播放取樣（`taskbar-lyrics-playback`）、設定與深淺色以事件送到歌詞視窗。改為頁面把同一份 payload 用 Tauri 命令送到 Rust（或直接由 Rust 端的播放快照提供時間），Rust 的原生歌詞執行緒（新模組，例如 `player-platform-windows/src/lyrics_native/`）以 channel 接收：歌詞行（已依偏好轉好繁簡、含偏移）、播放位置＋取樣時間＋播放狀態、外觀設定、非目前透明度。選句、連續條偏移、權重沿用現有純函式規則，在 Rust 重寫一份（`lyrics-strip.js` 的 `stripOffset`／`lyricsLineWeight`；Android 已有 Kotlin 版 `FloatingLyricsRules`，三份需用同一組測試向量對照）。
2. **繪製**：換歌、換設定、換 DPI、視窗寬改變時畫各行 surface；播放中只在跳轉／暫停／繼續／換行窗（視窗化範圍改變）時重建 DComp 動畫並 Commit。單行模式（非連續）的長句捲動同樣用 OffsetX 動畫；換行直接切換內容。
3. **效果對齊**：陰影（D2D Shadow，σ 0.3 em）、描邊（字形輪廓 0.08 em、α 0.5、反色、先描邊後填色）、整窗透明度（root effect group）、非目前透明度（每行 effect group）、連續模式選陰影時改描邊的既有規則照搬；emoji 彩色字型；字型備援表與 app.css 一致。
4. **視窗管理**：沿用 `taskbar_lyrics.rs`／`floating_lyrics.rs` 的位置、擁有者、遮蓋判斷與熱插拔邏輯，只把「建立 Tauri WebviewWindow」換成「請原生執行緒建立／移動／銷毀 HWND」。浮動歌詞的鎖頭按鈕與移動模式拖曳要以原生方式實作（`WM_NCHITTEST` 回 `HTCAPTION`、自畫鎖頭圖示）。
5. **多螢幕／DPI**：每個視窗以自己螢幕的 DPI 繪製，`WM_DPICHANGED` 時重畫；動畫天然以各螢幕更新率執行。
6. **Android**：不受影響（浮動歌詞已是原生 `FloatingLyricsOverlay`）。
7. **過渡**：保留 WebView 版作為設定或建置旗標的後備，直到原生版驗收完成。

粗估工作量：原生繪製與動畫模組 4–6 天、浮動歌詞互動（拖曳、鎖頭、面板設定對應）2–3 天、規則移植與對照測試 1–2 天、多螢幕／DPI／裝置遺失與實機驗收 2–3 天，合計約 2 週。

## 風險

- 兩套繪製（WebView 主視窗與原生歌詞）的外觀需持續對齊；新增歌詞外觀選項時要改兩邊。
- 規則邏輯從 JS 複製到 Rust（加上既有 Kotlin），三份需要共用測試向量避免偏離。
- DComp 動畫的時間基準是合成器套用時刻，與播放時鐘之間有約一幀的相位誤差；跳轉／暫停須立即重建。
- 裝置遺失、Explorer 重啟、熱插拔要自己處理（Tauri 視窗原本會替我們處理部分生命週期）。
- 位移取樣模式（最近鄰 vs 雙線性）對移動中文字的觀感需使用者實機判斷。
- 量測的 DWM 合成幀數證明合成在每個 vblank 發生，但沒有用 PresentMon／擷取逐幀驗證文字確實每幀移動（dwm 在每幀計算動畫值是 DComp 的設計）。

## 未驗證

- 沒有 PresentMon／ETW（未安裝，未下載），WebView2 的實際 present 數沒有在本研究中量測；dwm.exe CPU 無權限讀取。
- 沒有與 WebView 版做肉眼或像素並排比較；60 Hz 虛擬顯示器上的肉眼流暢度未由使用者確認。
- 裝置遺失恢復、`WM_DPICHANGED`、拖曳與鎖頭互動、工作列自動隱藏與全螢幕遮蓋都未在原型實作。
- 描邊用的自訂文字渲染器對彩色 emoji 只描外框輪廓（單色），與 Chromium 的 `-webkit-text-stroke` 對 emoji 的表現可能不同。

## 決策與最終設計（2026-10-23）

Windows 的工作列歌詞（每條工作列一個）與浮動歌詞改為原生繪製，WebView2 歌詞視窗（`?view=taskbar-lyrics`、`?view=floating-lyrics`、`TaskbarLyricsView.svelte`、`FloatingLyricsView.svelte`）已刪除。Android 浮動歌詞不變。

- **規則**：`crates/player-lyrics-rules`（選句、`display_offset_ms`、連續條排版／位移／視窗化、末句 500 ms／字元叢集、200 ms 交叉淡化與第一行前導、非目前透明度、「無」模式長句捲動 90%）。`tests/fixtures/lyrics-rules-vectors.json` 由 `scripts/generate-lyrics-rules-vectors.mjs` 以 JS 規則（`lyrics-strip.js` 等，作為參考實作保留）產生，Rust（`tests/shared_vectors.rs`）、node（`tests/lyrics-rules-vectors.test.mjs`）、Kotlin（`LyricsRulesVectorsTest`）都對照同一份；Android 與 Windows 已知不同的前導情況以 `android: false` 標記。
- **繪製**：`crates/player-lyrics-native`。一條 UI 執行緒（`moe-lyrics-native`，Per-Monitor V2）持有 D3D11（失敗退 WARP）、D2D、DWrite（Segoe UI＋Microsoft JhengHei＋系統備援、字重 500、行框依 CSS 規則置中）、DComp 裝置；每個視窗一個 target。每行（連續模式每格含譯文）點陣化一次並快取（鍵含文字、字級、顏色、效果）；陰影＝D2D Shadow σ 0.3 em、反色 α 0.75，描邊＝字形輪廓 0.08 em、反色 α 0.5、先描邊後填色。**連續模式選陰影時照樣畫陰影**（使用者 2026-10-23 決定：陰影已在快取點陣裡，沒有每幀成本；`native_text_effect`），不再改描邊；Android 仍改描邊。
- **動畫**：`plan.rs` 由規則產生分段線性關鍵格：連續條位移（涵蓋接下來 3 個換行點）、每格不透明度（在各行開始與淡化起點取樣，保證與規則一致，單元測試逐 10 ms 比對）、「無」模式長句捲動。以 `IDCompositionAnimation`（`AddCubic`＋`End`）交給 DWM；位移視覺用最近鄰取樣（整數裝置像素）。只在換行點、跳轉、暫停／繼續、播放校正、新歌詞、設定、大小或 DPI 改變時重排（`MOE_LYRICS_NATIVE_LOG=<檔案>` 會記錄每次重排與原因）。
- **播放校正不跳動（2026-10-23）**：舊版取樣與推算差 ≥ 30 ms 就把時鐘直接設成取樣值並重排，連續條會往前或往後跳（差×速度，30 ms 約 3–5 px）。現在 `plan::Motion`：差 < 4 ms 不動；4–250 ms 從目前推算位置繼續，以 `1 + 差/750 ms` 的速率跑 750 ms 吸收（位置連續、速率恆 > 0，不倒退）；≥ 250 ms、換歌、狀態或曲長改變才直接跳。速率結束點是計畫裡的一個節點（`StripInput.rate`／`slew_until_ms`，三次式係數依速率換算），不需要額外重排。每次重排以 `IDCompositionDevice::GetFrameStatistics` 的下一個合成影格時間（最多 50 ms）為計畫起點（`commit_lead`），因為 DWM 從接手 commit 的那一影格才開始新動畫，否則每次重排落後最多一影格。測試 `a_jittery_drifting_sample_feed_never_steps_or_reverses_the_strip` 以 0–25 ms 延遲抖動、快 0.3 % 的播放器與一次 120 ms 偏移逐 ms 重播，斷言不倒退、單步 < 1 px。Android `smoothPositionMs` 同樣改為從上一影格的顯示位置以速率吸收（原本落後時停住、超前時直接跳）。
- **裝置與 DPI**：每 2 秒（閒置時）檢查 `GetDeviceRemovedReason`，Commit／繪製回報裝置遺失也會重建全部裝置、target 與快取；`WM_DPICHANGED`（浮動視窗套用建議矩形）與 `WM_SIZE` 觸發重排。
- **視窗**：`WS_EX_NOREDIRECTIONBITMAP | LAYERED | TOPMOST | TOOLWINDOW | NOACTIVATE`，工作列視窗另加 `TRANSPARENT`、`WM_NCHITTEST` 回 `HTTRANSPARENT`，擁有者為該條工作列；`taskbar_lyrics.rs` 的擺位、遮蓋、自動隱藏、熱插拔、Explorer 重啟重建（`IsWindow` 失效即重建）不變。浮動視窗：移動模式整窗 `HTCAPTION` 拖曳（游標 SizeAll），游標在窗上時歌詞區顯示 rgba(0,0,0,.28)、10 px 圓角底色與鎖頭；固定模式 `WS_EX_TRANSPARENT`，沿用 100 ms 游標監看，只在游標位於鎖頭上時取消穿透；鎖頭有原生 tooltip（`TOOLTIPS_CLASS`，文字由主視窗以 `tr()` 翻譯後經 `floating-lyrics-tips` 送來）。`WM_MOVE` → `remember_position`（700 ms 後存）；有無譯文 → `fit_lines`；`WM_CLOSE` → 主視窗正常關閉流程。
- **資料流**：主視窗仍是唯一來源，沿用廣播事件 `taskbar-lyrics-data`、`taskbar-lyrics-playback`、`lyrics-inactive-opacity`，新增 `lyrics-native-delay`（每首延遲，換歌與變更時送）與 `floating-lyrics-tips`；`src-tauri/src/native_lyrics.rs` 以 `listen_any` 接收，渲染器啟動時送 `taskbar-lyrics-request`／`taskbar-lyrics-playback-request` 要一次。

### 實測（2026-10-23，隔離測試執行檔：另一個 identifier、資料夾 `target/nativetest/`、音量 0、無聲 WAV＋自製 LRC；使用者的播放器與其 WebView 歌詞同時在跑）

螢幕：DISPLAY1 3840×2160 240 DPI **120 Hz**；DISPLAY6（VDD 虛擬）2560×1440 192 DPI **60 Hz**。測試實例：2 個工作列歌詞＋浮動歌詞，全部「連續」＋陰影，主視窗開啟。

| 項目 | 結果 |
|---|---|
| 測試主視窗 rAF（CDP，10 s） | 120.0 fps，最長間隔 9.1–9.2 ms |
| DWM（`DCompositionGetTargetStatistics`，12 s） | 主螢幕 presents 120.0／refresh 120.0；副螢幕 58.9／60.1（關閉測試實例歌詞時為 120.1／58.1，因使用者播放器的 WebView 歌詞同時在動，DWM 數字無法單獨歸因） |
| 原生渲染執行緒 CPU | 約 1.4%（10 s 內 141 ms；只在換行點約每 2.8 s 重排三個視窗） |
| 測試行程總 CPU | 歌詞開 9.4%、關 1.0%；差額主要在其他執行緒（非渲染執行緒，推測為工作列 UI Automation 監看與事件處理），待細查 |
| 獨立 demo（只有原生視窗） | 行程 CPU 0.00%，主螢幕 120.0、副螢幕 59.9 presents/s |

未驗證：與 WebView 版的像素並排比較（兩者落在同一塊工作列空白，只有重疊截圖）、裝置遺失實際觸發、混合 DPI 拖曳、自動隱藏／全螢幕／Explorer 重啟、鎖頭 tooltip 與固定模式穿透的實機操作。

## 修訂 2026-10-23：位移改為三次曲線段

連續條位移不再是分段線性：`plan.rs` 的 `StripPlan.offset` 改為 `Cubics`（每段 `(秒, [c0, c1, c2, c3])`），由
`player_lyrics_rules::strip_cubic` 取得各換行點之間那一段的單調 C1 三次 Hermite 多項式（規則見 ADR 0032 的
2026-10-23 修訂），把毫秒係數換成秒，`win.rs` 的 `set_offset_x_cubic` 每段呼叫一次
`IDCompositionAnimation::AddCubic(時間, c0, c1, c2, c3)`，最後 `End`；DWM 自己逐幀計算（2026-10-24 起改為本程序每幀提交，見文末）。換行點就是
曲線的節點，所以一個計畫（接下來 3 個換行點）剛好是 3 段；切線只看相鄰節點，重排時取樣的是同一條全域曲線，前後計畫
銜接沒有折角。位移視覺仍用最近鄰取樣（整數裝置像素）。不透明度關鍵格仍是分段線性。單元測試
`the_cubic_keys_match_the_curve_every_10_ms_for_uneven_lines` 逐 10 ms 比對計畫與規則。

## 邊緣淡出（2026-10-23）

DirectComposition 不能以漸層遮罩 visual：`IDCompositionDevice3` 的濾鏡效果輸入只能是其他效果或 visual 自身內容，不接受
surface，所以無法放靜態漸層當 DestinationIn；`MIN_BLEND` 合成模式不是乘法，且無中間層時會與桌面混合；
`CompositionMaskBrush` 需要把整個渲染器改到 Windows.UI.Composition（Desktop 互通 1803+）。採用的做法：`edge_faded`
依 `edge_fade_bands` 把歌詞內容切成垂直條帶，每條是靜態 `SetClip2`（整數裝置像素，相鄰條不重疊、不留縫）加靜態
`SetOpacity2`，條帶內各建一份歌詞（同一份快取 surface、同一組計畫出的動畫），由 DWM 每幀求值；
只在重排（換行、跳轉、設定、尺寸／DPI）時重建，裝置遺失重建沿用原流程。每側最多 64 條：淡出區 ≤ 64 裝置像素時每條
1 px（精確的逐像素斜坡），更寬時每階 1/64。代價是 visual 數量約為 (2×64＋1)×（每條的歌詞格數＋1）；若實測 DWM 成本
過高，下一步是改用 Windows.UI.Composition 的 `CompositionMaskBrush`。

（已取代，見下節：實測三窗 dwm.exe 約 95%、120 Hz 螢幕只剩 33 presents/s，條帶版先以 `EDGE_FADE_BANDS = false`
關閉，再整個移除。）

## 改用 Windows.UI.Composition 與遮罩淡出（2026-10-23）

原因：條帶淡出每窗約 1,300 個 visual，使用者實測三個連續歌詞視窗、120 Hz 主螢幕時 dwm.exe 吃滿約一核（兩種歌詞都關時
9–16%）。DirectComposition 沒有漸層遮罩（見上節），所以渲染器的合成層改為 Windows.UI.Composition（Desktop 互通），
D3D11／D2D／DWrite 與點陣化、規則、計畫（`plan.rs`）都不變：

- 渲染執行緒建立 `CreateDispatcherQueueController`（`DQTYPE_THREAD_CURRENT`、`DQTAT_COM_STA`），本身的訊息迴圈照舊；
  一個 `Compositor`，每個 HWND 以 `ICompositorDesktopInterop::CreateDesktopWindowTarget(hwnd, true)` 建 target
  （視窗樣式、擁有者、置頂、不搶焦點、點擊穿透、鎖頭與拖曳都沒改），`ICompositorInterop::CreateGraphicsDevice`
  把既有 D2D 裝置包成 `CompositionGraphicsDevice`，每行（含陰影／描邊）只畫一次進快取的 `CompositionDrawingSurface`。
  變更由 dispatcher queue 隱式提交（不再呼叫 `Commit`）。
- 動畫（已由下節「每幀提交」取代）：`keyframes.rs` 把計畫轉成 `ScalarKeyFrameAnimation`。滑動的每段三次多項式
  ＝時間控制點固定 1/3、2/3 的 `CubicBezierEasingFunction`（此時貝茲的時間座標對參數是線性的，進度就是時間的三次
  多項式），數值控制點由段兩端斜率算出，所以與 `plan.rs` 的分段單調三次曲線（同樣的節點、漂移修正的節點）完全相同；
  淡化與長句捲動是線性關鍵格，由合成器每幀求值。測試每 10 ms 比對計畫（含漂移修正、
  行首錨點與規則一致）。動畫的起點仍提前到下一個合成幀：改用 `DwmGetCompositionTimingInfo` 的下一個 vblank
  （取代 DComp 的 `GetFrameStatistics`）。
- 整數裝置像素：所有歌詞 sprite 的 `CompositionSurfaceBrush` 用 `NearestNeighbor` 取樣、`Stretch::None`，滑動中的
  小數位移照樣落在整數像素上（與 DComp 的最近鄰相同；沒有用 `SnapToPixels`）。
- 淡出：每窗一個 `ContainerVisual`（歌詞內容）交給 `CompositionVisualSurface` 離屏繪製，畫面上的 `SpriteVisual` 用
  `CompositionMaskBrush`（source＝該 visual surface 的 surface brush，mask＝靜態水平 `CompositionLinearGradientBrush`，
  Absolute 座標、Clamp），漸層停駐點由 `sampling::edge_fade_stops`（與 `edge_fade_opacity` 相同的分段線性）產生；
  只在視窗尺寸、DPI 或歌詞區改變時重建，重排時只換內容。每窗只有一份歌詞 visual。窗透明度設在遮罩 sprite 上。
  診斷用 `MOE_LYRICS_NO_FADE=1` 關閉淡出（內容改為剪裁到歌詞區＋效果餘量）。
- 裝置遺失：每 2 秒檢查 D3D 裝置，遺失或繪製失敗即關閉 target、整組重建（新 compositor 與 target）；
  `CompositionGraphicsDevice.RenderingDeviceReplaced` 觸發時清除 surface 快取並重排全部視窗。
- 最低系統：**Windows 10 1809（UniversalApiContract 7，`CompositionVisualSurface` 需要）**；Desktop 互通本身是 1803。
  啟動時以 `ApiInformation::IsApiContractPresentByMajor` 檢查，不符合或 dispatcher queue 建立失敗時記錄
  「needs Windows 10 version 1809 or later」，不建立任何歌詞視窗（`create` 回傳 None），不會當機。
- 相依：只加同一個 `windows` 0.62.2 的 feature（Foundation、Foundation_Metadata、Foundation_Numerics、Graphics、
  Graphics_DirectX、System、UI、UI_Composition、UI_Composition_Desktop、Win32_Graphics_Dwm、Win32_System_Performance、
  Win32_System_WinRT、Win32_System_WinRT_Composition），`Cargo.lock` 不變。

實測（2026-10-23，隔離的 `--example demo`，3 個視窗〔浮動＋2 個工作列樣式〕、連續、陰影，各 20 秒；
主螢幕 **120 Hz**、第二螢幕 **60 Hz**；使用者的 Release 播放器同時執行，dwm.exe 閒置基線約 23.6–26%，
`Get-Counter '\Process(dwm)\% Processor Time'` 每 2 秒取樣）：

| 版本 | dwm.exe（單核 %） | 測試行程 CPU | presents/s（120 Hz／60 Hz） |
| --- | --- | --- | --- |
| DComp＋條帶淡出（e936cd2） | 94.9 | 10.5% | 33.5／23.5 |
| DComp、淡出關閉（ef1ce35） | 28.5 | 4.1% | 120.0／58.5 |
| WUC、淡出關閉（`MOE_LYRICS_NO_FADE=1`） | 28.1 | 2.8% | 120.0／60.0 |
| WUC＋遮罩淡出 | 25.2 | 3.5% | 120.0／60.7 |

遮罩淡出的 DWM 成本在量測雜訊內（與淡出關閉相同）。截圖確認淡出平滑、外緣全透明。未驗證：真實裝置遺失、
1809 以前系統的退路（只在本機 Windows 11 執行過）、鎖頭懸停外觀、Release 播放器內的實機外觀。

## 每幀提交（2026-10-24，取代關鍵格動畫）

問題：使用者回報連續歌詞又會跳動（aedd5e1 在 DComp 上修好過）。逐幀螢幕擷取（`DwmFlush` 節奏、60 Hz 擷取）證實：
286b893 的關鍵格版本在每次重排時向前跳約 11 px。原因有兩層：(1) WUC 的關鍵格動畫在合成器拿到批次的那一幀才開始，
本程序無法得知確切時間，每次重排都早或晚 0–2 幀；(2) 本機的 WUC 合成器時間比牆鐘慢約 4%（5 秒動畫實際 5,216 ms、
2 秒 2,090 ms，與 120 Hz／8.333 ms 的 DWM 計時不符；`GlobalPlaybackRate` 讀到 0、設為 1 也一樣；探測延遲在斜率擬合
中抵消），所以由合成器時鐘驅動的動畫會越來越落後播放時間（擷取：每行約 +100 ms）。原因仍不明。曾試過共用合成器時鐘
（`CompositionPropertySet` 加 `ExpressionAnimation`）加完成探測估計速率與原點，能同步，但複雜且依賴未明的合成器
行為；使用者決定改為每幀提交，該方案已移除。

做法（`sampling.rs`、`win.rs`）：
- 計畫（`plan.rs`，含漂移修正的分段單調三次曲線、200 ms 淡化）完全不變，仍只在換行、跳轉、暫停／繼續、播放校正、
  新歌詞、設定、尺寸／DPI 改變時重排；每行仍只點陣化一次（快取的 `CompositionDrawingSurface`），淡出遮罩不變。
- 渲染執行緒有視窗在動時，每個合成幀：依本程序的播放時鐘在計畫上取樣每個視窗的條位移、各格不透明度與長句捲動
  （`Live`），**只有裝置值改變時才寫入**——位移一律是整數裝置像素、變動至少 1 px 才寫；不透明度量化到 1/255、變了才寫；
  都沒變就不寫，也就沒有提交。寫完立刻處理訊息（隱式提交），再以 `DwmFlush` 等下一個 DWM 合成。`DwmFlush` 在 DWM
  沒東西可合成時會立刻返回，所以返回不到 1 ms 時改等約一幀，不會空轉；沒有視窗在動（暫停、沒歌詞、行尾停住）時
  回到原本的訊息等待，完全不輪詢。
- 節奏選擇：一個迴圈、以 `DwmFlush` 對齊 DWM 的合成（在 120／60 Hz 混合時跟著較快的合成，實測主螢幕 120 presents/s）。
  60 Hz 螢幕上的視窗也每個合成幀都更新，該螢幕每個 vblank 取到最新值，等於 60 fps；不需要每台螢幕各自的 vblank 等待
  （`D3DKMTWaitForVerticalBlankEvent`／`DCompositionWaitForCompositorClock` 會讓多螢幕時序複雜很多）。
- 取樣時間：寫入的值約在 `PRESENT_LATENCY`（32 ms）後出現在螢幕上（隱式提交、DWM 合成與 present）；以逐幀擷取對照
  DComp 版（ef1ce35）量得 120 Hz 約 39 ms、60 Hz 約 22 ms，取 32 ms 讓兩台都在約一幀內。這是一台機器上的實測值。
- 已移除：`keyframes.rs`、關鍵格／運算式動畫、`commit_lead`、緩動函式。`Cargo.lock` 不變。

規則（取代先前「本程序零每幀工作」的說法）：每幀 CPU 越低越好。每幀只做兩件事：依播放時鐘計算位置，以及提交有變的值；
不在每幀點陣化、排版或模糊，能快取的都快取；位置沒移動至少 1 個裝置像素（不透明度 1/255）就不寫入、不提交；閒置時睡眠。

測試：`sampling.rs` 的 `per_frame_samples_never_step_or_go_back`（120 Hz 取樣兩分鐘，換行與每 500 ms 的播放取樣都重排，
漂移修正 ±5–240 ms：寫入的位移永不後退，重排前後同一瞬間差 ≤ 1 px，每幀移動不超過曲線本身的移動＋1 px）、
`writes_only_on_device_changes`、`the_edge_fade_stops_match_the_rule`；`plan.rs` 既有的 60 秒 1 ms 模擬照舊。

實測（2026-10-24，隔離 demo；使用者播放器同時執行，dwm 閒置約 7%）：

| 版本 | dwm.exe | 測試行程 CPU | presents/s（120／60 Hz） |
| --- | --- | --- | --- |
| 關鍵格＋遮罩（286b893） | 15.6% | 3.5% | 120.0／32.0 |
| 每幀提交＋遮罩 | 17.0% | 4.9% | 117.3／37.5 |

（3 個視窗、連續、陰影、20 秒；60 Hz 欄是該螢幕的全部 present，三個視窗都在主螢幕。117 < 120 是位移不到 1 px 的幀不提交。）
行首到達（每行左緣到歌詞區中心的時刻，逐幀擷取、各 2 分鐘，相對 DComp 參考版同一量法）：120 Hz 主螢幕 −14 ms
（標準差 3.7 ms），60 Hz 虛擬螢幕 −7 ms（標準差 6.6 ms），前後半段相同（不漂移）；量法本身每次相差約 ±8 ms。
60 Hz 擷取 20 秒沒有任何後退。未驗證：Release 播放器內、真實裝置遺失、120 Hz 逐幀（擷取只有 60 Hz）。

## 呈現時刻取樣與次像素位移（2026-10-24）

使用者回報每幀提交版有輕微抖動。隔離 demo 的逐幀紀錄（`MOE_LYRICS_FRAME_LOG=<檔案>`：每幀的喚醒時間、取樣時間、
寫入的位移）在 120 Hz 主螢幕證實兩個原因都在：(1) `DwmFlush` 後的喚醒時間標準差 1.76 ms（p5 7.3、p95 9.5 ms），
取樣用「喚醒＋32 ms」，所以取樣間隔同樣抖動（1,178 幀中 146 幀偏離 1 ms 以上）；(2) 整數像素步進讓每幀位移是
1、2、3 px 不規則交錯。

- 取樣時刻（`present_time`）：取 `DwmGetCompositionTimingInfo` 的 `qpcVBlank`（DWM 閒置時依 `qpcRefreshPeriod`
  往後推整數個週期）加上 `PRESENT_LATENCY` 取整到整數週期（120 Hz 為 4 個週期＝33.3 ms），所以每幀的取樣時刻剛好相隔
  一個更新週期、與喚醒時間無關。實測取樣間隔偏離 1 ms 以上從 146／1178 降到 7／1177（剩下的是 DWM 漏掉的幀）。
  取不到 DWM 計時時退回「現在＋`PRESENT_LATENCY`」。
- 次像素（使用者決定預設採用，認為柔化不明顯）：歌詞 sprite 改線性取樣，位移以 `OFFSET_STEP_PX`＝1/8 裝置像素為單位，
  變動不到 1/8 px 不寫入（不透明度仍是 1/255）。理由：線性取樣下 1/8 px 的移動對邊緣像素覆蓋率的影響最多 1/8，
  已低於看得出移動的程度；最慢的滑動（約 100 px/s）1/8 px 不到 2 ms，滑動仍均勻；每移動 1 px 最多寫入 8 次。
  逐幀位移改為跟著曲線（例：2, 2, 2.125, 2, 2, 2.125…，先前是 1／2／3 交錯）。未拍靜止截圖比較銳利度（使用者已決定）。
- 測試：`per_frame_samples_never_step_or_go_back` 改為檢查每幀位移等於曲線本身的移動（誤差在兩個步進內）、重排前後差
  不超過一個步進。

## 音訊時鐘（2026-10-24）

使用者在 Release（c86b8d4）仍看到輕微抖動。原因：渲染器的時鐘來自主視窗每 500 ms 的 `taskbar-lyrics-playback`
取樣（延遲不定），每個取樣都以 rate 1 ± 漂移／750 ms 修正 750 ms，滑動速度因此每隔約半秒改變 1–4%。

做法：`player-audio-windows` 的 `PlaybackSnapshot` 新增 `position_at`（音訊工作執行緒約每 40 ms 從後端讀位置時蓋上的
時刻；只有位置改變的更新才蓋），`src-tauri/src/native_lyrics.rs` 把它做成 `ClockSource` 交給渲染器。位置來源：共享模式
是 rodio 已取用的樣本減延遲線的靜音，獨佔 WASAPI／DoP 是從環形緩衝取出的幀數減端點延遲，DSD 先轉 PCM 再走這兩條。
渲染器每次讀到新的讀數就比對自己的時鐘（`audio_sync`），以 1 秒視窗的平均誤差（抵消後端以緩衝為單位的階梯，實測
共享模式位置以 10 ms 為步）決定：< 3 ms 不動；到 250 ms 以 4 秒慢慢修正（每 10 ms 誤差改變速度 0.25%）；更大就跳
（跳轉、卡頓）。播放中頁面的取樣不再修正時鐘，只帶曲目、狀態與長度；狀態或曲目改變時的位置也取音訊時鐘。

實測（使用者的 Release 播放器，8cafbd6，`MOE_LYRICS_FRAME_LOG`，共享模式，約 20 秒連續播放）：渲染器時鐘每 500 ms 的
速度標準差 0.039%（最大偏離 0.11%），先前設計為每次取樣 1–4%；音訊讀數與渲染器時鐘的誤差平均 −0.3 ms、標準差 3.0 ms。
浮動歌詞每幀位移相對平滑值的殘差標準差 0.08 px（c86b8d4 時 0.27 px，但兩次是不同歌曲、速度不同）。
未驗證：獨佔模式與 DSD 的實際播放紀錄。

