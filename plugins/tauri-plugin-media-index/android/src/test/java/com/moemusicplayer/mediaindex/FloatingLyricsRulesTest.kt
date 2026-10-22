package com.moemusicplayer.mediaindex

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class FloatingLyricsRulesTest {
    private val starts = longArrayOf(1_000, 2_000, 4_000)

    @Test
    fun activeLineMatchesTheWebRule() {
        assertEquals(-1, FloatingLyricsRules.activeIndex(starts, 500, 0))
        assertEquals(0, FloatingLyricsRules.activeIndex(starts, 1_000, 0))
        assertEquals(1, FloatingLyricsRules.activeIndex(starts, 1_700, 300))
        assertEquals(2, FloatingLyricsRules.activeIndex(starts, 9_000, 0))
        assertEquals(-1, FloatingLyricsRules.activeIndex(longArrayOf(), 9_000, 0))
    }

    @Test
    fun lineDurationAndScroll() {
        assertEquals(1_000L, FloatingLyricsRules.lineDurationMs(starts, 0, null, 0))
        assertEquals(6_000L, FloatingLyricsRules.lineDurationMs(starts, 2, 10_500, 500))
        assertEquals(5_000L, FloatingLyricsRules.lineDurationMs(starts, 2, null, 0))
        assertEquals(1_000L, FloatingLyricsRules.lineDurationMs(longArrayOf(0, 100), 0, null, 0))
        assertEquals(0.5f, FloatingLyricsRules.scrollProgress(450, 1_000), 0.0001f)
        assertEquals(1f, FloatingLyricsRules.scrollProgress(950, 1_000), 0.0001f)
        assertEquals(0f, FloatingLyricsRules.scrollProgress(-5, 1_000), 0.0001f)
    }

    @Test
    fun colorsAndPosition() {
        assertEquals(0x000000, FloatingLyricsRules.effectRgb(0xffffff))
        assertEquals(0xffffff, FloatingLyricsRules.effectRgb(0x00e1ff and 0x0000ff))
        assertEquals(0x00e1ff, FloatingLyricsRules.parseHexColor("#00e1ff"))
        assertNull(FloatingLyricsRules.parseHexColor("red"))
        assertEquals(Pair(80, 0), FloatingLyricsRules.clampPosition(500, -20, 1_000, 200, 1_080, 2_400))
        assertEquals(Pair(0, 10), FloatingLyricsRules.clampPosition(10, 10, 2_000, 200, 1_080, 2_400))
    }
}