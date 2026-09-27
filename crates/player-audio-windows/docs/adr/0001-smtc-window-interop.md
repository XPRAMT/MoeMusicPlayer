# Publish the Windows system media session through window interop

The current audio path uses Rodio/CPAL rather than WinRT `MediaPlayer`, so the crate manually publishes its authoritative playback snapshot and metadata through `ISystemMediaTransportControlsInterop::GetForWindow` on an isolated worker. This keeps decoding and app identity in the existing audio/core layers while using the Tauri top-level HWND as the Windows session identity; `Next` and `Previous` remain disabled until a queue can fulfill them.

## Considered options

- **Use WinRT `MediaPlayer` automatic integration:** simpler SMTC ownership, but would require replacing or adapting the current Rodio/CPAL playback path and changing how loaded local files reach the native player.
- **Use `GlobalSystemMediaTransportControlsSessionManager`:** it discovers and controls global media sessions for a controller app; it does not publish this app's window-bound player session.
- **Use manual window-bound SMTC interop:** selected because it supports a native audio backend, updates metadata/status/timeline directly, and exposes commands without moving playback or queue ownership into the system-control adapter.

The selected API is documented for desktop apps in [Microsoft's WinRT desktop API guidance](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/winrt-api-desktop-app-support) and [`ISystemMediaTransportControlsInterop::GetForWindow`](https://learn.microsoft.com/en-us/windows/win32/api/systemmediatransportcontrolsinterop/nf-systemmediatransportcontrolsinterop-isystemmediatransportcontrolsinterop-getforwindow). The binding uses `windows-rs` 0.62.2, licensed MIT OR Apache-2.0.
