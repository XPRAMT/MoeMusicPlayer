# ADR 0004: M3U files as synchronized library sources

## Status

Accepted

## Context

An imported M3U/M3U8 must continue to contribute media to the library and reflect later external edits. A playlist entry may be absent from every folder source, repeated in one file, shared with folders or other lists, temporarily unavailable, or malformed at the individual audio-tag level. Clearing a source mapping must not erase a Track still provided elsewhere.

## Alternatives considered

1. Keep import as a one-time snapshot. This avoids synchronization state, but external edits never reach the application and playlist-only tracks cannot recover when they appear later.
2. Treat the playlist file as a folder root. This reuses the recursive scanner, but it scans unrelated files, cannot preserve repeated entry order, and couples source reconciliation to directory contents.
3. Treat the file as a first-class source that projects ordered entries and deduplicated media mappings. This adds a narrow parser/scanner path but aligns reconciliation with the file's actual membership and reuses canonical Track identity.

## Decision

A registered playlist file owns its projected playlist entries and one source mapping for each unique local media locator. The path and Playlist ID are registered in JSON; SQLite stores the playlist projection, source mappings, per-file sync state, and scan results. At each startup, background sync checks playlist path, size, modification time, and content digest; it parses and projects only when changed, while it stats every unique referenced audio path and reads tags only when that file's size or UTC modification time changed. Repeated playlist entries retain their order but share one source mapping and Track ID. Each readable, syntactically valid file reconciles entries and mappings; an entry removed from the file drops only that playlist source mapping. Tracks remain in the library while any other enabled source mapping exists. A missing or unreadable individual song retains its locator and previous mapping while other songs update. A playlist file that cannot be read or parsed does not reconcile or replace the previous projection. The external file is authoritative and the application does not write back to it automatically.

## Consequences

- A file shared by folder and playlist sources resolves to one Track ID; playlist duplicates remain separate ordered entries.
- A playlist may be the only source for a playable Track.
- A fully readable source update removes stale references from that source without deleting a Track still referenced elsewhere.
- Per-source and per-track fingerprints are needed to keep startup work incremental; content digest protects against same-size, same-timestamp playlist edits.
- Removing the playlist source registration must detach only that source's mapping; ordinary playlist rows and media from other sources remain independent.
- Re-importing the same canonical file path reuses its registered Source ID and Playlist ID. A legacy static playlist is reused only when its name and complete ordered locator sequence match; same-name content that differs is preserved as a separate playlist.
- If neither the settings JSON nor its backup is valid, the source registry is non-authoritative. Startup skips synchronization and destructive reconciliation until the user re-registers sources and confirms the recovered registry.
