# ADR 0010: Explicit favorites write-back to playlist files

## Status

Accepted

## Context

Imported M3U/M3U8 files are normally treated as external authoritative sources and are not modified by background synchronization. The player also needs to let a listener add or remove the current song from several existing playlist files without flattening duplicates, changing unrelated order, or persisting an ephemeral File Explorer track ID into the library database. Android requires a persisted SAF write grant for a document tree.

## Alternatives considered

1. Keep playlist files read-only and maintain favorite membership in a separate application database. This is safe for external files but does not update the user's actual playlist.
2. Rewrite and globally sort the entire playlist after a toggle. This changes the order of existing entries, including intentionally unsorted lists.
3. Treat an explicit checkbox action as authorization to modify only that playlist file, inserting a new path at its natural-sort position while keeping every existing entry in the same relative order. This gives the requested behavior and preserves the rest of the playlist.

## Decision

The playback dock favorite control shows every enabled M3U/M3U8 source with an independent checkbox. Membership is determined by the current song's path, and a checked state is loaded from the playlist projection. Toggling one checkbox reads and parses that one file, resolves path identities, inserts only a missing song before the first naturally later path or removes that song's entries, serializes the unchanged relative order of all other entries, writes the selected file, and updates only its SQLite playlist projection. It does not start a full library scan.

On Windows, MyLove.m3u8 is created in portable UserData and registered as an enabled playlist source. On Android, the first setup opens the system folder picker initially at the shared public Music folder; the user may choose another folder. The file is stored in that shared folder so the file survives app removal. Adding an Android playlist source requests and persists both SAF read and write grants. If an older registered source has no write grant, its checkbox remains disabled with guidance to add the source again and grant write access.

## Consequences

- The database projection remains a cache of the selected playlist file and is updated after explicit writes; normal synchronization stays read-only.
- Existing playlist entries retain their relative order, and the new entry is placed using natural numeric path comparison.
- Duplicate entries for the selected song are treated as one membership: checking is idempotent and unchecking removes its duplicate slots.
- The same song may be selected in multiple independent playlist sources.
- A File Explorer song that exists only in the in-memory SQLite catalog is written to the M3U by path; its temporary TrackId is not persisted.
- Android file access depends on the persisted SAF grant. A user can choose a folder other than public Music in the system picker.
