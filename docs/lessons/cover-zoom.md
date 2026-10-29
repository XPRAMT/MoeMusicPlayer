# 經驗：播放頁封面縮放（cover zoom）

決策與完整參數見 ADR 0036（「縮放效能設計」「封面清晰度：重光柵微移」「還原動畫跑在 stage 上」）；現況見 `docs/status/now-playing-and-lyrics.md`。本文只記反覆修復的轉折、邊界情況與驗證方法。

## 最終做法

只對 `.cover-stage` 套 `transform`，圖層結構永久固定（`z-index: 1`＋`will-change: transform`）；放大後的清晰度靠「重光柵微移」（`translate: 0.01px` 的 CSS transition，`src/lib/cover-reraster.js`），雙擊還原由 stage 自己跑 Web Animation（`restoreGlide`，`fill: none`）。沒有任何覆蓋 canvas。

## 修復時間線（只留轉折）

| commit | 改了什麼 | 為什麼前一版不行 |
| --- | --- | --- |
| `401e1e86` | 所有輸入合併成每幀一次 rAF、只讀一次 rect；縮放期間凍結歌詞版面（`lyrics-layout-freeze.js`） | 每個輸入事件各自讀寫版面，強制重排；歌詞每步重排使縮放掉幀 |
| `0f709a47` | 還原時最終版面與 FLIP 補償在同一個 task 提交；`repin` 在 microtask（`settleLayout`） | 先前在下一個 rAF 才重新釘錨，每步多畫一幀未補償的偏移 |
| `d4fd3927` | Android：還原改畫縮圖大小的 canvas 複本做滑回；清晰度改由覆蓋 canvas；圖層結構固定；歌詞列只以行索引為 key | 結束手勢時移除 `will-change`／切 `z-index` 會讓 Android 重建圖層，封面與歌詞空白約 2 幀；新建歌詞列來不及點陣化 |
| `11435f1c`、`b13fdb8c`、`ef601ff4` | 覆蓋 canvas 在手勢中跟著 stage 移動、複製 swipe 回饋、忽略「值沒變」的 MutationObserver 紀錄 | canvas 被平移／縮放／swipe 回饋的 inline style 當成外來變動而隱藏，雙指時忽清忽糊；`classList.remove` 不存在的 class 也會產生 mutation 紀錄 |
| `4c6140ce` | 新增 `data-cover-pannable`，只有放大時滑鼠拖曳才是平移 | `data-cover-zoomed` 同時代表「縮小」，縮小的封面滑鼠拖曳不能切歌 |
| `f48d753d`、`3b971cf1` | swipe 放開時同一次寫入就清掉並隱藏 canvas；彈回真的結束（computed translate／scale 歸零、沒有執行中的 transition）才重畫，畫前再核對 rect | canvas 複製的彈回錯過結束點，留下錯位的「第二張封面」（ghost）；清掉 transition 字串並不會停止執行中的 transition |
| `3d8a30b7`、`4daefd59`、`47bc4e26` | 清晰度改為重光柵微移，canvas 只剩還原滑回；比較 translate 時解析數值（瀏覽器把 `0.01px 0px` 序列化成 `0.01px`），加 safety net | canvas 與 stage 的接手時機一直產生閃爍與 ghost；字串比對誤判微移是否還是自己的 |
| `196b3e5c` | 還原滑回也改在 stage 上跑 Web Animation，刪除整個 canvas（`cover-overlay.js` → `cover-swipe-feedback.js`） | 只要有第二份影像就有同步問題 |
| `941420ff` | window 捕獲階段的 `pointerup`／`pointercancel` 與 `buttons === 0` 的 `pointermove` 清掉殘留的滑鼠指標（`cover-pointers.js`） | 在封面外放開滑鼠時節點收不到 pointerup，`pointers` 永遠非空，微移再也不觸發，封面一直是糊的 |

反覆修的根本原因：用「第二份影像（canvas）」補清晰度，等於要在每種手勢、回饋、換歌、版面變動下讓兩份影像逐幀一致，每補一個情況就冒出下一個。拿掉第二份影像、改找能讓瀏覽器自己重光柵的觸發條件，問題才收斂。

## 邊界情況

- 常駐 `will-change` 的圖層維持升級當時的光柵倍率，放大後一定糊（Android LayerTree：998×998 圖層套 7.1 倍）。
- 直接寫 `scale`（1.000001–1.0005）或用 Web Animation 動畫它都不能穩定觸發重光柵；translate 的 CSS transition 可以，0.01 px 就夠。
- 瀏覽器把 `translate: 0.01px 0px` 讀回成 `0.01px`；判斷屬性是不是自己寫的要解析數值，不要比字串。
- 清掉 inline `transition` 不會停止已在跑的 transition；要看 `getAnimations()` 與 computed 值。
- `classList.remove` 一個不存在的 class 仍會產生 MutationObserver 紀錄；觀察者要比對 old value。
- 第二指以合成 `pointercancel` 取消第一指的滑動；swipe 回饋可能在微移進行中蓋過它，此時以回饋為準、只收回自己寫的屬性。
- 滑鼠在節點外放開（拖出視窗、被 swipe-dismiss 捕獲）收不到 pointerup；觸控／筆的 `buttons === 0` 不代表放開，不能套同一條規則。
- 換歌新圖載入、播放頁重新開啟要重設「已微移」狀態；還原動畫中換歌時元件被銷毀，`destroy` 要取消動畫。
- 還原動畫中再滾輪或按下：先讀含動畫的矩形再取消動畫，用 `stateFromVisual` 接手（倍率 ≤ 1 時無法帶位移，回到版面中心）。
- Windows GPU 快，很多 Android 的空白幀、ghost 在 Windows 看不出來；一定要在手機錄影確認。

## 已排除的做法（不要再試）

- 手勢結束後移除 `will-change`、或切 `z-index`／`contain`／`isolation`／`position`：Android 重建圖層，空白約 2 幀。
- 高解析覆蓋 canvas（清晰度或還原滑回）：與 stage 的接手時機造成閃爍、一次性模糊、ghost；需要鏡像所有 inline style。
- 寫 `scale` 微小值或以 Web Animation 觸發重光柵：不穩定。
- 縮放期間每步重排歌詞、以 layout generation 當列的 key：掉幀，Android 空白。

## 驗證方式

- 自動測試：`node --test tests/cover-zoom.test.mjs tests/cover-reraster.test.mjs tests/cover-pointers.test.mjs tests/lyrics-layout-freeze.test.mjs tests/swipe-dismiss.test.mjs`。
- Windows CDP（Release exe、埠 9222，啟動方式見 AGENTS.md「工作與驗收規則」）：
  - 效能：在 1.0→1.3、1.5→6、1.0→0.5 三段送合成滾輪，比對 `Performance.getMetrics` 的 LayoutCount／RecalcStyleCount／LayoutDuration／ScriptDuration 增量、rAF 間隔直方圖與 LoAF。
  - 還原：雙擊後逐 rAF 讀 `.cover-stage` 的 `getBoundingClientRect`，第 0 幀要與還原前重合，每幀位移不超過 200 ms ease 曲線的一步（120 Hz 約總位移 4%）。
  - 殘留指標：滑鼠按住封面拖到視窗外放開，再放大停 50 ms，`localStorage['moe.debug.coverSwipe']='1'` 時主控台要出現 `[cover-reraster]`。
- Android 實機：放大到數倍後停住，封面要在約 0.3 秒內變清晰；雙指縮放、兩指平移、左右滑切歌、下滑收合、雙擊還原各做一次，螢幕錄影逐幀確認沒有空白、缺角、第二張封面、結束時閃一下；效能用 `adb shell dumpsys gfxinfo com.moemusicplayer.app framestats`。

## 下次改這塊前必看

1. 不得新增第二份封面影像（canvas、img 複本）；清晰度問題先看重光柵條件。
2. 不得在手勢或動畫開始／結束切換任何合成／堆疊屬性（AGENTS.md 硬性規則）。
3. 版面變動與補償 transform 必須在同一幀繪製前完成，不能有未補償的一幀。
4. 任何「這個屬性是不是我寫的」的判斷，都用解析後的值與 computed 狀態，不要用字串。
5. 改完要在 Android 手機錄影驗證，Windows 沒問題不代表 Android 沒問題。
