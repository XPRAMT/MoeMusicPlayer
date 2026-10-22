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
