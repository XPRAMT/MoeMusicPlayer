# Windows Shell Open Playback Queue

- Status: accepted
- Date: 2026-10-12T01:14:48+08:00

## Context

Opening an audio file from File Explorer must play the supplied path and include sibling audio files without adding unregistered media to the persistent music library. The existing playback queue and checkpoint are stored in SQLite and resolve tracks through the library, so shell-open playback needs a separate in-memory projection while retaining existing track identities where paths already match the library.

## Decision

- Register supported audio handlers per user under `HKCU`; never change Windows defaults programmatically. The About page offers registration and launches the `ms-settings:defaultapps` URI through `ShellExecuteW` for the user to choose MoeMusicPlayer. Do not pass the URI as an `explorer.exe` path argument.
- Use Tauri's Windows single-instance plugin to forward later file-open arguments to the running process. The first process reads its own arguments. Resolve relative arguments against the forwarded working directory and accept only file paths with supported audio extensions.
- Canonicalize the selected file, enumerate only regular audio files in its containing directory, exclude child directories, de-duplicate canonical paths, and sort by case-insensitive filename. Set the initial cursor to the selected file so it starts immediately; subsequent order follows the existing shuffle preference.
- Resolve every sibling path against the persistent library. Reuse the existing `TrackId`, metadata, and statistics when found. For missing paths, read tags and codec properties into a generated in-memory track identity; do not insert a source, mapping, or track into SQLite.
- Store unindexed siblings in a separate in-memory SQLite temporary catalog. Route lyric lookup, provider candidate cache, manual lyric selection, and listening statistics to that catalog by transient `TrackId`; library tracks continue to use the persistent database. When Explorer search is active, merge persistent and temporary library search results, use the same sort and filter rules, and de-duplicate matching filesystem paths.
- Read shuffle statistics from each track's owning catalog. On a new Explorer-open queue, flush pending statistics for the previous temporary catalog before releasing it; on process exit the temporary catalog is discarded. Never copy temporary lyric or listening data into the persistent library.
- Keep the complete Explorer-opened queue in process memory, including when every item is already in the library. Suppress queue/session and position checkpoint writes while this queue is active so restart keeps the last durable library/playlist session rather than a partial external queue.
- Read artwork for transient tracks from their original path. The playback queue and system media surface use the same in-memory summaries and paths.

## Consequences

- File associations remain available to the current Windows user and point at the current executable path; moving the portable directory requires registering again.
- Windows retains control of the default-app choice. The user chooses MoeMusicPlayer in Settings after registration.
- Explorer-opened queues and their unindexed track metadata, lyrics, candidates, and listening statistics disappear on exit. Tracks already present in the persistent library keep their normal persistent statistics. Temporary search results are paginated with the active library sort and are de-duplicated by canonical file path.
- Android does not register Windows file associations or accept this shell-open flow.
