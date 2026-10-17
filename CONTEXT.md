# Library and Playback Language

This glossary defines the music-library identity and playback terms shared by the Windows and Android flows.

## Library

**Track**: A logical music item the user can browse or play. One track can be reachable through more than one configured source.

**Track ID**: The stable application identity of a track. It remains the reference used by playlists and playback when a source-specific item or locator changes.

**Temporary catalog（暫存曲庫）**: An in-memory SQLite catalog for unindexed tracks in the current Windows File Explorer queue. It supplies the same track, locator, lyric, candidate, and listening-statistics records as the library database, but its lifetime is limited to that queue and application run.

**Source mapping**: The relationship between a track and one item observed in a configured source. It carries source identity and the locator/fingerprint used to synchronize that item.

**Media locator**: A platform-specific address for reading media, such as a filesystem path or a content URI. It belongs to the native side of the application and is not a renderer-facing track identity.

**「演出者」 (artist)**: The user-facing artist value sourced from the embedded `ARTIST` tag. It is distinct from the separate `PERFORMER` tag; `PERFORMER` is not a substitute for `ARTIST`.

**Artist credit**: One track may credit multiple artists; each distinct artist is associated with that track for browsing and per-artist statistics.

**Artist identity**: Artist names are compared after trimming surrounding whitespace and ignoring case and Simplified/Traditional Chinese differences. Names containing Hiragana or Katakana preserve their Han spelling during Simplified/Traditional comparison; the displayed spelling is the most frequent original form, preferring Traditional on ties.

**Artist listening time**: Each artist credited on a track receives the full accumulated listening time of that distinct track; collaborative tracks are not divided among artists.

**Artist track duration**: The sum of the media durations of an artist's distinct visible tracks. A collaborative track contributes its full duration to each credited artist; this differs from Artist listening time.

**Artist track duration**: The sum of the media durations of an artist's distinct visible tracks. A collaborative track contributes its full duration to each credited artist; this differs from Artist listening time.

**Artist catalog metrics**: Artist counts use tracks visible in the enabled library sources. Song count is distinct tracks; album count is distinct non-empty album names under the artist-name comparison rules.

**Year**: The displayed year is derived only from the embedded `YEAR` value. Accepted forms are a four-digit year (`YYYY`) or a valid full date (`YYYY-MM-DD`); other forms do not define a display year.

**Music collation（音樂文字排序）**: Track titles, artists, and album names use one ICU4X collator with the operating system's current locale defaults and numeric ordering enabled. Windows supplies the locale through `GetUserDefaultLocaleName`; Android uses `Locale.getDefault()` and forces Zhuyin collation when the country is Taiwan. Artist and album indexes use the same backend comparator as SQLite track and playlist ordering.

**App settings**: User-owned preferences stored in a versioned JSON document, including theme colors, shuffle, repeat mode, Now Playing layout/appearance, and the source registry. JSON is the authority for these values; SQLite rows are rebuildable music-data projections.

**Main-window background source**: A saved choice of solid color, a UserData-copied image, a recursive slideshow folder, the current track's artwork, or the current Windows desktop wallpaper image. Slideshow timing/order and blur/opacity live in the versioned JSON settings; artwork and wallpaper are display resources rather than SQLite data.

**Source registry**: The stable list of enabled or disabled folders and explicitly registered playlist files. A playlist-file entry associates its source ID and native path with the internal Playlist ID; static playlists do not imply a source-file path.

## Playback

**Playback session**: The restorable state of a listening session: its ordered queue traversal, selected queue entry and track, and saved playback position. Stable Track IDs identify queued tracks; a native file path is not session identity.

**Accumulated listening time**: The credited duration during which audio is in the Playing state and its natural playback position advances. Paused, loading, ready, stopped, and seek jumps do not add listening time.

**Shuffle ratio**: The accumulated listening time divided by a track’s known duration. A lower ratio increases the track’s chance in a newly generated shuffle traversal; an unknown or zero duration has neutral weight.

**Playback snapshot**: A point-in-time view of the current playback session, including state, position, duration, volume, any current error, and on Windows the output's sample-rate path. The audio backend is authoritative for these values.

**Resampling mode**: The Windows playback setting that decides who converts a track whose sample rate differs from the output: the app's high-quality resampler (default, stream stays at the device mix rate) or the Windows audio engine (stream opened at the track's rate). Both use shared mode and are not bit-perfect.

**DSD（Direct Stream Digital）**: A 1-bit, very-high-rate audio representation stored in DSF or DFF files. Its rates are 64, 128, 256 or 512 times 44.1 kHz and are named DSD64, DSD128, DSD256 and DSD512.
_Avoid_: SACD (a disc format), 1-bit PCM

**DSD-to-PCM conversion**: Playing a DSD track by converting it in software to PCM at a fixed rate (176.4 kHz by default) before the normal output path. It works on any output device and is not bit-perfect.

**DoP (DSD over PCM)**: Sending the original DSD bits to a DSD-capable DAC packed inside PCM frames. It needs an exclusive, unprocessed output at an exact rate, so it is separate from the normal output path and is not available yet.

**Playback queue**: A session-local ordered sequence of stable Track IDs drawn from the active library query or a user playlist. Library order follows the visible library sort; playlist order follows entry order and keeps repeated playable entries. Queue IDs stay in Rust and are not copied into the Renderer.

**Shuffle**: A queue traversal order that keeps the current entry in place, visits each remaining entry once per cycle, and supports previous/next traversal through the same order. Weighted shuffle uses `1 / (1 + played_ms / duration_ms)` to raise the relative chance of less-played tracks when a traversal is generated; unknown or zero duration uses neutral weight `1`. Turning shuffle off resumes source order at the current entry.

**Repeat mode**: `off` stops after the final queue entry, `all` wraps the queue at its ends, and `one` repeats the current track only when it reaches its natural end. Manual next still advances in every mode.

## Playlists

**Playlist**: An ordered set of media entries with a stable Playlist ID. A playlist can be a static in-app list or the current projection of a registered playlist-file source.

**Playlist-file source**: A registered M3U/M3U8 file with a stable Source ID and Playlist ID. The external file is authoritative for its projected entries; synchronization reads it and never writes changes back automatically.

**Playlist projection**: The last successfully parsed ordered entries from a playlist-file source. A file-level read or parse failure preserves the previous projection. A missing or unreadable individual track preserves its locator and any prior source mapping while other entries continue syncing.

**Playlist media reference**: A local media locator listed by a playlist source. Repeated references remain repeated at their original positions, while their source mapping is deduplicated by canonical locator identity and reuses the same Track ID as folder or other playlist sources.

**M3U/M3U8**: Text interchange formats for ordered playlist entries, including path and optional display metadata. A playlist file can include entries that are not currently present in a folder source and can therefore contribute playable library tracks itself.

## Operating-system media controls

**System media control**: Windows' operating-system-facing representation of the current playback session. It shows supported metadata and sends transport requests; the application audio backend remains authoritative for playback state.

## Diagnostics

**Diagnostic snapshot（診斷快照）**: A versioned machine-readable summary of the application build, process, data-file health metadata, and last startup component status for a local support tool. It excludes music metadata and user-settings/database contents.

**Panic record（panic 記錄）**: A timestamped record of an unhandled Rust panic with its thread, source location, message, and stack trace. It helps locate the original failure rather than only reporting a later recovery symptom.

## Portable updates

**Build identity（建置識別）**: Embedded UTC time with millisecond precision plus the full git commit. Display dates and package SemVer cannot order same-day builds.

**Pending update（待套用更新）**: Verified application/helper bytes and their fixed target in the separate `UserData/updates` journal, applied before the database opens at the next launch.

**Update permit（更新許可）**: A nonce written after strict persistence succeeds, or before database initialization at next launch. A process exit alone does not grant replacement permission.

**Launch acknowledgement（啟動交接確認）**: Confirmation by the new executable before Tauri/database initialization; it does not certify later UI, audio, or migrations.

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
