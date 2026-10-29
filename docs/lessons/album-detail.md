# 經驗：專輯詳情頁（封面收合、摘要、演出者列、封面掃描）

決策見 ADR 0012（收合與捲動容器、演出者列）、ADR 0037（列表與專輯頁）；現況見 `docs/status/ui.md`。本文只記反覆修復的轉折、邊界情況與驗證方法。

## 最終做法

收合由一個進度變數 `--album-detail-collapse-progress` 驅動，每幀最多寫一次（`album-detail-collapse.js`，所有平台、不做功能偵測）；封面 `scale`、摘要 `translateY`／`margin-left`／`max-height`、欄位表頭 `translateY` 都讀它。摘要文字隨封面縮小連續重排（唯一允許的版面工作），演出者列以 JS 切整行（`album-artists-fit.js`）。封面掃描在開啟專輯時做一次，離開頁面即取消。

## 修復時間線（只留轉折）

| commit | 改了什麼 | 為什麼前一版不行 |
| --- | --- | --- |
| `b67f09e5` | 封面固定 200px 排版、只用 `transform: scale`；支援捲動時間軸時不再每次捲動寫 inline style | 先前每次捲動改版面與 inline style，主執行緒工作多 |
| `ba5badfe` | 給捲動時間軸動畫明確的 `animation-duration: 1s` | WebView2 把 scroll-driven 的 `auto` duration 當成已完成的動畫，捲動位置 0 時封面就顯示最小（只顯示最小張的圖） |
| `5e1299b8` | 摘要文字隨封面縮小連續重排（使用者核准的例外），加 `moe.debug.albumReflow` 量測 | 摘要固定寬度時，封面讓出的空間空著 |
| `3d4b92f2` | 捲動時間軸整個換成每幀一次的進度變數 | Android WebView 在捲動時間軸驅動頁面時丟掉 fling 的中間幀（每次 fling 中位數 12 個捲動事件、最大一步 830 px、最大間隔約 1 s）；改用變數後 131 個事件、最大一步 58 px、最大間隔 19 ms |
| `bfeb351a`、`8bc87173`、`a4a0e8b7` | 演出者列改 JS 二分切整行＋遲滯；摘要固定高度；沒被切時垂直置中 | `line-clamp` 在 Android WebView 無效（`display` 仍是 `flow-root`）；觀察摘要本身會形成 fit → 高度 → resize → fit 迴圈，在 4、5 行之間跳動；漸層遮罩會切半行 |
| `94643a5b` → `0312d3ad`（還原）→ `40b989fa`、`a038e7d4`、`23f8e52e` | 封面掃描：一次只跑一個、離開頁面取消（檔案之間與讀檔中都可取消）、只讀 tag、每次掃描一行 log | 一個 commit 同時改遠端查詢上限、取消、記憶缺圖、計數，5 分鐘後整個還原（commit 沒寫理由），之後拆成三個可分別驗證的 commit 重做 |

反覆修的根本原因：收合的驅動方式換了三次（inline style → 捲動時間軸 → 進度變數），每次都只在一個平台驗證。捲動時間軸在 WebView2 有 duration 問題、在 Android WebView 有丟幀問題，兩者都只有實機 CDP 才看得到。

## 邊界情況

- WebView2 的 scroll-driven 動畫 `animation-duration: auto` 可能被當成已完成（progress 1），封面一開頁就是最小。
- Android WebView 用捲動時間軸時 fling 會跳幀；單元測試和 Windows 都看不出來。
- 「封面縮放卡頓」的背景：使用者錄影發現，專輯詳情封面收合在「連續」歌詞動畫開啟時掉幀，工作列＋浮動歌詞同時開會卡死；「無」不卡；歌詞陰影改描邊（不再 blur）就不卡。當時 Windows 歌詞視窗是 WebView，與主視窗共用同一個 GPU 程序與 vsync 執行緒；後來歌詞視窗改成原生視窗。程式化捲動量到的約 120 fps 不能代表這個情況。
- `line-clamp` 在 Android WebView 無效；CSS 行高是 `normal` 時要量實際渲染行高。
- 演出者列行數改變時，統計區塊會移一行（已接受）；切與不切交界時內容最多移半行。
- 捲動狀態還原、範圍或標題高度改變時，shell 的 style 字串被整個換掉，進度變數必須立刻重寫，不能等下一次捲動。
- 封面掃描要能在單一檔案讀取中途取消，否則離開頁面後遠端讀取還在跑。

## 已排除的做法（不要再試）

- CSS 捲動時間軸（`scroll-timeline`／`animation-timeline`）：WebView2 duration 問題、Android fling 丟幀。
- 每次捲動寫多個 inline style 或改版面：主執行緒負擔，且會讓列表捲動掉幀。
- `line-clamp`、漸層遮罩切演出者：Android 無效、會切半行。
- 觀察摘要自身尺寸來決定行數：形成迴圈。
- 一個 commit 同時改掃描的多個面向：出問題只能整個還原。

## 驗證方式

- 自動測試：`node --test tests/album-detail-collapse.test.mjs tests/album-detail-compositor-collapse.test.mjs tests/album-artists-fit.test.mjs tests/album-reflow-debug.test.mjs tests/album-detail-cover-scan.test.mjs`。
- Windows CDP：開專輯，`scrollTop` 0 時封面要是 200px（讀 `.album-detail-art` 的 `getBoundingClientRect` 與 computed transform）；逐 rAF 拖捲軸三趟，表頭下緣以上不得出現列，捲動容器 `clientHeight`／`scrollHeight`／top 不變。摘要重排成本：`localStorage['moe.debug.albumReflow']='1'`，主控台每秒印一次每幀強制版面毫秒數。
- Android CDP（真實手指 fling）：記錄每次 fling 的捲動事件數、最大一步、最大間隔，以上面兩組數字為基準。
- 卡頓情境（使用者實機）：分別在歌詞動畫「無」「連續」、只開工作列歌詞、只開浮動歌詞、兩者都開時，螢幕錄影專輯詳情封面收合，逐幀統計是否連續數幀沒更新。
- 封面掃描：開專輯後立刻離開，log 要出現該次掃描被取消的那一行，之後沒有它的讀取。

## 下次改這塊前必看

1. 收合只讀一個進度變數；新增會動的元素也讀它，不要另起捲動時間軸或捲動監聽。
2. 改動要同時在 Windows CDP 與 Android 真實 fling 驗證。
3. 除了摘要文字，其他元素只能用 transform；摘要的重排要用 `moe.debug.albumReflow` 量。
4. 掉幀問題先排除歌詞視窗的影響（「無」與「連續」對照），不要只用程式化捲動量 fps。
5. 掃描、遠端查詢的改動拆小，每個都能單獨驗證與還原。
