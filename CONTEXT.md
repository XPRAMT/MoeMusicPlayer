# Library and Playback Language

This glossary defines the music-library identity and playback terms shared by the Windows and Android flows.

## Library

**Track**: A logical music item the user can browse or play. One track can be reachable through more than one configured source.

**Track ID**: The stable application identity of a track. It remains the reference used by playlists and playback when a source-specific item or locator changes.

**Source mapping**: The relationship between a track and one item observed in a configured source. It carries source identity and the locator/fingerprint used to synchronize that item.

**Media locator**: A platform-specific address for reading media, such as a filesystem path or a content URI. It belongs to the native side of the application and is not a renderer-facing track identity.

## Playback

**Playback session**: The currently selected track together with the state reported by the audio backend. It is separate from the full library and any future queue.

**Playback snapshot**: A point-in-time view of the current playback session, including state, position, duration, volume, and any current error. The audio backend is authoritative for these values.

## Playlists

**Playlist**: A user-owned ordered set of media entries. An entry may link to a stable Track ID and retains its imported media locator when it has no current library mapping.

**M3U/M3U8**: Text interchange formats for ordered playlist entries, including path and optional display metadata. A playlist file can include entries that are not currently present in the application library.

## Operating-system media controls

**System media control**: Windows' operating-system-facing representation of the current playback session. It shows supported metadata and sends transport requests; the application audio backend remains authoritative for playback state.
