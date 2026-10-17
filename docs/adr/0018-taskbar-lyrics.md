# Taskbar lyrics: a click-through window over the free part of the taskbar

Status: Accepted

Decision date: 2026-10-18, Asia/Taipei (UTC+08:00)

## Context

The user wants the current lyric line shown on the Windows taskbar, in the empty stretch between the taskbar buttons and the notification area, with its options on Settings → Lyrics. Windows has no API for putting content on the taskbar. Making a window a child of `Shell_TrayWnd` is fragile on Windows 11, where the buttons are drawn by XAML, and WebView2 inside such a child is worse. Measured on this machine (Windows 11, 240 DPI, 3840×2160): the taskbar is `Shell_TrayWnd` (0,2080 to 3840,2160 physical px), its buttons are UI Automation `Button` elements (widgets, start, search, one per task, the tray icons) with exact physical rectangles, and `TrayNotifyWnd` still exists as a window. The tasks are centred, so the free stretches are on both sides of them (here 395 to 1506 and 2334 to 3105).

## Decision

- **A separate, transparent, click-through, always-on-top Tauri window** (label `taskbar-lyrics`, the same bundle with `?view=taskbar-lyrics`) is placed over the free stretch. It has no decorations, no shadow and no taskbar button, never takes focus (`WS_EX_NOACTIVATE`, `WS_EX_TOOLWINDOW`) and ignores the mouse, so taskbar clicks pass through to the taskbar underneath. It is created hidden and shown only while a stretch is available.
- **Free stretch = taskbar bounds minus the union of its buttons.** `player-platform-windows::taskbar` asks UI Automation (`IUIAutomation::FindAll`, descendants of `Shell_TrayWnd` with control type Button) for every button's rectangle, adds `TrayNotifyWnd`, merges the intervals along the taskbar's long axis and takes the gaps. A gap must be at least 120 CSS px wide after an 8 CSS px margin on each side. Settings choose the stretch: the widest (default), the leftmost or the rightmost. The same code covers Windows 10 (toolbar buttons are Buttons too) and a left-aligned Windows 11 taskbar (one gap, between the last task and the tray). Vertical taskbars are not supported: the window stays hidden.
- **A watcher thread** re-probes about every 400 ms (the buttons change when apps open or close), moves and resizes the window with `SetWindowPos` in physical pixels, and re-asserts `HWND_TOPMOST` every tick because the taskbar raises itself over other topmost windows when it is clicked. The window is hidden when the taskbar auto-hides, when a full-screen app or presentation is running (`SHQueryUserNotificationState`), when no stretch fits, and when the taskbar is not visible. The thread also reads the taskbar's own light/dark theme (`SystemUsesLightTheme`; the taskbar follows this, not the apps theme) and tells the window when it changes.
- **The window owns its lyric state.** The main window's lyrics view only exists on the Now Playing page and its timers are throttled when the window is minimized, so the taskbar window polls `playback_get_snapshot` itself (about every 200 ms, extrapolating the position between polls), loads lyrics for the current track with the existing lyrics controller (local, embedded, cached online, automatic search), and applies the existing lyrics preferences (Simplified to Traditional) and the per-track timing offset (`localStorage`, shared by every window of the app). It shows the active line of synced lyrics; plain lyrics, no lyrics and a stopped player show nothing.
- **Settings** (`settings.json` → `taskbarLyrics`, defaults off): enabled, stretch (auto/left/right), alignment (left/centre/right), font size 10–32 CSS px (default 16), colour (automatic black/white by the taskbar theme, or a custom `#rrggbb`) and a second smaller line with the translation. They are shown on Settings → Lyrics on Windows only and applied immediately; the saved value is broadcast as the `taskbar-lyrics-settings` event.
- Closing the main window destroys the taskbar window so it cannot keep the process alive.

## Consequences

- The text is drawn by the webview over the taskbar, not by the shell: it is not part of the taskbar's accessibility tree, and a window that is also topmost can briefly cover it.
- If Explorer restarts, the next probe finds the new `Shell_TrayWnd`.
- Per-monitor taskbars (secondary monitors) are not used; only the primary taskbar gets lyrics.
- Android has no taskbar; the settings section is hidden there.
