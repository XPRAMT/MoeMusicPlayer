# ADR 0003: JSON user settings and source registry

## Status

Accepted

## Context

Theme colors, playback modes, and configured source locations are user choices. The application also needs SQLite records for indexed tracks, playlist entries, source mappings, and scan state. Keeping both kinds of data in SQLite made it unclear which copy was authoritative and made settings difficult to inspect or back up independently.

Existing installations store theme colors and configured library roots in SQLite. The newer playlist-file source flow also needs a stable link between a source path and its internal `PlaylistId`; an older static playlist does not prove which file it came from.

## Alternatives considered

1. Keep SQLite as the canonical store and export JSON for portability. This leaves the requested JSON settings file as a stale secondary copy and requires SQLite for preferences.
2. Write JSON and SQLite as equal peers. A crash between writes can leave conflicting theme, mode, or source values with no reliable winner.
3. Make versioned JSON authoritative for user settings and source registration; retain SQLite only for music data and derived source/sync projections. This gives each data class one owner and supports atomic settings replacement.

## Decision

- Store `settings.json` beside the app database in the platform app-data directory. The file has a schema version, theme colors, shuffle, repeat mode, and a registry of stable source IDs.
- Folder entries store source kind, enabled state, display name, and a native locator. On Windows, filesystem paths are encoded as UTF-16 code units so the JSON round trip does not pass through lossy UTF-8. Playlist-file entries store the path and the associated internal `PlaylistId`.
- SQLite remains authoritative for tracks, playlist contents, source mappings, and scan diagnostics. Its library-root rows are projections used by synchronization, not the source registry. Existing root rows are imported once when the JSON file is first created. Existing static playlists are not inferred to have an original file path; only a new explicit M3U/M3U8 import creates a playlist-file registry entry.
- Theme and playback mode changes update JSON before the IPC reports success. Source mutations update JSON first and then refresh their SQLite projection. If projection refresh fails, startup reconciliation can retry from JSON.
- Writes use a temporary file in the same directory, flush the file, preserve the previous valid document as `.bak`, and replace the settings file. A valid backup is restored when the primary file is corrupt. Corrupt files are preserved under a unique `.corrupt` name. If no valid backup exists, defaults are loaded with an explicit warning; legacy SQLite roots are not re-imported after JSON has once been created, to avoid reviving removed sources.
- Unknown future schema versions fail without overwriting the file. Known older JSON shapes are upgraded in memory and written back using the current version.

## Consequences

- User preferences and configured source paths have a single JSON authority; SQLite can be rebuilt from the registry without losing track metadata or playlist contents.
- Source registration and its SQLite projection are not one cross-store transaction. Startup reconciliation repairs a partial projection, while JSON mutation remains the commit point.
- Backups should include both `settings.json` and the SQLite music database. Existing installations receive a one-time migration from SQLite before the legacy theme table is retired.
- Old imported playlists remain ordinary static playlists until the user explicitly imports the source file again; no source path is guessed from playlist entries.
