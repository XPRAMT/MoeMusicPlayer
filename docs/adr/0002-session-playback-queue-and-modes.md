# ADR 0002: Session playback queue and modes

## Status

Accepted for the Windows local playback implementation; persisted-session and weighted-shuffle details are extended by [ADR 0005](0005-listening-time-and-weighted-shuffle.md).

## Context

Next/previous, shuffle, and repeat need a stable traversal order while the library and playlist views remain paged. Copying queue candidates into the Renderer would make queue setup grow with the library and violate the UI boundary. Re-querying an offset for each command could change the traversal when library ordering or playlist entries change during playback.

## Decisions

- The Renderer sends a narrow queue source, the selected Track ID, and the current library query or playlist entry position. It never sends all candidate IDs.
- Rust builds a session-local sequence of stable Track IDs from SQLite. Library queues use the same normalized search predicate and `sort_title, track_id` order as paged browsing. Playlist queues use entry order, keep duplicate playable Track IDs, and skip unresolved or disabled entries.
- The queue sequence is an in-memory `Vec<TrackId>` in the Rust playback service. For the 100,000-track target this is a compact identity-only snapshot; current metadata and paths are resolved on demand. Queue edits or library changes do not reorder an active session.
- Shuffle keeps the selected entry first and retains each generated traversal so previous and next move through the same history. Disabling shuffle returns to source order at the current entry. New traversal weighting is defined by ADR 0005.
- Repeat One repeats only after natural end; manual Next advances. Repeat All wraps queue navigation and natural playback. Repeat Off stops at the final entry and does not wrap previous from the first entry.
- Audio playback state remains authoritative in the audio worker. The shell observes `Ended` and advances the queue once, then resolves and loads the next Track ID through the existing database path boundary.

## Alternatives considered

- Sending every candidate ID from the Renderer was rejected because the library may contain 100,000 tracks and the Renderer must not own the full library.
- Storing full `TrackSummary` records in the queue was rejected because queue traversal only needs stable IDs and full metadata can be queried for the active track.
- Querying SQLite by a moving offset for each Next was rejected because edits or sync changes could shift the sequence during a playback session.
- Using a shuffled bag without retaining its order was rejected because Previous could not reliably return to the track just heard.

## Consequences

Queue construction and navigation are testable without a GUI. The Renderer carries only the active source and selected item. The session queue consumes memory proportional to the number of IDs; at 100,000 entries this remains far smaller than full summaries. App SQLite now persists the exact traversal, cursor, and playback position across restart. Weighted shuffle and its listening-time accounting contract are defined in ADR 0005.
