# ADR 0005: Listening-time accounting and weighted shuffle

## Status

Accepted

## Context

Shuffle should favor tracks with less listening relative to their duration while remaining random. Listening time must follow actual audio playback, including when the renderer is throttled or minimized, and must not be confused by seek jumps, pauses, queue duplicates, or rapid track changes. The active traversal also needs to remain stable so Previous and restart preserve the order already presented.

## Decisions

- The audio worker measures only Playing segments. For each segment, credited time is the increase in the lesser of monotonic elapsed time and natural audio-position advance, minus time already credited in that segment. State transitions, track changes, seeks, stops, and natural end close the segment before changing identity; a seek jump itself contributes no time. Muting does not stop accounting.
- The actor keeps cumulative per-track counters under opaque runtime identities and exposes dirty checkpoints. It does not write SQLite. A background service collector maps identities to stable Track IDs, persists batches transactionally, and clears only counters acknowledged by SQLite.
- SQLite uses a fresh runtime UUID plus owner PID and process-start identity. Checkpoints are idempotent watermarks. Normal shutdown pauses audio, flushes the final batch, then finishes the runtime. A missing process or reused PID can be cleaned up; unknown process identity or access errors preserve the runtime.
- The collector checkpoints at most every five seconds while healthy and wakes at playback boundaries. On database errors it retains dirty counters and retries; the five-second crash-loss bound applies only while persistence is healthy. Persistence errors are exposed through the existing playback snapshot error path without replacing an audio or restore error.
- For a track with positive known duration, shuffle weight is `1 / (1 + played_ms / duration_ms)`. Unknown or zero duration uses neutral weight `1`. A new shuffle traversal assigns each remaining slot `score = weight + U` with `U ~ Uniform(0, 1)`, sorts slots by descending score, and pins the selected entry first. Duplicate playlist entries remain separate traversal slots but use their Track ID's shared statistics.
- Statistics are read when a new queue traversal is generated or shuffle is turned on. The current traversal is not reordered during playback or at Repeat-All boundaries; Previous, restart, and the saved traversal therefore remain exact.
- Preferences (including shuffle/repeat mode) remain authoritative in versioned JSON. Queue/session and listening totals remain in App SQLite. No filesystem path is stored as a statistics identity.

## Alternatives considered

- Renderer polling was rejected because background timer throttling can miss playback and because it would make the UI clock authoritative.
- Lossy audio events were rejected as the sole counter because a full event queue can drop updates. The actor instead maintains cumulative dirty counters until database acknowledgement.
- Writing SQLite from the audio actor or once per position poll was rejected because blocking storage work would affect playback and create excessive writes.
- Reweighting each next track or every Repeat-All boundary was rejected because it changes an already visible traversal and invalidates exact Previous/restart behavior.
- Strictly sorting by ratio alone was rejected because shuffle must remain randomized; adding a uniform `U` keeps randomness while still favoring higher weights.

## Consequences

- Tracks with a lower played-to-duration ratio have a higher chance, not a guarantee, of appearing earlier in a newly generated traversal. Unknown-duration tracks remain eligible with neutral weight.
- Normal healthy operation persists within five seconds plus the final boundary flush. If SQLite remains unavailable, in-memory dirty increments are retained and the error is visible, but abrupt process loss can exceed that bound.
- The isolated service integration fixture uses a synthetic clocked audio backend; it verifies persistence and queue weighting but is not a Rodio, WASAPI, or real-device listening test. Real Tauri GUI and audio acceptance remain separate.
