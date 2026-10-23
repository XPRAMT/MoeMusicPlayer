package com.moemusicplayer.mediaindex

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
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
    fun continuousStripMatchesTheWebRule() {
        val (cells, widths) = FloatingLyricsRules.stripLayout(floatArrayOf(100f, 50f, 80f), floatArrayOf(60f, 90f), 20f)
        assertEquals(listOf(0f, 120f, 230f), cells.toList())
        assertEquals(listOf(100f, 90f, 80f), widths.toList())
        assertEquals(165f, FloatingLyricsRules.stripAnchor(cells, widths, 1, "center"), 0.001f)
        val (even, evenWidths) = FloatingLyricsRules.stripLayout(floatArrayOf(100f, 100f, 100f), floatArrayOf(), 20f)
        val times = longArrayOf(1_000, 3_000, 5_000)
        val at = { align: String, ms: Long -> FloatingLyricsRules.stripOffset(times, even, evenWidths, align, ms, 400f) }
        assertEquals(0f, at("left", 1_000), 0.001f)
        assertEquals(-60f, at("left", 2_000), 0.001f)
        assertEquals(-120f, at("left", 3_000), 0.001f)
        assertEquals(30f, at("center", 3_000), 0.001f)
        assertEquals(300f, at("right", 1_000), 0.001f)
        assertEquals(0f, at("left", 0), 0.001f)
        assertEquals(-240f, at("left", 9_000), 0.001f)
        assertEquals("left", FloatingLyricsRules.STRIP_ALIGN)
        assertEquals("center", FloatingLyricsRules.STRIP_TARGET)
        assertEquals(0f, FloatingLyricsRules.stripOffset(longArrayOf(), even, evenWidths, "left", 0, 400f), 0.001f)
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
    fun positionIsCarriedOnBetweenCoarseReports() {
        // Paused: the report as is.
        assertEquals(1_000L, FloatingLyricsRules.smoothPositionMs(900L, 1_000L, 0L, 500L, playing = false))
        // Playing: the last report plus the time since it.
        assertEquals(1_016L, FloatingLyricsRules.smoothPositionMs(null, 1_000L, 100L, 116L, playing = true))
        // A fresh report slightly behind the carried-on value: hold, never step back.
        assertEquals(1_120L, FloatingLyricsRules.smoothPositionMs(1_120L, 1_100L, 200L, 210L, playing = true))
        // A seek back: follow it.
        assertEquals(5_010L, FloatingLyricsRules.smoothPositionMs(1_120L, 5_000L, 200L, 210L, playing = true))
        assertEquals(110L, FloatingLyricsRules.smoothPositionMs(9_000L, 100L, 200L, 210L, playing = true))
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
        assertEquals(100, FloatingLyricsRules.widthDp(10))
        assertEquals(8192, FloatingLyricsRules.widthDp(9_999))
        // Lyric area plus a 60 px lock gutter; the area shrinks so both fit the screen.
        assertEquals(960, FloatingLyricsRules.overlayWidth(900, 1_080, 60))
        assertEquals(1_080, FloatingLyricsRules.overlayWidth(3_000, 1_080, 60))
        // Only the 900 px lyric area is centred: (1080 - 900) / 2.
        assertEquals(90, FloatingLyricsRules.lyricCentredStart(1_080, 960, 60))
        // With 10 px side margins (gutter 60 + 20) the window starts 10 px left of the centred area.
        assertEquals(80, FloatingLyricsRules.lyricCentredStart(1_080, 980, 80, 10))
        assertEquals(8, FloatingLyricsRules.sideMarginPx(32f))
        assertEquals(1, FloatingLyricsRules.sideMarginPx(0f))
        // Area 110 px tall: 20 px above and below make the panel 150 (text height x 1.5).
        assertEquals(20, FloatingLyricsRules.panelRoomPx(110))
        assertEquals(0, FloatingLyricsRules.panelRoomPx(0))
        // 移動到狀態列: the panel's centre on the bar's, whatever the font (the same on every tap).
        assertEquals(-28, FloatingLyricsRules.centredOnBar(64, 120))
        assertEquals(10, FloatingLyricsRules.centredOnBar(64, 44))
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
    fun lineStartsWithItsLeftEdgeOnTheLyricAreaCentre() {
        val (cells, widths) = FloatingLyricsRules.stripLayout(floatArrayOf(100f, 100f, 100f), floatArrayOf(), 20f)
        val times = longArrayOf(1_000, 3_000, 5_000)
        val at = { ms: Long, view: Float -> FloatingLyricsRules.stripOffset(times, cells, widths, FloatingLyricsRules.STRIP_ALIGN, ms, view, FloatingLyricsRules.STRIP_TARGET) }
        assertEquals(150f, at(1_000, 300f), 0.001f)
        assertEquals(30f, at(3_000, 300f), 0.001f)
        assertEquals(90f, at(2_000, 300f), 0.001f)
        assertEquals(-90f, at(5_000, 300f), 0.001f)
        // The panel width only moves the centre.
        assertEquals(130f, at(3_000, 500f), 0.001f)
        assertEquals(150f, cells[1] + at(3_000, 300f), 0.001f)
    }

    @Test
    fun stripWindowDrawsOnlyNearbyCells() {
        val starts = FloatArray(100) { it * 110f }
        val widths = FloatArray(100) { 100f }
        // Cell 50 right-anchored in a 300 px view: shift 300 - 5600.
        val range = FloatingLyricsRules.stripWindow(starts, widths, 300f - 5600f, 300f, 300f)
        assertEquals(45, range.first)
        assertEquals(53, range.last)
        assertTrue(FloatingLyricsRules.stripWindow(FloatArray(0), FloatArray(0), 0f, 300f, 300f).isEmpty())
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

    // Continuous strip fade: lines start at 1 000, 2 000 and 4 000 ms; a line fades over the 200 ms before the next starts.
    private fun weights(lyricsMs: Long) = (0..2).map { FloatingLyricsRules.cellWeight(starts, it, lyricsMs) }

    @Test
    fun inactiveOpacityIsClampedToTheSettingsRange() {
        assertEquals(10, FloatingLyricsRules.inactiveOpacityPercent(0))
        assertEquals(10, FloatingLyricsRules.inactiveOpacityPercent(-5))
        assertEquals(37, FloatingLyricsRules.inactiveOpacityPercent(37))
        assertEquals(100, FloatingLyricsRules.inactiveOpacityPercent(250))
        assertEquals(50, FloatingLyricsRules.DEFAULT_INACTIVE_OPACITY_PERCENT)
    }

    @Test
    fun otherLinesAreMainOpacityTimesInactiveOpacity() {
        // 80% main, 50% inactive: the current line stays 0.8, the others are 0.4.
        assertEquals(0.8f, FloatingLyricsRules.cellOpacity("continuous", 80, 50, 1f), 0.0001f)
        assertEquals(0.4f, FloatingLyricsRules.cellOpacity("continuous", 80, 50, 0f), 0.0001f)
        // Relative to the main opacity, which dims the whole overlay.
        assertEquals(0.5f, FloatingLyricsRules.inactiveFactor("continuous", 50, 0f), 0.0001f)
        assertEquals(1f, FloatingLyricsRules.inactiveFactor("continuous", 50, 1f), 0.0001f)
    }

    @Test
    fun fullInactiveOpacityDoesNotFadeAnything() {
        assertEquals(1f, FloatingLyricsRules.inactiveFactor("continuous", 100, 0f), 0.0001f)
        assertEquals(1f, FloatingLyricsRules.inactiveFactor("continuous", 100, 0.5f), 0.0001f)
        assertEquals(0.7f, FloatingLyricsRules.cellOpacity("continuous", 70, 100, 0f), 0.0001f)
    }

    @Test
    fun noAnimationKeepsEveryLineAtTheMainOpacity() {
        assertEquals(1f, FloatingLyricsRules.inactiveFactor("none", 10, 0f), 0.0001f)
        assertEquals(0.8f, FloatingLyricsRules.cellOpacity("none", 80, 50, 0f), 0.0001f)
    }

    @Test
    fun missingOrOutOfRangeInactiveOpacityFallsBackIntoRange() {
        assertEquals(0.1f, FloatingLyricsRules.inactiveFactor("continuous", 0, 0f), 0.0001f)
        assertEquals(1f, FloatingLyricsRules.inactiveFactor("continuous", 999, 0f), 0.0001f)
    }

    @Test
    fun fadeStartsTwoHundredMsBeforeTheNextLineAndEndsWhenItStarts() {
        // Well inside line 0 (1 000..2 000), before the lead-in: it is current, the others are not.
        assertEquals(listOf(1f, 0f, 0f), weights(1_500))
        // Lead-in start (2 000 - 200): nothing has changed yet.
        assertEquals(listOf(1f, 0f, 0f), weights(1_800))
        // Midpoint: the old line is halfway down and the next halfway up.
        assertEquals(listOf(0.5f, 0.5f, 0f), weights(1_900))
        // The next line's start: the fade is exactly finished.
        assertEquals(listOf(0f, 1f, 0f), weights(2_000))
        // After it, it stays put.
        assertEquals(listOf(0f, 1f, 0f), weights(3_000))
    }

    @Test
    fun fadeQuarterPointsAreLinear() {
        assertEquals(0.75f, FloatingLyricsRules.cellWeight(starts, 0, 1_850), 0.0001f)
        assertEquals(0.25f, FloatingLyricsRules.cellWeight(starts, 1, 1_850), 0.0001f)
        assertEquals(0.25f, FloatingLyricsRules.cellWeight(starts, 0, 1_950), 0.0001f)
        assertEquals(0.75f, FloatingLyricsRules.cellWeight(starts, 1, 1_950), 0.0001f)
    }

    @Test
    fun beforeTheFirstLineItFadesInAndTheLastLineStaysCurrent() {
        // Before the first line's lead-in nothing is current.
        assertEquals(listOf(0f, 0f, 0f), weights(0))
        assertEquals(listOf(0f, 0f, 0f), weights(800))
        // The first line fades in over the 200 ms before it starts, as the strip rests on it.
        assertEquals(listOf(0.5f, 0f, 0f), weights(900))
        assertEquals(listOf(1f, 0f, 0f), weights(1_000))
        // The last line has no next line to hand over to: it stays at 1 to the end.
        assertEquals(listOf(0f, 0f, 1f), weights(4_000))
        assertEquals(listOf(0f, 0f, 1f), weights(900_000))
        assertEquals(0.5f, FloatingLyricsRules.cellWeight(starts, 2, 3_900), 0.0001f)
    }

    @Test
    fun fadeDependsOnlyOnTheTimeSoASeekNeedsNoSpecialCase() {
        // Seeking to the middle of a fade, back out of it, or far ahead gives the same values as playing there.
        assertEquals(listOf(0.5f, 0.5f, 0f), weights(1_900))
        assertEquals(listOf(0f, 0f, 1f), weights(5_000))
        assertEquals(listOf(1f, 0f, 0f), weights(1_100))
        assertEquals(listOf(0f, 0f, 0f), weights(-3_000))
    }

    @Test
    fun fadeEdgeCases() {
        assertEquals(0f, FloatingLyricsRules.cellWeight(starts, -1, 2_000), 0f)
        assertEquals(0f, FloatingLyricsRules.cellWeight(starts, 3, 2_000), 0f)
        assertEquals(0f, FloatingLyricsRules.cellWeight(longArrayOf(), 0, 0), 0f)
        // Lines sharing a start time: only the later one is ever current.
        val same = longArrayOf(1_000, 1_000, 3_000)
        assertEquals(0f, FloatingLyricsRules.cellWeight(same, 0, 1_500), 0f)
        assertEquals(1f, FloatingLyricsRules.cellWeight(same, 1, 1_500), 0f)
        // A line closer than the fade length to the next one still hands over, finishing at the next start.
        val quick = longArrayOf(1_000, 1_100)
        assertEquals(0f, FloatingLyricsRules.cellWeight(quick, 0, 1_100), 0f)
        assertEquals(1f, FloatingLyricsRules.cellWeight(quick, 1, 1_100), 0f)
    }

    @Test
    fun fadeIsOnlyActiveInTheLeadInBeforeALine() {
        assertTrue(!FloatingLyricsRules.fadeActive(starts, 500))
        assertTrue(FloatingLyricsRules.fadeActive(starts, 900))
        assertTrue(!FloatingLyricsRules.fadeActive(starts, 1_500))
        assertTrue(FloatingLyricsRules.fadeActive(starts, 1_900))
        assertTrue(!FloatingLyricsRules.fadeActive(starts, 2_000 + 1))
        assertTrue(!FloatingLyricsRules.fadeActive(starts, 9_000))
        assertEquals(0, FloatingLyricsRules.firstStartAfter(starts, 999))
        assertEquals(1, FloatingLyricsRules.firstStartAfter(starts, 1_000))
        assertEquals(3, FloatingLyricsRules.firstStartAfter(starts, 4_000))
    }
}
