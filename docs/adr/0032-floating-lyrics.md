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
- `opacityPercent` (10–100, default 100) is saved separately in `taskbarLyrics` and `floatingLyrics` and applies to
  the main line and the translation alike (Windows: opacity of the lyric container; Android: window alpha). It
  replaces the fixed 80% of Android fixed mode. Android 12+ still blocks touches through an overlay above the
  system's maximum obscuring opacity (normally 0.8), so in fixed mode the window alpha is the setting capped at that
  limit; in move mode it is the setting as is.
- Android lines are drawn by `LyricLineView` instead of a single-line `TextView` (whose horizontally scrolling layout
  put short centred or right-aligned lines outside the view, so nothing was shown). A line that fits follows the
  alignment; a longer one slides with a fractional offset updated every display frame (`Choreographer`) while it
  slides during playback, otherwise every 100 ms.