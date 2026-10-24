package com.moemusicplayer.mediaindex

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
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
        assertEquals(-45f, at("left", 2_000), 0.001f) // C1 glide: starts from rest
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
        // The window is opaque; only a locked overlay is capped by the system limit.
        assertEquals(1f, FloatingLyricsRules.windowAlpha(null), 0.0001f)
        assertEquals(0.8f, FloatingLyricsRules.windowAlpha(0.8f), 0.0001f)
        assertEquals(1f, FloatingLyricsRules.windowAlpha(1.5f), 0.0001f)
        assertEquals(100, FloatingLyricsRules.mainOpacityPercent(250))
        assertEquals(1, FloatingLyricsRules.mainOpacityPercent(0))
        assertEquals(1, FloatingLyricsRules.secondaryOpacityPercent(0))
        assertEquals(37, FloatingLyricsRules.secondaryOpacityPercent(37))
        assertEquals(100, FloatingLyricsRules.secondaryOpacityPercent(250))
        assertEquals(100, FloatingLyricsRules.DEFAULT_MAIN_OPACITY_PERCENT)
        assertEquals(50, FloatingLyricsRules.DEFAULT_SECONDARY_OPACITY_PERCENT)
        // Animation none draws the line at main; continuous draws at 1 (the mask dims).
        assertEquals(0.6f, FloatingLyricsRules.lineViewAlpha("none", 60), 0.0001f)
        assertEquals(1f, FloatingLyricsRules.lineViewAlpha("continuous", 60), 0.0001f)
        assertEquals(0.01f, FloatingLyricsRules.lineViewAlpha("none", 0), 0.0001f)
        assertEquals(400, FloatingLyricsRules.centredStart(1080, 280))
        assertEquals(0, FloatingLyricsRules.centredStart(100, 300))
    }

    @Test
    fun positionIsCarriedOnBetweenCoarseReports() {
        val r = FloatingLyricsRules
        // Paused: the report as is.
        assertEquals(1_000.0, r.smoothPositionMs(900.0, 400L, 1_000L, 0L, 500L, playing = false), 1e-9)
        // Playing, first frame: the last report plus the time since it.
        assertEquals(1_016.0, r.smoothPositionMs(null, 0L, 1_000L, 100L, 116L, playing = true), 1e-9)
        // A seek either way: follow it.
        assertEquals(5_010.0, r.smoothPositionMs(1_120.0, 200L, 5_000L, 200L, 210L, playing = true), 1e-9)
        assertEquals(110.0, r.smoothPositionMs(9_000.0, 200L, 100L, 200L, 210L, playing = true), 1e-9)
    }

    @Test
    fun coarseJitteryReportsNeverStepOrReverseThePosition() {
        // Reports refreshed every 100-200 ms, each up to 60 ms off, frames every 16 ms: the shown position always
        // moves forward by less than two frames' worth and stays within the jitter of the truth.
        var seed = 11L
        fun rand(): Double {
            seed = seed * 6364136223846793005L + 1442695040888963407L
            return ((seed ushr 33).toDouble()) / (1L shl 31).toDouble()
        }
        var reported = 0L
        var reportedAt = 0L
        var nextReport = 0L
        var shown: Double? = null
        var shownAt = 0L
        var now = 0L
        while (now < 60_000L) {
            if (now >= nextReport) {
                reported = (now - rand() * 60.0).toLong().coerceAtLeast(0L)
                reportedAt = now
                nextReport = now + 100 + (rand() * 100).toLong()
            }
            val next = FloatingLyricsRules.smoothPositionMs(shown, shownAt, reported, reportedAt, now, playing = true)
            shown?.let { prev ->
                assertTrue("backwards at $now: $prev -> $next", next >= prev)
                assertTrue("step at $now: $prev -> $next", next - prev < 32.0)
            }
            if (now > 2_000L) assertTrue("lost the clock at $now: $next", kotlin.math.abs(next - now) < 120.0)
            shown = next
            shownAt = now
            now += 16
        }
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
        assertEquals(105f, at(2_000, 300f), 0.001f)
        assertEquals(-90f, at(5_000, 300f), 0.001f)
        // The panel width only moves the centre.
        assertEquals(130f, at(3_000, 500f), 0.001f)
        assertEquals(150f, cells[1] + at(3_000, 300f), 0.001f)
    }

    @Test
    fun lastContinuousLineGlidesUntilItsFinalGlyphIsCentredAtTrackEnd() {
        val (cells, widths) = FloatingLyricsRules.stripLayout(floatArrayOf(120f, 80f), floatArrayOf(160f, 0f), 20f)
        val times = longArrayOf(1_000, 5_000)
        val mainWidths = floatArrayOf(120f, 80f)
        val lastGlyphCenters = floatArrayOf(0f, 70f)
        val at = { ms: Long -> FloatingLyricsRules.stripOffset(times, cells, widths, "left", ms, 300f, "center", 11_000, mainWidths, lastGlyphCenters) }

        assertEquals(150f - 180f, at(5_000), 0.001f)
        // C1 curve (same value as lyrics-strip.js): ahead of the old linear 35 px, still short of the end.
        assertEquals(-79.4643f, at(8_000), 0.001f)
        // Monotone, never backwards, with no speed jump at the line start (5000 ms).
        var previous = at(0)
        for (ms in 0L..12_000L step 10) {
            val value = at(ms)
            assertEquals("backwards at $ms", true, value <= previous + 1e-3f)
            previous = value
        }
        val before = at(4_990) - at(4_980)
        val after = at(5_010) - at(5_000)
        assertEquals(before, after, 0.05f)
        // Final glyph occupies [140, 160], placing its centre at x=150.
        assertEquals(150f - 180f - 70f, at(11_000), 0.001f)
        assertEquals("shadow", FloatingLyricsRules.effectiveTextEffect("continuous", "shadow"))
        assertEquals("stroke", FloatingLyricsRules.effectiveTextEffect("continuous", "stroke"))
        assertEquals("none", FloatingLyricsRules.effectiveTextEffect("continuous", "none"))
        assertEquals("shadow", FloatingLyricsRules.effectiveTextEffect("none", "shadow"))
    }

    @Test
    fun theShadowMatchesWindowsSigmaAndItsPadCoversTheBlur() {
        // Skia: sigma = 0.57735 r + 0.5; Windows: sigma = 0.3 em.
        val radius = FloatingLyricsRules.shadowLayerRadius(40f)
        assertEquals(12f, 0.57735f * radius + 0.5f, 0.001f)
        assertEquals(36, FloatingLyricsRules.shadowPadPx(40f))
        assertEquals(0xBF, FloatingLyricsRules.SHADOW_ALPHA)
    }

    @Test
    fun lineRasterCacheKeysOnLookAndKeepsOnlyTheWindow() {
        var created = 0
        val released = mutableListOf<String>()
        val cache = LineRasterCache<String>({ text, _ -> created++; text }, { released += it })
        val look = LineLook(40f, -1, 10f, 0, 0f, 0, 2f)
        assertEquals(null, cache.get("a"))
        cache.setLook(look)
        cache.get("a"); cache.get("a"); cache.get("b")
        assertEquals(2, created)
        cache.setLook(look.copy())
        assertEquals(2, cache.size)
        for (changed in listOf(look.copy(sizePx = 41f), look.copy(color = 0), look.copy(shadowRadius = 0f),
            look.copy(shadowColor = 1), look.copy(strokeWidth = 1f), look.copy(strokeColor = 1), look.copy(density = 3f), look.copy(weight = 700))) {
            cache.get("a")
            cache.setLook(changed)
            assertEquals(0, cache.size)
        }
        cache.get("a"); cache.get("b"); cache.get("c")
        released.clear()
        cache.retain(setOf("b"))
        assertEquals(listOf("a", "c"), released.sorted())
        assertEquals(1, cache.size)
    }

    @Test
    fun fontWeightRangesAndTheTranslationOffsetMatchTheSettings() {
        assertEquals(550, FloatingLyricsRules.DEFAULT_FONT_WEIGHT)
        assertEquals(100, FloatingLyricsRules.fontWeight(0))
        assertEquals(1000, FloatingLyricsRules.fontWeight(5_000))
        assertEquals(300, FloatingLyricsRules.fontWeight(300))
        assertEquals(500, FloatingLyricsRules.auxiliaryFontWeight(550))
        assertEquals(50, FloatingLyricsRules.auxiliaryFontWeight(100))
        assertEquals(950, FloatingLyricsRules.auxiliaryFontWeight(1000))
    }

    @Test
    fun weightToTypefaceUsesTheExactWeightFromApi28AndBoldFrom600Before() {
        assertEquals(FloatingLyricsRules.TypefaceChoice(550, false), FloatingLyricsRules.typefaceChoice(550, 28))
        assertEquals(FloatingLyricsRules.TypefaceChoice(700, true), FloatingLyricsRules.typefaceChoice(700, 34))
        assertEquals(FloatingLyricsRules.TypefaceChoice(1000, true), FloatingLyricsRules.typefaceChoice(5_000, 34))
        assertEquals(FloatingLyricsRules.TypefaceChoice(null, false), FloatingLyricsRules.typefaceChoice(550, 27))
        assertEquals(FloatingLyricsRules.TypefaceChoice(null, false), FloatingLyricsRules.typefaceChoice(599, 24))
        assertEquals(FloatingLyricsRules.TypefaceChoice(null, true), FloatingLyricsRules.typefaceChoice(600, 24))
        assertEquals(FloatingLyricsRules.TypefaceChoice(null, true), FloatingLyricsRules.typefaceChoice(1000, 26))
    }

    @Test
    fun finalContinuousLineArrivalUsesFiveHundredMillisecondsPerGraphemeAndTrackEndCap() {
        assertEquals(7_000L, FloatingLyricsRules.finalLineArrivalMs(5_000L, "最後一句", null))
        assertEquals(6_200L, FloatingLyricsRules.finalLineArrivalMs(5_000L, "最後一句", 6_200L))
        assertEquals(6_500L, FloatingLyricsRules.finalLineArrivalMs(5_000L, "Aか\u3099🙂", null))
        assertEquals(5_500L, FloatingLyricsRules.finalLineArrivalMs(5_000L, "", null))
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

    @Test
    fun theTextEdgeMayTouchTheScreenEdgeWhileTheMarginsGoPast() {
        // 7 px side margins, 20 px room above and below: the panel hangs out by exactly that.
        assertEquals(Pair(-7, -20), FloatingLyricsRules.clampPosition(-300, -300, 1_000, 150, 1_080, 2_400, 7, 20, 7, 20))
        assertEquals(Pair(87, 2_270), FloatingLyricsRules.clampPosition(900, 3_000, 1_000, 150, 1_080, 2_400, 7, 20, 7, 20))
        assertEquals(20, FloatingLyricsRules.textVerticalInset(150))
    }

    @Test
    fun aWidthChangeKeepsTheLyricAreaCentre() {
        val centre = { left: Int, width: Int -> left + 7 + (width - 84) / 2 }
        val wider = FloatingLyricsRules.resizeAroundCentre(500, 1_000, 1_400, 7, 77)
        assertEquals(300, wider)
        assertEquals(centre(500, 1_000), centre(wider, 1_400))
        val narrower = FloatingLyricsRules.resizeAroundCentre(500, 1_000, 600, 7, 77)
        assertEquals(700, narrower)
        assertEquals(centre(500, 1_000), centre(narrower, 600))
    }

    // Continuous strip fade: lines start at 1 000, 2 000 and 4 000 ms; a line fades over the 200 ms before the next starts.
    private fun weights(lyricsMs: Long) = (0..2).map { FloatingLyricsRules.cellWeight(starts, it, lyricsMs) }

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
    fun beforeTheFirstLineNothingIsFadedOut() {
        // Until the first line's lead-in every cell is fully "current" (the main opacity), whatever the position.
        assertEquals(listOf(1f, 1f, 1f), weights(0))
        assertEquals(listOf(1f, 1f, 1f), weights(500))
        assertEquals(listOf(1f, 1f, 1f), weights(800))
        assertEquals(listOf(1f, 1f, 1f), weights(-3_000))
    }

    @Test
    fun theOthersFadeOutOverTheLeadInOfTheFirstLine() {
        // From 200 ms before the first line the others ease 1 to the inactive value; the first line stays 1.
        assertEquals(listOf(1f, 0.5f, 0.5f), weights(900))
        assertEquals(listOf(1f, 0.75f, 0.75f), weights(850))
        assertEquals(listOf(1f, 0.25f, 0.25f), weights(950))
        // Its start: finished, the first line is current and the rest are inactive.
        assertEquals(listOf(1f, 0f, 0f), weights(1_000))
        assertEquals(listOf(1f, 0f, 0f), weights(1_500))
        // With a 50% inactive setting the lead-in goes 1.0 -> 0.5 for the others, never below it.
        assertEquals(0.75f, FloatingLyricsRules.fadedFactor(0.5f, FloatingLyricsRules.cellWeight(starts, 1, 900)), 0.0001f)
        assertEquals(0.5f, FloatingLyricsRules.fadedFactor(0.5f, FloatingLyricsRules.cellWeight(starts, 1, 1_000)), 0.0001f)
        assertEquals(1f, FloatingLyricsRules.fadedFactor(0.5f, FloatingLyricsRules.cellWeight(starts, 1, 800)), 0.0001f)
    }

    @Test
    fun theLastLineStaysCurrent() {
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
        // Back before the first line: everything is un-faded again.
        assertEquals(listOf(1f, 1f, 1f), weights(-3_000))
        assertEquals(listOf(1f, 1f, 1f), weights(0))
        // Into the first line's lead-in.
        assertEquals(listOf(1f, 0.5f, 0.5f), weights(900))
    }
    @Test
    fun fadeEdgeCases() {
        assertEquals(0f, FloatingLyricsRules.cellWeight(starts, -1, 2_000), 0f)
        assertEquals(0f, FloatingLyricsRules.cellWeight(starts, 3, 2_000), 0f)
        assertEquals(0f, FloatingLyricsRules.cellWeight(longArrayOf(), 0, 0), 0f)
        // A single line is always current.
        assertEquals(1f, FloatingLyricsRules.cellWeight(longArrayOf(1_000), 0, 0), 0f)
        assertEquals(1f, FloatingLyricsRules.cellWeight(longArrayOf(1_000), 0, 5_000), 0f)
        // Lines sharing a start time: only the later one is ever current.
        val same = longArrayOf(1_000, 1_000, 3_000)
        assertEquals(0f, FloatingLyricsRules.cellWeight(same, 0, 1_500), 0f)
        // ...but before them all of them are un-faded, like every other cell.
        assertEquals(1f, FloatingLyricsRules.cellWeight(same, 0, 500), 0f)
        assertEquals(1f, FloatingLyricsRules.cellWeight(same, 1, 500), 0f)
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

    @Test
    fun aMoveIsRedrawnOnlyWhenItReachesAnotherPixelAndLandsExactly() {
        assertFalse(FloatingLyricsRules.needsPixelRedraw(-10.2f, -10.7f, -10.4f))
        assertTrue(FloatingLyricsRules.needsPixelRedraw(-10.2f, -11.1f, -10.7f))
        assertTrue("stopped: the exact target", FloatingLyricsRules.needsPixelRedraw(-10.2f, -10.6f, -10.6f))
        assertFalse(FloatingLyricsRules.needsPixelRedraw(-10.6f, -10.6f, -10.6f))
        // 30 px over 120 frames: one redraw every 4 frames, evenly spaced, never more than a pixel behind.
        var shown = 0f
        var previous = 0f
        val redraws = mutableListOf<Int>()
        for (frame in 1..120) {
            val raw = 30f * frame / 120f
            if (FloatingLyricsRules.needsPixelRedraw(shown, raw, previous)) { shown = raw; redraws.add(frame) }
            assertTrue(raw - shown < 1f)
            previous = raw
        }
        assertEquals(30, redraws.size)
        assertEquals(30f, shown)
    }
}
