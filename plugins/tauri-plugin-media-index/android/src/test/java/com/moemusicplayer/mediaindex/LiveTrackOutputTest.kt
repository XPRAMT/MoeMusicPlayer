package com.moemusicplayer.mediaindex

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LiveTrackOutputTest {
    @Test
    fun aTrackThatIsReleasedLeavesNothingPlaying() {
        val live = LiveTrackOutput<String>()
        live.initialized("96 kHz")
        assertEquals("96 kHz", live.current)
        assertTrue(live.released())
        assertNull(live.current)
    }

    @Test
    fun aLateReleaseOfTheOldTrackKeepsTheNewOne() {
        // Pinned to a Bluetooth headset while playing: the new track opens before the old one's release is reported.
        val live = LiveTrackOutput<String>()
        live.initialized("96 kHz speaker")
        live.initialized("96 kHz headset")
        assertFalse(live.released())
        assertEquals("96 kHz headset", live.current)
        assertTrue(live.released())
        assertNull(live.current)
    }

    @Test
    fun anUnpairedReleaseNeverGoesBelowZero() {
        val live = LiveTrackOutput<String>()
        assertTrue(live.released())
        live.initialized("48 kHz")
        assertEquals("48 kHz", live.current)
        assertTrue(live.released())
    }
}
