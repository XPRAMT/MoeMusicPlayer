# Floating lyrics on Windows and Android

Status: Accepted and implemented on 2026-10-23

Decision date: 2026-10-23, Asia/Taipei (UTC+08:00)

Builds on [ADR 0018](0018-taskbar-lyrics.md) and [ADR 0031](0031-multi-monitor-taskbar-lyrics.md).

## Context

Taskbar lyrics only exist on Windows and are tied to the taskbar. The maintainer wants a desktop-style floating line
of lyrics that can be placed anywhere, on Windows and on the phone, with the same look options as the taskbar lyrics but
configured separately, and a way to make it ignore the mouse or finger once it is placed.

## Decision

- Settings: `settings.floatingLyrics` (`FloatingLyricsSettings`): `enabled` (off by default, also for older settings
  files), the same look fields as `taskbarLyrics` (align, font size, translation scale, custom colour, translation,
  text effect) with the same clamps, plus `locked` and `position` (physical px, Windows only). Repaired field by field
  like the taskbar settings. Both features can be on at once.
- Settings UI: the lyrics panel has tabs 一般 / 工作列歌詞 / 浮動歌詞 (the taskbar tab only on Windows); the last tab is
  kept in `localStorage` (`moe.settings.lyricsTab`). The look controls are one component (`LyricsLookSettings.svelte`)
  used by both tabs.
- Windows: one transparent always-on-top window `floating-lyrics` (`?view=floating-lyrics`, 800 CSS px wide, tool
  window, no activation) rendering through `TaskbarLyricsView` with `variant="floating"`, so the line selection,
  scrolling and effects are the same code as the taskbar lyrics. Move mode: drag anywhere (`startDragging`), the
  position is saved 700 ms after the last move; a padlock button appears on hover. Fixed mode: the whole window ignores
  the cursor; a 100 ms watcher reads the cursor position and turns cursor events back on only while it is over the
  unlock button (DPI-scaled), which is shown on hover. The window is kept inside the work area of a connected monitor
  (re-checked every 2 s), so an unplugged monitor does not strand it.
- Android: a native `TYPE_APPLICATION_OVERLAY` window (`FloatingLyricsOverlay.kt`). The page pushes the settings and
  the current track's timed lines (`floatingLyricsUpdate`); the overlay picks the line every 100 ms from the playback
  service's own player position, so it keeps going while the app is in the background. The permission is granted by
  the user on the system page (`openOverlayPermissionSettings`), never by adb or the app. Move mode: finger drag, a
  lock button. Fixed mode: `FLAG_NOT_TOUCHABLE`; Android 12+ only lets touches through windows at most 80% opaque, so
  the overlay is drawn at that opacity; it is unlocked from the player's settings only. Lock state and position are
  kept in the plugin's SharedPreferences (the source of truth on the phone).

## Consequences

- Only synced (timed) lyrics are shown; unsynced lyrics show nothing.
- Android: if the track changes while the WebView is suspended, the new track's lines arrive when the page runs again;
  until then the overlay shows nothing for the new track (it never shows the old track's lines).
- Android stroke is approximated with a tight shadow.
## Amendment (2026-10-23): reset, opacity, translation, Android drawing

- Reset position: Windows puts the window at the exact centre of the primary monitor (its full bounds, then clamped
  to the work area); the first placement without a saved position is the same. Android clears the saved overlay
  position (`floatingLyricsUpdate { resetPosition: true }`) and centres the overlay on the screen; a first overlay
  without a saved position is centred too. The mode buttons and "Reset position" share one row on both platforms.
- The translation line is no longer faded: the 80% opacity of `.taskbar-lyrics-translation` (shared by the taskbar
  and Windows floating lyrics) and the `alpha = 0.8` of the Android translation view were removed.
- `opacityPercent` (1–100, default 100) is saved separately in `taskbarLyrics` and `floatingLyrics` and applies to
  the main line and the translation alike (Windows: opacity of the lyric container; Android: window alpha). It
  replaces the fixed 80% of Android fixed mode. Android 12+ still blocks touches through an overlay above the
  system's maximum obscuring opacity (normally 0.8), so in fixed mode the window alpha is the setting capped at that
  limit; in move mode it is the setting as is.
- Android lines are drawn by `LyricLineView` instead of a single-line `TextView` (whose horizontally scrolling layout
  put short centred or right-aligned lines outside the view, so nothing was shown). A line that fits follows the
  alignment; a longer one slides with a fractional offset updated every display frame (`Choreographer`) while it
  slides during playback, otherwise every 100 ms.

## Amendment (2026-10-23): width, fitted height, status bar, 1–100% opacity

- `widthPx` (200–4000, default 800) is saved in `floatingLyrics`: CSS px on Windows, dp on Android, narrowed to the
  screen. A slider 「寬度」 on the floating tab sets it.
- The height is not a setting. Each line box is the text's own height with a 10% buffer (Windows: line-height
  1.375 em = 1.25 em × 1.1; Android `LyricLineView`: (descent − ascent) × 1.1). The window holds the main line plus
  the translation line only while the current line has one: the floating page calls `floating_lyrics_fit
  { hasTranslation }` and `floating_lyrics::fit_lines` resizes (top-left kept, then kept on the work area); the
  Android window wraps its content. The floor is the lock button (44 CSS px).
- Android: `FLAG_LAYOUT_IN_SCREEN | FLAG_LAYOUT_NO_LIMITS` and cutout mode SHORT_EDGES; clamping and centring use
  the real display size, so the overlay can be dragged over the status bar. Windows still clamps to the work area.
- `opacityPercent` for taskbar and floating lyrics is 1–100 (Rust `OPACITY_RANGE_PERCENT`, JS `LYRICS_OPACITY_PERCENT`,
  Kotlin `FloatingLyricsRules.opacityPercent`). Android fixed mode is still capped at the system limit (~0.8).
- Android keeps portrait and landscape apart: `widthPx` is the portrait width and `landscapeWidthPx` the landscape
  one (the slider, labelled 「寬度（直向）／（橫向）」, edits the current orientation); the position is saved under
  `x_portrait`/`y_portrait` and `x_landscape`/`y_landscape` in `moe_floating_lyrics`. A rotation
  (`ComponentCallbacks.onConfigurationChanged`) applies that orientation's width and saved position; with none it is
  centred. Reset clears only the current orientation. Windows has one width (`widthPx`); with no saved position the
  window is centred on the primary monitor, as an Android orientation without one is centred on the screen.
- Settings layout: the floating tab's 「對齊」 row is labelled 「文字排版」 (text alignment inside the panel,
  `LyricsLookSettings` `alignLabel`). The 「模式」 row is gone. A 「面板」 item, the same on both
  platforms except that 「狀態列」 is Android only, holds 「狀態列」, a D-pad (↑ ↓ ← →, 1 px per tap, saved per
  orientation on Android) whose centre is the lock toggle (lock icon only: locked = 固定模式, unlocked = 移動模式),
  「水平置中」 / 「垂直置中」 (one axis only, the other kept; Windows: the monitor holding the panel's centre point,
  `floating_lyrics_centre_axis { horizontal }` / `centre_axis`; Android: the screen, `floatingLyricsUpdate
  { centreAxis }`, saved for the current orientation), 「重設位置」 (Windows: still the primary monitor's centre) and
  the width slider. Windows arrows call `floating_lyrics_nudge { dx, dy }` (1 physical px, kept on the
  work area; the move event saves the position); Android sends `floatingLyricsUpdate { nudgeX, nudgeY }`.
- Android 「狀態列」 toggles a
  per-orientation mode (`statusbar_portrait` / `statusbar_landscape`): no padding, lock button as tall as the status bar
  (`status_bar_height`), main line only, font sized so its line (text height × 1.1) equals the status bar height
  (`FloatingLyricsRules.fontForLineHeight`), and the panel moved to y = 0 beside the top cutout on the roomier side
  (`statusBarX`; centred with no cutout); the position is saved for that orientation. The D-pad moves the panel 1 px,
  saved for the current orientation. `floatingLyricsStatus`
  returns `statusBar` for the current orientation; the page refreshes it on rotation. Width stays the per-orientation
  width setting.

## Amendment (2026-10-23): lock gutter, no padding, width 100–screen, D-pad repeat, smooth Android scroll

- No padding and no minimum height: the panel is only the lines (text height × 1.1, line-height 1.375 em). The lock
  button is as tall as the main line (`ceil(font × 1.375)`; the status bar height in 狀態列 mode), so it scales with
  the font and DPI and never props the panel open.
- No left gutter. The lyric area, which alone carries the move-mode background, starts at the panel's left edge; the
  lock sits in a transparent gutter on the right, as wide as the lock. `widthPx` / `landscapeWidthPx` are the
  **lyric area** width; the panel is that width plus the gutter. 水平置中, 重設位置 and first use centre the lyric
  area (not the panel) on the monitor / screen, per orientation on Android.
- Width range 100–8192 stored (CSS px / dp); on screen the lyric area is narrowed so it and the gutter fit the
  current monitor (Windows: the one holding the panel, `floating_lyrics_screen_width`) or the current orientation's
  screen width (Android `floatingLyricsStatus.screenWidthDp`). The slider's maximum is that screen width.
- 面板 layout: 「移動到狀態列」 on its own row (Android only); then the D-pad (lock toggle in the centre) with a vertical
  stack 水平置中 / 垂直置中 / 重設位置 to its right; then the width slider. D-pad: a tap moves 1 px; held past 300 ms it
  repeats 1 px every 33.3 ms (30 px/s; the hold delay stays 300 ms) until pointer up / leave / cancel (`pressFloatingLyricsNudge`); the position is saved once
  on release (Android `nudgeX/nudgeY` with `save: false`, then `savePosition: true`).
- Android scroll fps: measured with `dumpsys gfxinfo` while playing on a 120 Hz display, the overlay (already
  hardware accelerated) drew ~7 frames/s at ~5 ms each with no jank, so it was not render bound. Root cause:
  `PlaybackService.overlaySample()` returns ExoPlayer `currentPosition`, which only advances in coarse steps, and
  `LyricLineView` only invalidates when the progress changes, so the line moved only at those steps. Fix: the
  overlay carries the position on from the last report by the elapsed `uptimeMillis` while playing
  (`FloatingLyricsRules.smoothPositionMs`; a report slightly behind is held so the text never steps back, a jump of
  more than 250 ms resyncs), so every Choreographer frame has new progress and is drawn.

## Amendment (2026-10-23): shadow room, 「移動到狀態列」 is a move only

- Text shadow / glow was clipped by the tight panel. The lyric area (move-mode background; where the text is drawn
  and aligned) stays text height × 1.1 (main line plus the translation line when drawn). The whole window / overlay
  is text height × 1.5 (Windows `PANEL_LINE_HEIGHT` 1.875 em of the line font sizes; Android `panelRoomPx` = area ×
  2 / 11 above and below) with the area vertically centred; the room around it is transparent. A transparent side
  margin of 0.25 em (`SIDE_MARGIN_EM`, at least 1 px) sits left of the area and right of the lock gutter.
  The window is margin + area + lock + margin. `widthPx` / `landscapeWidthPx` are still the lyric area width;
  centring, reset and first use still centre the lyric area; the screen clamp and the slider maximum take the lock
  and both margins off the screen width. The lock keeps the main line height. Android lets the shadow draw past
  the line views (`clipChildren = false`; a line only clips its sides while it slides).
- 「移動到狀態列」 (Android) is no longer a mode. The old per-orientation status bar mode fitted the font to the bar,
  dropped the translation and set y = 0, so the first tap filled the bar regardless of the font size and the next
  tap behaved differently. Now the tap only moves the panel: the user's font size, translation line and the panel
  rules above are unchanged; y puts the panel's (hence the lyric area's) vertical centre on the status bar's
  (`centredOnBar`, may be above the screen top), x as before beside the cutout (`statusBarX`). It depends only on
  the current size, so every tap lands the same; the position is saved for the current orientation. The old
  `statusbar_<orientation>` preference and `floatingLyricsStatus.statusBar` are gone.
- 「動畫」 setting (taskbar and floating lyrics, saved apart as `animation`: `none` default / `continuous`; older
  files and unknown values read as `none`). `none` is the one-line-at-a-time look above. `continuous` joins every
  timed line with some text into one strip, lines separated by U+3000 (one em of the main line), and the translations
  into a second strip with the same cells (a cell is as wide as the wider of its line and its translation; the text is
  aligned inside it), so each translation stays under its line. The strip is shifted so the current line's anchor
  (left edge / middle / right edge for left / centre / right) sits on the same point of the lyric area, gliding
  linearly to the next line's anchor until that line starts. Amended (superseded by the centre rule further down): the strip always uses the right anchor
  (`LYRICS_STRIP_ALIGN` / `FloatingLyricsRules.STRIP_ALIGN` = right; cells' text right-aligned too), whatever the
  文字排版 setting, whose buttons stay enabled and apply to 「無」; before the first line it rests on the first, after the
  last on the last (`lyrics-strip.js` `lyricsStripOffset`, Kotlin `FloatingLyricsRules.stripOffset`). The shift is
  a pure function of the lyrics time (position + offsets), so seek and pause follow the playback clock at once.
  Amended (2026-10-23, measured): only the cells that can show during the current glide plus one lyric area of margin
  on each side are drawn (`lyricsStripWindow`, Kotlin `FloatingLyricsRules.stripWindow`); each keeps its layout offset
  (absolutely placed on Windows, widths measured with a canvas), so the swap at a line change moves nothing. A strip of
  the whole song cost about 20x the CPU/GPU per frame (Edge test: 120 lines ~2.5 s CPU/s vs 9 lines ~0.13 s/s) and,
  since all WebView2 windows share one GPU process, slowed the main window, more so with a taskbar window per monitor.
  Amended (2026-10-23, user decision; supersedes the right anchor): the anchor is the lyric area's horizontal CENTRE. When
  a line starts, its LEFT edge sits exactly on the centre, and the strip slides left linearly so the next line's left
  edge reaches the centre exactly when that line starts (`LYRICS_STRIP_ALIGN` = left, `LYRICS_STRIP_TARGET` = center;
  Kotlin `STRIP_ALIGN` / `STRIP_TARGET`; `lyricsStripOffset` / `stripOffset` take the target apart from the anchor). It
  depends only on the lyric area's width, not on the panel width, and ignores the 文字排版 setting (buttons stay
  enabled); cells' text is left-aligned. The lines before the current one fill the left half, so the window margin
  still covers both sides.
  Windows runs each glide as a linear CSS animation re-planned at every line start and clock correction; Android
  redraws the strip in `LyricLineView` every display frame while playing. The strip is cut at the lyric area's sides.

## Amendment (2026-10-23): continuous animation dims the lines that are not current

- In the continuous animation the current line keeps the window's own 「透明度」 (taskbar / floating `opacityPercent`,
  applied to the whole window as before). Every other line that can be seen (the previous and next one, and any
  further ones in view) is also multiplied by the shared lyrics setting 「非目前歌詞透明度」
  (`lyricsPreferences.inactiveOpacityPercent`, 10–100, default 50, the one Now Playing already uses; no new setting).
  Example: main 80%, not-current 50% -> 40%. The translation strip follows its own line. The animation 「無」 is
  unchanged (`lyricsLineOpacityFactor` returns 1 for it).
- The fade is a function of the lyrics position, not an event timer. Over the last 200 ms before a line starts
  (`LYRICS_FADE_MS`; shortened to the current line's own length when that is shorter) the current line fades linearly
  from 1 to the not-current factor and the next from the factor to 1, so both finish at the very moment the glide
  reaches the next line. Before the first line nothing is dimmed (every line is 1, like Now Playing). So
  that the first line's start is no jump, the same 200 ms rule applies to it: in the 200 ms before the first line
  starts (shortened so it never begins before position 0) every line except the first fades from 1 to the
  not-current factor, and the first stays 1; at its start it is the current line and the others are already at the
  factor. From the last line's start nothing fades (it stays 1). Android computes the same weights from its own position (`FloatingLyricsRules.cellWeight`), so a prelude shorter than 200 ms starts already part-way through the first-line lead-in at position 0, whereas Windows shortens the fade so it never begins before position 0. Pure functions in `lyrics-strip.js`:
  `lyricsStripFocus(startTimesMs, positionMs)` -> `{ current, incoming, progress, fadeStartMs, fadeMs }`,
  `lyricsLineWeight(focus, index)` (0 not current ... 1 current) and
  `lyricsLineOpacityFactor(weight, inactiveOpacityPercent, animation)` = `inactive + (1 - inactive) * weight` (the
  percent goes through the existing normalize). A seek, a pause or a resume re-plans like the glide does: while
  playing, the cells whose weight changes in the fade (the two lines, or before the first line every line but the first) get one opacity-only CSS animation (delay = fadeStart - position, negative when
  already in progress, fill both; names alternate a/b so a re-plan restarts it); when paused the static value at
  that position is shown. All other cells carry a static `opacity`.
- Cost: opacity only (no layout, no text repaint); one `opacity` per cell of the already windowed strip (about 9
  cells), no `will-change` per cell (no extra layer per line); the compositor makes a layer for the two animated
  cells only during their 200 ms. The window opacity stays on the container, so main x not-current needs no second
  multiplication.
- How the value reaches the windows (Windows): the main window (the settings live there) broadcasts the event
  `lyrics-inactive-opacity` (payload: the integer percent) whenever `lyricsPreferences.inactiveOpacityPercent`
  changes (so the slider's unsaved preview shows live) and when a lyrics window may have opened; each window also reads
  `settings_get_lyrics_preferences` once when it starts and runs the value through `normalizeLyricsPreferences`. No Rust
  change.
- Android contract (the overlay draws natively): the page sends `floating_lyrics_android` / `floatingLyricsUpdate` with
  the payload `{ "inactiveOpacityPercent": <integer 10..100> }` as its own top-level key (next to `settings`, `data`,
  `setLocked`...), on every change of the lyrics general setting and when the app starts (it is not part of
  `settings`, which stays the floating look). Unknown to an older overlay, which ignores it. The overlay must run the
  same rule on its own smoothed position (`position + offsetMs`): factor of a line = `inactive + (1 - inactive) *
  weight`, weight from the focus above with a 200 ms fade ending at the next line's start, multiplied by the overlay's own
  opacity.

## Amendment (2026-10-23): text-shadow on the text, no parent filter

- Decision: the text shadow of the taskbar lyrics and the Windows floating lyrics (one component,
  `src/lib/TaskbarLyricsView.svelte`) is a single-layer `text-shadow` on the text elements themselves
  (`.taskbar-lyrics-track`, `.taskbar-lyrics-strip-text`); there is no `filter` / `drop-shadow` on any parent. The blur radius
  is `--taskbar-lyrics-shadow-blur` (0.3em, the old filter's standard deviation) x 2 to keep the old look. In shadow mode
  the clipping containers (`.taskbar-lyrics-main`, `-translation`) use `overflow: clip` with `overflow-clip-margin` equal
  to the shadow radius and no padding (scrolling and alignment measure `clientWidth`).
  `tests/taskbar-lyrics-single-shadow.test.mjs` checks: no filter / drop-shadow, a single `text-shadow` layer, the animated
  cells are not under a filter, no new `will-change`.
- Reason: in the continuous animation a parent filter recomputes the blur of the whole area on every frame (and whenever the
  fading opacity changes), and all WebView2 windows share one GPU process. The maintainer measured that, with continuous
  lyrics open, the album-detail cover collapse in the main window dropped frames badly (one lyrics window: low frame rate;
  two: frozen), while animation 「無」 was fine.
- Trade-off: the shadow shape differs slightly from the old filter; the shadow at a clipped edge is kept at most 0.6em.
  Android is not affected (native drawing).
- Status: not yet verified on the maintainer's machine. If it still drops frames, the next candidates are giving the lyrics
  windows their own `data_directory` (a separate WebView2 process group including its own GPU process) and lowering the lyrics
  windows' update rate.

## Amendment (2026-10-23): continuous-mode outline and terminal line glide

- When the selected text effect is `shadow` and animation is `continuous`, render a crisp outline instead of a blur to
  avoid re-rasterizing blur during motion. This is a rendering override only: the saved choice stays `shadow`; animation
  `none` and explicit `stroke`/`none` choices keep their selected appearance. Windows uses the existing stroke style;
  Android draws a stroke pass before the fill pass.
- The last line keeps gliding from its left-edge anchor at line start. Estimate its travel time as 500 ms per Unicode
  grapheme (user decision); if the track ends sooner, cap the glide at track end. At arrival, measure the final
  grapheme's visual bounds and put that glyph-bounds centre at the lyric-area centre. Lyrics display offset is included
  in the lyric-time endpoint. When track duration is unknown, use the grapheme estimate without a duration cap.
- Validation: JavaScript geometry tests cover line start, midpoint, estimated arrival, track-end cap and grapheme-centre
  position; Android `FloatingLyricsRulesTest` covers the 500 ms estimate, cap, grapheme counting and effect selection.
  Release WebView2 / device runtime check is tracked with the current build status in the project `AGENTS.md`.

## 更新（2026-10-23，ADR 0034）

Windows 的浮動歌詞與工作列歌詞改為原生 DirectComposition 繪製（見 ADR 0034），WebView 視窗已移除；行為規則不變，唯一刻意的改變：連續模式選陰影時，Windows 原生版照畫陰影（點陣化一次，沒有每幀成本）。2026-10-24 起 Android 也一樣：每種模式都畫使用者選的效果；`LyricLineView` 把每行連同陰影／描邊點陣化成一張 Bitmap（`LineRasterCache`，外觀〔字級、顏色、效果、密度〕改變就全部重建，只保留目前條狀視窗內的行），滑動與淡化只以新位置與透明度畫該圖，不每幀模糊；陰影對齊 Windows（σ 0.3 em、文字反色 75%），描邊不變；EdgeFadeLayout 的 DST_IN 照樣淡化陰影。

## Amendment 2026-10-23: C1 glide curve

The continuous strip no longer glides piecewise linearly (the speed jumped at every line start). The offset is a
monotone C1 cubic Hermite (`lyricsStripCubic` / `strip_cubic` / `FloatingLyricsRules.stripOffset`) through the knots
(line start, that line's left edge on the lyric-area centre) plus the terminal knot (final arrival, last glyph centre):

- Knots are exact: at each line start that line's left edge is on the centre; at the final arrival (500 ms per
  grapheme, clipped by the track end) the last glyph centre is.
- Tangents: 0 at the first knot (the strip rests before the first line, so the lead-in starts from rest) and at the
  last knot (it eases to a stop at the terminal arrival, or at the last line start when there is no terminal glide).
  Inside, the weighted harmonic mean of the two neighbouring slopes (Fritsch-Butland, as in PCHIP; weights
  `2h1+h0`, `h1+2h0`), and 0 when either slope is 0 or a neighbour has the same time (a jump). The harmonic mean is
  pulled toward the smaller slope and never exceeds 3x it, which satisfies Fritsch-Carlson, so the glide never goes
  backwards and a very short line (huge slope) does not make its neighbours rush; a long instrumental gap (tiny slope)
  makes the strip slow down around it instead of carrying the previous speed into it.
- Locality: a segment depends only on its four nearest knots, so the curve is one global function of the lyrics time.
  Seeks and pauses evaluate it at the new position as before, and the Windows plan (which covers the next three line
  starts) samples the same curve at every re-plan, so hand-overs have no kink.
- The 200 ms opacity crossfades are unchanged. Non-left anchors (not used by the app) take
  max(last anchor, last cell start + glyph centre) as the terminal value so the curve stays monotone.

## Edge fade (2026-10-23)

The taskbar and floating lyric areas (Windows and Android, both animation modes) fade out at the left and right: a
square zone on each side, as wide as the area is high (half the width when the area is narrower than twice its height),
where everything the lyrics draw (main line, translation, shadow, outline) falls linearly from opaque at the inner
edge to transparent at the outer edge. The padlock column and the move-mode background are not faded. Since the outer
edge is transparent, the horizontal clip room kept for the shadow at the area's sides is gone. Rules:
`player-lyrics-rules::edge_fade_*`, `lyricsEdgeFade*` (lyrics-strip.js, shared vectors `edgeFade`),
`FloatingLyricsRules.edgeFade*`. Android masks the lyric column (`EdgeFadeLayout`) with a `LinearGradient` in
`DST_IN` on a saved layer. Windows (from 2026-10-23) renders the lyric content once per window into a `CompositionVisualSurface`
and paints it through a `CompositionMaskBrush` whose mask is a static horizontal gradient from the same rule, which
needs Windows.UI.Composition and so Windows 10 1809 or later (older systems get no lyrics windows). The first Windows
version used static clipped bands, each with its own copy of the lyrics (about 1,300 visuals per window). With 3
windows on a 120 Hz display, dwm.exe used about 95% of one core and composed only 33 frames/s. With the mask it is
within noise of having no fade (about 25% vs 28% here, at full 120/60 Hz; see ADR 0034). Since 2026-10-24 the
Windows renderer samples the glide, fades and scroll itself on every composition frame and commits only values that
changed by at least one device pixel (or 1/255 opacity); see ADR 0034.

## Placement clamp and width changes (2026-10-24)

The floating panel is kept on screen by its lyric text area, not by the whole window: the transparent side margin
left of the lyric area, the margin right of the padlock (so the padlock itself stays reachable) and the room above
and below the lyric area may hang past the work area (Windows `place_on_monitors_with` with `text_insets`, used by
every placement: drag check, saved position restore, nudge, centre-axis, reset and the 2 s monitor check; Android
`FloatingLyricsRules.clampPosition` with insets, `textVerticalInset`). A width change (寬度) keeps the lyric area's
horizontal centre (`resize_around_centre` / `resizeAroundCentre`, the centre that 水平置中 aligns), then the clamp
shifts the panel just enough and the result is saved like a drag; on Android per orientation, and a rotation's own
width change does not count as a resize.

Mouse dragging in move mode (Windows, 2026-10-24) no longer goes through `HTCAPTION`: the system caption drag keeps the
window on screen and stopped before the transparent margins could leave it. The window now captures the mouse on a
button press outside the padlock and follows the cursor with `SetWindowPos`; on release (`WM_CAPTURECHANGED`) the app
applies the placement rule once (`settle_after_drag`), and the move events still save the position 700 ms after moving
stops. Android's drag already clamps with the text insets on every move.
