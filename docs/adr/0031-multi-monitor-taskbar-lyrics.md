# Taskbar lyrics on every taskbar, one window per taskbar

Status: Accepted and implemented on 2026-10-23

Decision date: 2026-10-23, Asia/Taipei (UTC+08:00)

Extends [ADR 0018](0018-taskbar-lyrics.md).

## Context

Taskbar lyrics had one window on the primary taskbar (`Shell_TrayWnd`). With "show taskbar on all displays" Windows
adds a `Shell_SecondaryTrayWnd` per other monitor. The maintainer's setup: DISPLAY1 3840×2160 at 240 DPI (primary) and
DISPLAY6 2560×1440 at 192 DPI to its right, lower edge 8 px below the primary's. A single window spanning both cannot
work: each monitor has its own DPI, its own free stretch and its own fullscreen state, and a window can only be owned
by one taskbar.

UI Automation on the secondary taskbar shows its Buttons (Start, search, task view, apps) and the clock as a
`SystemTray.AccentButton` Button. There is no `TrayNotifyWnd`, so the occupied list is just the Buttons.

## Decision

- **One independent window per taskbar** (labels `taskbar-lyrics-N`, capability glob `taskbar-lyrics-*`), owned by
  that taskbar (`GWLP_HWNDPARENT`), each with its own probe, DPI and covered check. The watcher re-enumerates the
  taskbars on every pass (about 400 ms), so hot-plug and Explorer restarts destroy and create windows. It runs
  `SetWindowPos` again when the DPI changed or the real rectangle differs.
- **Covered:** the primary keeps ADR 0018's rule. A secondary taskbar only uses "the foreground window covers this
  monitor", because `SHQueryUserNotificationState` is global. Auto-hide stays global (hide everywhere). Vertical
  taskbars stay unsupported.
- **`taskbarLyrics.displayMode`:** `all` (default, also for old settings files), `playerMonitor` (the main window's
  monitor, falling back to the primary), `custom` (`customMonitors`, stable IDs). Offline monitors keep their
  selection. An empty custom list shows nothing.
- **Stable ID:** the target `monitorDevicePath` from `DisplayConfigGetDeviceInfo`, falling back to the GDI device
  name. It changes if the monitor is moved to another port.
- **Numbering:** Windows has no public API for the number shown in Settings → Display. We use the order of the
  active paths from `QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS)`, counting a cloned source once, and fall back to the
  `EnumDisplayMonitors` order. The settings page tells the user to check with "Identify".
- **Data:** appearance is shared. The main window broadcasts lyric data and playback samples with `emit`; Rust
  broadcasts settings and theme with `app.emit`. Each new window sends `taskbar-lyrics-request` when it starts.
  Closing the main window or `shutdown` destroys every `taskbar-lyrics*` window.

## Consequences

- Pure logic is tested: `lyrics_area` (per-DPI margins, both monitors' real free stretches), `number_monitors`,
  `wants_taskbar` and the settings migration.
- `playerMonitor` re-creates windows when the player moves between monitors (one poll later).
- The numbering may differ from Windows' in unusual setups; the user can still pick by name and resolution (button
  tooltip).
