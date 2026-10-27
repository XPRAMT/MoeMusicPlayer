package com.moemusicplayer.mediaindex

/**
 * The format of the AudioTrack that plays now, kept across track changes (ADR 0044). Media3 reports
 * `onAudioTrackReleased` after the old track is released on its own thread, which can be after the new track has
 * already reported `onAudioTrackInitialized` (for example when the player is pinned to another device and reopens its
 * track while it plays). Clearing on every release would then forget the live track, and the settings page would show
 * the playing device as idle. So releases are counted against initialisations: the format is forgotten only when no
 * track is left open.
 */
internal class LiveTrackOutput<T : Any> {
    private var open = 0

    /** The newest initialised track's format while at least one track is open; null when none is. */
    var current: T? = null
        private set

    fun initialized(format: T) {
        open += 1
        current = format
    }

    /** True when this release closed the last open track (the format is gone). */
    fun released(): Boolean {
        open = (open - 1).coerceAtLeast(0)
        if (open > 0) return false
        current = null
        return true
    }
}
