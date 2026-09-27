# ADR 0001: Playlist exchange and Windows media-control boundaries

## Status

Accepted for the current Windows implementation.

## Context

M3U/M3U8 import and export must preserve ordered entries, Unicode paths, and entries that are not currently mapped into the library. Windows System Media Transport Controls (SMTC) must expose only actions the player can actually perform. The Renderer must not become a file parser, path resolver, database client, or source of playback truth.

## Decisions

- Parse and write playlist files in Rust. Import and export use Tauri's native file dialog from the Rust boundary; Renderer commands carry playlist IDs and bounded page data, never playlist locators or file contents.
- Persist playlist order and entry metadata in SQLite. Store an optional stable Track ID plus the imported native locator so unmatched or temporarily offline entries survive import and export. Resolve a Track ID only from a unique exact locator mapping; do not guess identity from tags or filenames.
- Resolve relative M3U paths against the imported playlist file's parent directory. Export M3U8 as UTF-8; use absolute paths unless a user-selected relative root can safely express every entry.
- Keep the audio backend authoritative. The Windows SMTC adapter runs its WinRT integration on its own thread and reports transport requests through a bounded event channel. The Tauri shell translates supported requests into existing player commands and periodically publishes the backend snapshot and safe metadata.
- Do not enable previous/next system controls until queue navigation exists. Enable seek only while a loaded track has a usable duration. Do not send paths, Track IDs, or artwork through SMTC.
- Report SMTC as ready only when its Windows controller attaches successfully; an audio engine starting does not imply that operating-system controls are available.

## Alternatives considered

- Parsing playlist file content in the Renderer would mix file IO and format rules into the UI and allow playlist paths/content to cross the IPC boundary. Rejected.
- Store only Track IDs would discard valid M3U entries that are not currently in the library or whose source is offline. Rejected.
- Letting SMTC callbacks call audio commands directly would couple the Windows adapter to player lifetime and make callback behavior the source of playback state. Rejected.
- Enabling every SMTC transport action by default would advertise unavailable queue behavior. Rejected.

## Consequences

Playlist parsing, persistence, and export can be tested without a GUI. The Renderer receives only paged entry summaries. SMTC integration remains optional and independently diagnosable. Windows GUI behavior still requires later interactive acceptance; compile and unit tests do not establish that the dialogs or system media flyout work on a user's desktop.