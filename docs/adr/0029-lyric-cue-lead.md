# Lyric line changes lead the line start

Status: Accepted and implemented on 2026-10-22

Decision date: 2026-10-22, Asia/Taipei (UTC+08:00)

## Context

On the Now Playing page a new line only started its change animation (smooth scroll, 140 ms colour/opacity
transitions) when the line started, so the page settled after the singing had already begun and the lyrics felt late.
The smooth scroll's length was chosen by the browser, so it could not be made to end at a given moment.

## Decision

- One constant, `LYRIC_CUE_LEAD_MS` = 200 ms (first set to 300 ms, shortened the same day) in `src/lib/lyrics-cue-timing.ts`, is both the lead and the animation
  length; CSS reads it through `--lyric-cue-ms`. Not user-adjustable.
- While playing, a line is shown `lead` before its start: lead = min(200 ms, gap to the previous line); the first line
  is never shown before position 0. The user's lyric delay still applies on top. The animation runs for the time left
  until the line really starts, so a shortened lead or resuming inside the lead animates for what remains.
- The scroll is a fixed-length ease-out rAF tween (only while it runs) instead of `scrollTo({ behavior: 'smooth' })`.
- `activeTimedIndex` stays the line really playing; `displayTimedIndex` drives what is drawn. Paused playback does not
  lead; seeks while playing, track changes and reduced motion stay instant. Play counts and the previous-button
  three-second rule use the real position.
- Taskbar lyrics: no fade; the line switches at its real start (no lead); long-line scrolling starts at the real start.
  (Current state: the taskbar and floating lyric windows are now drawn natively, ADR 0034/0041; they still have no
  lead and change line at `startMs - offset`, and the 200 ms line fade of the old WebView window no longer exists
  there. The lead applies to the Now Playing page only.)

## Consequences

- The highlighted line on the Now Playing page changes up to 200 ms before the line's timestamp, by design.
- Playback-clock corrections can still move a change by up to the clock's convergence error (< 250 ms, ADR 0028).

## Browse and seek

- A scroll by the user (wheel, pointer or touch on the timed lyrics viewport, including the scrollbar) starts
  browsing. While browsing, the row under the viewport centre (`findBrowseCentreRow`, a search over the same layout
  the virtual window uses, run in the existing scroll handler) shows an icon-only play button, unless it is the
  current line (`browseButtonIndex`); the seek time is in its tooltip (`appTooltip`) and aria-label. Every timed row
  always reserves a symmetric 24 px gutter on both sides (`--lyric-browse-gutter`) and the 22 px button (a 22 px
  icon, `--lyric-browse-button`) sits in the left one, 1 px from the row edge (`--lyric-browse-inset`) and 1 px from
  the text, so it never covers text, centred text stays centred and rows never reflow when it appears; rows are border-box, so the row measurements, karaoke widths and the cover-zoom frozen width include the
  gutter unchanged. It fades in over 200 ms (no fade with reduced motion).
- The button seeks to the moment the line becomes current: `ceil(startMs - display offset)`, clamped at 0
  (`browseSeekPositionMs`); no `LYRIC_CUE_LEAD_MS`. The seek goes through the shell (`onSeek` →
  `commitPlaybackSeek` → `seekPlayback`), browsing ends at once, and the jump lands instantly as any seek. A line
  whose seek target is at or past the track's known duration gets no button, since a seek there would only end the
  track (ADR 0036; an unknown or zero duration does not limit it).
- Hold, no timeout (`browseHoldAfterScroll`, `browseCueAction`): when the user's scroll ends (no pointer or finger
  down, the wheel guard over, the scroll and its momentum at `scrollend`) a hold of `BROWSE_HOLD_MS` (3 s) starts;
  every new user scroll clears it and it restarts when that scroll ends. A line change inside the hold, or while the
  user is still scrolling, does not pull back; the hold ending does nothing by itself (no timer); the first line
  change after it ends browsing and the normal cue scroll returns to the current line. Example: scroll ends 0:10, a
  change at 0:11 is ignored, the hold ends 0:13, the view stays, the next line at 0:20 pulls back. The button keeps
  showing meanwhile. A seek or track change while browsing (and the user not scrolling) returns at once, instantly.
  While browsing, `correctTimedCenter` does not recentre and re-measured rows keep the view in place.
- Pressing the button is not a scroll (it neither stops the cue scroll nor starts browsing) and carries
  `data-swipe-ignore` for the Now Playing swipe gestures. Plain lyrics and the lyric windows have no browsing.

## 目前設計細節（自 AGENTS.md 移入）

以下是目前仍有效的實作細節；AGENTS.md 只保留摘要與指向本節的連結。

播放頁同步歌詞（ADR 0029）：虛擬視窗依捲動位置繪製（`lyrics-window.js` 的 `getTimedLyricWindow(layout, scrollTop, 視窗高, 頭部留白, 額外 scrollTop[], overscan=6)`），滑動中範圍取目前視窗與起點／終點的聯集，只在列範圍或留白改變時（`sameLyricWindow`）更新。換行動畫提前開始、在該句時間點剛好結束：提前量與時長只定義在 `src/lib/lyrics-cue-timing.ts` 的 `LYRIC_CUE_LEAD_MS`（200 ms，不開放設定），兩句間距更短時縮短為間距，第一句不早於 0（`displayCuePositions`）；`activeTimedIndex`（`data-current-cue-index`）是真正的當前句，畫面用 `displayTimedIndex`（`data-active-cue-index`、`.active`、`aria-current`）。捲動用自訂 rAF 補間（`startCueScroll`，結束後 `correctTimedCenter`，使用者滾輪／觸控中止），color／opacity transition 用 `--lyric-cue-ms`。所有捲動變化經 `lyrics-scroll-glide.ts` 的 `planTimedScroll`：正常換行為 200 ms ease-out（`restingGlide`），調整每首延遲或 glide 進行中的換行以 `retargetGlide` 從目前位置與速度重設目標（`LYRIC_OFFSET_GLIDE_MS`＝`LYRIC_CUE_LEAD_MS`），位置取整到裝置像素（`snapToDevicePixel`）；`cuePositionMs` 依 `timedOffsetMs` 重算。暫停中不提前；繼續播放時離下一句不到提前量則以剩餘時間動畫；跳轉、換歌、版面改變與 `prefers-reduced-motion` 一律 instant。播放次數、上一首 3 秒判斷用原始時間。測試：`tests/lyrics-cue-timing.test.mjs`、`tests/lyrics-scroll-glide.test.mjs`、`tests/lyrics-controller.test.mjs`、`tests/lyrics-view.playwright.js`（需另開測試伺服器）。Release 實機未驗收。
