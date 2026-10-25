package com.moemusicplayer.mediaindex

import com.moemusicplayer.mediaindex.PauseHide.Activity
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** 「暫停時隱藏」: the same cases as `player-lyrics-native`'s `pause_hide` tests (Windows). */
class PauseHideTest {
    private fun clock(
        playing: Boolean = false,
        active: Boolean = true,
        wantsPlay: Boolean = playing,
        buffering: Boolean = false,
    ) = ClockSnapshot("a", 1, 0, 0, playing, 1f, active, 200_000L, wantsPlay, buffering)

    @Test
    fun theClockReportsMapToActivities() {
        assertEquals(Activity.IDLE, PauseHide.activity(null))
        assertEquals(Activity.PLAYING, PauseHide.activity(clock(playing = true)))
        assertEquals(Activity.PAUSED, PauseHide.activity(clock(wantsPlay = false)))
        assertEquals(Activity.PAUSED, PauseHide.activity(clock(wantsPlay = false, buffering = true)))
        assertEquals(Activity.TRANSIENT, PauseHide.activity(clock(wantsPlay = true, buffering = true)))
        // Ended / idle between tracks, and ready but held back (a call holding the audio).
        assertEquals(Activity.IDLE, PauseHide.activity(clock(active = false, wantsPlay = true)))
        assertEquals(Activity.IDLE, PauseHide.activity(clock(active = false, wantsPlay = false)))
        assertEquals(Activity.IDLE, PauseHide.activity(clock(wantsPlay = true)))
    }

    @Test
    fun aPauseHidesAtOnceAndPlayShowsAgain() {
        val gate = PauseGate()
        assertFalse("nothing seen yet: hidden", gate.shown)
        assertTrue("the setting off always shows", gate.visible(false))
        assertTrue(gate.observe(Activity.PLAYING, 0))
        assertFalse(gate.observe(Activity.PAUSED, 10))
        assertNull(gate.dueAt())
        assertTrue(gate.observe(Activity.PLAYING, 20))
    }

    @Test
    fun aTrackSwitchNeverHides() {
        val gate = PauseGate()
        gate.observe(Activity.PLAYING, 0)
        // A manual switch: buffering after the load, however long.
        assertTrue(gate.observe(Activity.TRANSIENT, 100))
        assertTrue(gate.tick(60_000))
        assertTrue(gate.observe(Activity.PLAYING, 60_001))
        // A natural end: ended while the next file is checked, buffering, playing.
        assertTrue(gate.observe(Activity.IDLE, 61_000))
        assertEquals(61_000 + PauseHide.IDLE_HIDE_DELAY_MS, gate.dueAt())
        assertTrue(gate.observe(Activity.TRANSIENT, 62_500))
        assertNull(gate.dueAt())
        assertTrue(gate.observe(Activity.PLAYING, 62_900))
    }

    @Test
    fun aLastingIdleStateHidesAfterTheDelay() {
        val gate = PauseGate()
        gate.observe(Activity.PLAYING, 0)
        assertTrue(gate.observe(Activity.IDLE, 1000))
        assertTrue(gate.tick(1000 + PauseHide.IDLE_HIDE_DELAY_MS - 1))
        assertFalse(gate.tick(1000 + PauseHide.IDLE_HIDE_DELAY_MS))
        assertNull(gate.dueAt())
        assertFalse(gate.observe(Activity.IDLE, 9000))
        assertTrue(gate.observe(Activity.PLAYING, 9001))
    }

    @Test
    fun aWindowThatOpensWhileNothingPlaysStaysHidden() {
        for (first in listOf(Activity.PAUSED, Activity.IDLE, Activity.TRANSIENT)) {
            val gate = PauseGate()
            assertFalse(first.name, gate.observe(first, 0))
            assertNull(gate.dueAt())
            assertTrue(gate.observe(Activity.PLAYING, 5))
        }
    }

    @Test
    fun aPauseWhileBufferingIsPublished() {
        LyricsClock.resetForTest()
        try {
            var calls = 0
            LyricsClock.onPlayState = { calls++ }
            LyricsClock.publish("a", 0, false, 1f, true, 1000, 0, wake = true, wantsPlay = true, buffering = true)
            assertEquals(Activity.TRANSIENT, PauseHide.activity(LyricsClock.snapshot))
            // Only playWhenReady changes: still a new report, and the hook runs.
            LyricsClock.publish("a", 0, false, 1f, true, 1000, 0, wake = true, wantsPlay = false, buffering = true)
            assertEquals(Activity.PAUSED, PauseHide.activity(LyricsClock.snapshot))
            LyricsClock.clear()
            assertEquals(3, calls)
            assertEquals(Activity.IDLE, PauseHide.activity(LyricsClock.snapshot))
        } finally {
            LyricsClock.resetForTest()
        }
    }
}
