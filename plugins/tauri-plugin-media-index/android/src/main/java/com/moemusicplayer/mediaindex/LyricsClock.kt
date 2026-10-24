package com.moemusicplayer.mediaindex

import java.util.concurrent.atomic.AtomicInteger

/**
 * One report of the playback clock for the render thread (ADR 0032, GL overlay): immutable, published by the playback
 * service on the player's thread into [LyricsClock.snapshot] and read by the render thread once per frame.
 *
 * `anchorNanos` is on the `System.nanoTime()` base, which is also the base of `Choreographer`'s frame time (both are
 * CLOCK_MONOTONIC on Android), so the render thread needs no conversion. (`SystemClock.elapsedRealtimeNanos()` is
 * CLOCK_BOOTTIME and would differ after any suspend, so it is not used.) `positionMs` was the player's position at
 * `anchorNanos`; while `playing` the position at a later time `t` is `positionMs + (t - anchor) * speed`.
 */
internal class ClockSnapshot(
    val trackId: String,
    /** Increases with every published snapshot, so the reader can tell a new report from the one it already used. */
    val generation: Long,
    val positionMs: Long,
    val anchorNanos: Long,
    val playing: Boolean,
    val speed: Float,
    /** Ready or buffering with a media item: lyrics are due. */
    val active: Boolean,
    /** [FloatingLyricsRules.NO_TIME] while the track length is unknown. */
    val durationMs: Long,
)

/**
 * The published clock. [publish] is called on the player's thread (the main thread); [snapshot] is read from any
 * thread. A "wake" publication (state change, seek, new track) also tells the render thread, a plain re-anchor does not
 * (the render thread reads the newest value at its next frame anyway).
 */
internal object LyricsClock {
    @Volatile
    var snapshot: ClockSnapshot? = null
        private set

    /** Called on the publishing thread after a wake publication; set by the overlay. */
    @Volatile
    var onWake: (() -> Unit)? = null

    /** How many readers want the clock kept fresh by polling (the GL overlay while it exists). */
    val demand = AtomicInteger()

    private var generation = 0L
    private var lastPositionMs = Long.MIN_VALUE
    private var lastTrackId: String? = null
    private var lastPlaying = false
    private var lastSpeed = 1f
    private var lastActive = false
    private var lastDuration = FloatingLyricsRules.NO_TIME
    private var lastAnchorNanos = 0L

    /**
     * Publish a report. With `wake` false the snapshot is only replaced when the position (or anything else) differs
     * from the last one, or the last anchor is older than [REANCHOR_NANOS]; the poll uses that so the anchor sits where
     * the player's coarse position last stepped (the smoothing in [ClockModel] depends on that).
     */
    @Synchronized
    fun publish(
        trackId: String,
        positionMs: Long,
        playing: Boolean,
        speed: Float,
        active: Boolean,
        durationMs: Long,
        nowNanos: Long,
        wake: Boolean,
        anchorBackNanos: Long = 0L,
    ) {
        val unchanged = trackId == lastTrackId && positionMs == lastPositionMs && playing == lastPlaying &&
            speed == lastSpeed && active == lastActive && durationMs == lastDuration
        // Nothing new: while paused the old anchor is exact; while playing it is kept until it is [REANCHOR_NANOS] old.
        if (!wake && unchanged && (!playing || nowNanos - lastAnchorNanos < REANCHOR_NANOS)) return
        lastTrackId = trackId
        lastPositionMs = positionMs
        lastPlaying = playing
        lastSpeed = speed
        lastActive = active
        lastDuration = durationMs
        // A changed coarse position was reached somewhere between the previous poll and now: anchor in the middle.
        val anchor = nowNanos - anchorBackNanos
        lastAnchorNanos = nowNanos
        snapshot = ClockSnapshot(trackId, ++generation, positionMs, anchor, playing, speed, active, durationMs)
        if (wake) onWake?.invoke()
    }

    /** No player (service stopped): readers see no clock. */
    @Synchronized
    fun clear() {
        snapshot = null
        lastTrackId = null
        lastPositionMs = Long.MIN_VALUE
        onWake?.invoke()
    }

    /** Test hook: forget everything. */
    @Synchronized
    fun resetForTest() {
        snapshot = null
        onWake = null
        generation = 0L
        lastTrackId = null
        lastPositionMs = Long.MIN_VALUE
        lastPlaying = false
        lastSpeed = 1f
        lastActive = false
        lastDuration = FloatingLyricsRules.NO_TIME
        lastAnchorNanos = 0L
        demand.set(0)
    }

    /** A playing position that has not stepped for this long is re-anchored by the next poll. */
    const val REANCHOR_NANOS = 500_000_000L
}

/**
 * The render thread's view of the clock: extrapolates the newest [ClockSnapshot] to the frame time and smooths the
 * result with [FloatingLyricsRules.smoothPosition] (a small drift between two reports is slewed, never stepped back; a
 * jump of 250 ms or more, a new track, a seek is taken as is). One instance per render thread; not thread safe.
 */
internal class ClockModel {
    private var previousMs = 0.0
    private var hasPrevious = false
    private var previousAtMs = 0.0
    private var lastTrack: String? = null
    private var lastGeneration = -1L

    /** True after [positionAt] saw a snapshot of another generation than the one before (used by tests / idle logic). */
    var sawNewReport = false
        private set

    /**
     * The position (fractional ms) at `nowNanos` (the frame time) for `snapshot`; NaN when there is no clock.
     * `nowNanos` earlier than the anchor (the report was made after the frame's vsync) counts as the anchor itself.
     */
    fun positionAt(snapshot: ClockSnapshot?, nowNanos: Long): Double {
        if (snapshot == null) {
            hasPrevious = false
            lastTrack = null
            lastGeneration = -1L
            return Double.NaN
        }
        sawNewReport = snapshot.generation != lastGeneration
        lastGeneration = snapshot.generation
        if (snapshot.trackId != lastTrack) {
            lastTrack = snapshot.trackId
            hasPrevious = false
        }
        val now = nowNanos / 1_000_000.0
        val smoothed = FloatingLyricsRules.smoothPosition(
            previousMs, hasPrevious, previousAtMs, snapshot.positionMs, snapshot.anchorNanos / 1_000_000.0, now,
            snapshot.playing, speed = snapshot.speed.toDouble(),
        )
        previousMs = smoothed
        hasPrevious = true
        previousAtMs = now
        return smoothed
    }

    fun reset() {
        hasPrevious = false
        lastTrack = null
        lastGeneration = -1L
    }
}
