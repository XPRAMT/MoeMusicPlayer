# Library and Playback Language

This glossary defines the music-library identity and playback terms shared by the Windows and Android flows.

## Library

**Track**: A logical music item the user can browse or play. One track can be reachable through more than one configured source.

**Track ID**: The stable application identity of a track. It remains the reference used by playlists and playback when a source-specific item or locator changes.

**Source mapping**: The relationship between a track and one item observed in a configured source. It carries source identity and the locator/fingerprint used to synchronize that item.

**Media locator**: A platform-specific address for reading media, such as a filesystem path or a content URI. It belongs to the native side of the application and is not a renderer-facing track identity.

**「演出者」 (artist)**: The user-facing artist value sourced from the embedded `ARTIST` tag. It is distinct from the separate `PERFORMER` tag; `PERFORMER` is not a substitute for `ARTIST`.

**Year**: The displayed year is derived only from the embedded `YEAR` value. Accepted forms are a four-digit year (`YYYY`) or a valid full date (`YYYY-MM-DD`); other forms do not define a display year.

**App settings**: User-owned preferences stored in a versioned JSON document, including theme colors, shuffle, repeat mode, and the source registry. JSON is the authority for these values; SQLite rows are rebuildable music-data projections.

**Source registry**: The stable list of enabled or disabled folders and explicitly registered playlist files. A playlist-file entry associates its source ID and native path with the internal Playlist ID; static playlists do not imply a source-file path.

## Playback

**Playback session**: The currently selected track together with the state reported by the audio backend. It is separate from the full library and from the ordered Playback queue.

**Playback snapshot**: A point-in-time view of the current playback session, including state, position, duration, volume, and any current error. The audio backend is authoritative for these values.

**Playback queue**: A session-local ordered sequence of stable Track IDs drawn from the active library query or a user playlist. Library order follows the visible library sort; playlist order follows entry order and keeps repeated playable entries. Queue IDs stay in Rust and are not copied into the Renderer.

**Shuffle**: A queue traversal order that keeps the current entry in place, visits each remaining entry once per cycle, and supports previous/next traversal through the same order. Turning shuffle off resumes source order at the current entry.

**Repeat mode**: `off` stops after the final queue entry, `all` wraps the queue at its ends, and `one` repeats the current track only when it reaches its natural end. Manual next still advances in every mode.

## Playlists

**Playlist**: An ordered set of media entries with a stable Playlist ID. A playlist can be a static in-app list or the current projection of a registered playlist-file source.

**Playlist-file source**: A registered M3U/M3U8 file with a stable Source ID and Playlist ID. The external file is authoritative for its projected entries; synchronization reads it and never writes changes back automatically.

**Playlist projection**: The last successfully parsed ordered entries from a playlist-file source. A file-level read or parse failure preserves the previous projection. A missing or unreadable individual track preserves its locator and any prior source mapping while other entries continue syncing.

**Playlist media reference**: A local media locator listed by a playlist source. Repeated references remain repeated at their original positions, while their source mapping is deduplicated by canonical locator identity and reuses the same Track ID as folder or other playlist sources.

**M3U/M3U8**: Text interchange formats for ordered playlist entries, including path and optional display metadata. A playlist file can include entries that are not currently present in a folder source and can therefore contribute playable library tracks itself.

## Operating-system media controls

**System media control**: Windows' operating-system-facing representation of the current playback session. It shows supported metadata and sends transport requests; the application audio backend remains authoritative for playback state.

## Lyrics

**Lyrics document (歌詞文件)**: One lyric text associated with a track. It may contain line timing or untimed text and may come from a local file, embedded track content, or an online provider.
_Avoid_: lyrics file, when referring to embedded or provider content.

**Line timing (逐行時間)**: The playback position associated with the start of one lyric line. It supports line-level synchronization and does not imply word-level timing.
_Avoid_: word timing.

**Provider candidate (Provider 候選)**: An alternate lyric document returned by an online lyrics provider for a track, available for evaluation or user selection.
_Avoid_: search hit, when referring to a candidate that includes lyric content.

**Manual selection (手動選取)**: The user's explicit choice of a provider candidate for a track. It records a deliberate preference distinct from a result selected automatically.
_Avoid_: automatic match.

**Automatic lyrics cache (自動歌詞快取)**: A provider result saved after the track match meets the automatic-selection policy. It is replaceable cache data and does not represent an explicit user choice.
_Avoid_: manual selection.
