package com.moemusicplayer.mediaindex

/**
 * 「暫停時隱藏」 for the Android floating lyrics (ADR 0032): the same rules as `player-lyrics-native`'s `pause_hide`
 * (Windows). Only a real pause or stop hides a shown window at once. The states the player passes through while one
 * track makes way for the next (buffering after the load, ended while the next file is checked) never hide it at once:
 * buffering holds the current visibility, and the other not-playing states hide only once they lasted
 * [IDLE_HIDE_DELAY_MS] (the end of the queue, a restored session that does not play, a call that holds the audio).
 */
internal object PauseHide {
    const val IDLE_HIDE_DELAY_MS = 3_000L
    /** The hide / show fade (the project's animation length). */
    const val FADE_MS = 200L

    enum class Activity {
        /** Playing: show. */
        PLAYING,
        /** Buffering (or seeking) while the user wants it to play: keep what is shown. */
        TRANSIENT,
        /** Paused or stopped by the user: hide. */
        PAUSED,
        /** Any other not-playing state (ended, idle, playback held back, no player): hide after [IDLE_HIDE_DELAY_MS]. */
        IDLE,
    }

    /** From the playback service's clock report (null: no player). */
    fun activity(snapshot: ClockSnapshot?): Activity = when {
        snapshot == null -> Activity.IDLE
        snapshot.playing -> Activity.PLAYING
        // Ended or idle (no media prepared): between tracks, or done.
        !snapshot.active -> Activity.IDLE
        !snapshot.wantsPlay -> Activity.PAUSED
        snapshot.buffering -> Activity.TRANSIENT
        // Ready, wanted, yet not playing (audio focus held by a call, for example).
        else -> Activity.IDLE
    }
}

/** The visibility the floating window would have with 「暫停時隱藏」 on (`pause_hide::PauseGate`). Not thread safe. */
internal class PauseGate {
    /** Nothing observed yet: hidden (a window that opens while nothing plays does not flash). */
    var shown = false
        private set
    var activity: PauseHide.Activity? = null
        private set
    private var idleSince = NONE

    /** Records the activity observed at `nowMs`; returns whether the window shows. */
    fun observe(activity: PauseHide.Activity, nowMs: Long): Boolean {
        when (activity) {
            PauseHide.Activity.PLAYING -> {
                shown = true
                idleSince = NONE
            }
            PauseHide.Activity.TRANSIENT -> idleSince = NONE
            PauseHide.Activity.PAUSED -> {
                shown = false
                idleSince = NONE
            }
            PauseHide.Activity.IDLE -> if (shown && idleSince == NONE) idleSince = nowMs
        }
        this.activity = activity
        return tick(nowMs)
    }

    /** Re-evaluates at `nowMs` with the last activity (call at [dueAt]); returns whether the window shows. */
    fun tick(nowMs: Long): Boolean {
        if (idleSince != NONE && nowMs - idleSince >= PauseHide.IDLE_HIDE_DELAY_MS) {
            shown = false
            idleSince = NONE
        }
        return shown
    }

    /** When a pending idle hide is due (ms), or null. */
    fun dueAt(): Long? = if (idleSince == NONE) null else idleSince + PauseHide.IDLE_HIDE_DELAY_MS

    /** Whether the window shows given its setting. */
    fun visible(hideWhenPaused: Boolean): Boolean = !hideWhenPaused || shown

    private companion object {
        const val NONE = Long.MIN_VALUE
    }
}
