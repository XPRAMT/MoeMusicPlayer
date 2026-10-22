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
    fun opacityAndCentre() {
        assertEquals(1f, FloatingLyricsRules.windowAlpha(100, null), 0.0001f)
        assertEquals(0.5f, FloatingLyricsRules.windowAlpha(50, null), 0.0001f)
        assertEquals(0.01f, FloatingLyricsRules.windowAlpha(0, null), 0.0001f)
        assertEquals(0.01f, FloatingLyricsRules.windowAlpha(1, null), 0.0001f)
        assertEquals(0.05f, FloatingLyricsRules.windowAlpha(5, 0.8f), 0.0001f)
        assertEquals(0.8f, FloatingLyricsRules.windowAlpha(100, 0.8f), 0.0001f)
        assertEquals(0.6f, FloatingLyricsRules.windowAlpha(60, 0.8f), 0.0001f)
        assertEquals(100, FloatingLyricsRules.opacityPercent(250))
        assertEquals(400, FloatingLyricsRules.centredStart(1080, 280))
        assertEquals(0, FloatingLyricsRules.centredStart(100, 300))
    }

    @Test
    fun widthAndLineHeight() {
        // Status bar 88 px, text 1.17 em: 88 / (1.17 * 1.1) = 68.4 -> 68.
        assertEquals(68f, FloatingLyricsRules.fontForLineHeight(88, 1.17f), 0.001f)
        assertEquals(1f, FloatingLyricsRules.fontForLineHeight(0, 1.17f), 0.001f)
        assertEquals(140, FloatingLyricsRules.statusBarX(1_080, 800, null, 10))
        // Camera at 500..580: right side (500 px) >= left side (500 px): start right of it.
        assertEquals(280, FloatingLyricsRules.statusBarX(1_080, 800, Pair(500, 580), 10)) // clamped to the screen
        assertEquals(590, FloatingLyricsRules.statusBarX(1_080, 300, Pair(500, 580), 10))
        assertEquals(490, FloatingLyricsRules.statusBarX(1_080, 300, Pair(800, 880), 10))
        assertEquals("portrait", FloatingLyricsRules.orientationKey(false))
        assertEquals("landscape", FloatingLyricsRules.orientationKey(true))
        assertEquals(200, FloatingLyricsRules.widthDp(10))
        assertEquals(4000, FloatingLyricsRules.widthDp(9_999))
        assertEquals(900, FloatingLyricsRules.overlayWidth(900, 1_080))
        assertEquals(1_080, FloatingLyricsRules.overlayWidth(3_000, 1_080))
        // 40 px of text plus 10%.
        assertEquals(44, FloatingLyricsRules.lineHeightPx(-30f, 10f))
        // The status bar area (y = 0) is reachable.
        assertEquals(Pair(0, 0), FloatingLyricsRules.clampPosition(0, 0, 1_000, 120, 1_080, 2_400))
    }

    @Test
    fun shortLinesFollowTheAlignmentAndLongOnesSlide() {
        assertEquals(0f, FloatingLyricsRules.lineStartX(300f, 100f, "left", 0.7f), 0.001f)
        assertEquals(100f, FloatingLyricsRules.lineStartX(300f, 100f, "center", 0.7f), 0.001f)
        assertEquals(200f, FloatingLyricsRules.lineStartX(300f, 100f, "right", 0.7f), 0.001f)
        assertEquals(0f, FloatingLyricsRules.lineStartX(300f, 500f, "right", 0f), 0.001f)
        assertEquals(-100f, FloatingLyricsRules.lineStartX(300f, 500f, "center", 0.5f), 0.001f)
        assertEquals(-200f, FloatingLyricsRules.lineStartX(300f, 500f, "left", 2f), 0.001f)
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