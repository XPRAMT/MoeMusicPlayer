package com.moemusicplayer.mediaindex

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

class LyricsClockTest {
    private val ms = 1_000_000L

    @Before
    fun reset() = LyricsClock.resetForTest()

    @After
    fun cleanup() = LyricsClock.resetForTest()

    private fun publish(
        position: Long, nowMs: Long, playing: Boolean = true, wake: Boolean = false, track: String = "a",
        speed: Float = 1f, back: Long = 0L,
    ) = LyricsClock.publish(track, position, playing, speed, true, 200_000L, nowMs * ms, wake, back)

    @Test
    fun aPlainRepublishOfTheSameStepKeepsTheAnchor() {
        publish(1000, 100)
        val first = LyricsClock.snapshot!!
        publish(1000, 140)
        assertSame(first, LyricsClock.snapshot)
        publish(1100, 180)
        val second = LyricsClock.snapshot!!
        assertEquals(1100L, second.positionMs)
        assertEquals(180 * ms, second.anchorNanos)
        assertTrue(second.generation > first.generation)
    }

    @Test
    fun aPlayingPositionThatDoesNotStepIsReanchoredAfterHalfASecond() {
        publish(1000, 100)
        val first = LyricsClock.snapshot!!
        publish(1000, 700)
        val second = LyricsClock.snapshot!!
        assertTrue(second.generation > first.generation)
        assertEquals(700 * ms, second.anchorNanos)
    }

    @Test
    fun aPausedPositionKeepsItsAnchorForever() {
        publish(1000, 100, playing = false)
        val first = LyricsClock.snapshot!!
        publish(1000, 5000, playing = false)
        assertSame(first, LyricsClock.snapshot)
    }

    @Test
    fun theAnchorCanBeMovedBackByHalfAPollInterval() {
        publish(2000, 1000, back = 20 * ms)
        assertEquals(980 * ms, LyricsClock.snapshot!!.anchorNanos)
    }

    @Test
    fun aWakePublicationAlwaysReplacesAndWakesTheReader() {
        var wakes = 0
        LyricsClock.onWake = { wakes++ }
        publish(1000, 100)
        assertEquals(0, wakes)
        publish(1000, 110, wake = true)
        assertEquals(1, wakes)
        assertEquals(110 * ms, LyricsClock.snapshot!!.anchorNanos)
        LyricsClock.clear()
        assertNull(LyricsClock.snapshot)
        assertEquals(2, wakes)
    }

    @Test
    fun generationsAlwaysIncrease() {
        var last = 0L
        repeat(10) { i ->
            publish(i * 100L, i * 100L, wake = true)
            val g = LyricsClock.snapshot!!.generation
            assertTrue(g > last)
            last = g
        }
    }

    private fun snapshot(position: Long, anchorMs: Double, playing: Boolean = true, speed: Float = 1f, track: String = "a", gen: Long = 1) =
        ClockSnapshot(track, gen, position, (anchorMs * ms).toLong(), playing, speed, true, 200_000L)

    @Test
    fun theModelExtrapolatesFromTheAnchorAtTheFrameTime() {
        val model = ClockModel()
        val s = snapshot(1000, 100.0)
        assertEquals(1000.0, model.positionAt(s, (100.0 * ms).toLong()), 1e-6)
        assertEquals(1016.0, model.positionAt(s, (116.0 * ms).toLong()), 1e-6)
    }

    @Test
    fun aFrameTimeBeforeTheAnchorIsTheAnchor() {
        val model = ClockModel()
        assertEquals(1000.0, model.positionAt(snapshot(1000, 100.0), (90.0 * ms).toLong()), 1e-6)
    }

    @Test
    fun thePlaybackSpeedScalesTheExtrapolation() {
        val model = ClockModel()
        val s = snapshot(1000, 100.0, speed = 2f)
        model.positionAt(s, (100.0 * ms).toLong())
        assertEquals(1100.0, model.positionAt(s, (150.0 * ms).toLong()), 1e-6)
    }

    @Test
    fun aPausedClockIsExactAndNeverCarriedOn() {
        val model = ClockModel()
        val s = snapshot(1500, 100.0, playing = false)
        assertEquals(1500.0, model.positionAt(s, (100.0 * ms).toLong()), 0.0)
        assertEquals(1500.0, model.positionAt(s, (900.0 * ms).toLong()), 0.0)
    }

    @Test
    fun aSmallDriftBetweenReportsIsSlewedNotStepped() {
        val model = ClockModel()
        var t = 0.0
        model.positionAt(snapshot(0, 0.0, gen = 1), 0)
        var last = 0.0
        // 60 frames of 1/60 s, then a report that is 40 ms ahead of the carried position.
        repeat(60) { i -> t = (i + 1) * 16.6667; last = model.positionAt(snapshot(0, 0.0, gen = 1), (t * ms).toLong()) }
        val ahead = snapshot((t + 40).toLong(), t, gen = 2)
        val next = model.positionAt(ahead, ((t + 16.6667) * ms).toLong())
        assertTrue("never backwards", next >= last)
        assertTrue("not a step of 40 ms", next - last < 16.6667 * 1.2)
    }

    @Test
    fun aBigJumpIsTakenAtOnce() {
        val model = ClockModel()
        model.positionAt(snapshot(1000, 0.0, gen = 1), 0)
        val seek = snapshot(60_000, 100.0, gen = 2)
        assertEquals(60_000.0, model.positionAt(seek, (100.0 * ms).toLong()), 1e-6)
    }

    @Test
    fun aNewTrackStartsFromItsOwnReport() {
        val model = ClockModel()
        model.positionAt(snapshot(1000, 0.0, track = "a"), (1.0 * ms).toLong())
        assertEquals(10.0, model.positionAt(snapshot(10, 5.0, track = "b", gen = 2), (5.0 * ms).toLong()), 1e-6)
    }

    @Test
    fun noSnapshotIsNoClock() {
        val model = ClockModel()
        assertTrue(model.positionAt(null, 0).isNaN())
        assertNotNull(snapshot(0, 0.0))
    }
}
